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
const MAX_GAMMA_INV_ITERS: usize = 64;
const MIN_LN_X: f64 = -740.0;
const MAX_LN_X: f64 = 709.0;

fn validate_gamma_args(a: f64, x: f64) -> Result<(), MathError> {
    if !a.is_finite() || a <= 0.0 {
        return Err(MathError::InvalidParameter { name: "a", value: a });
    }
    if x.is_nan() || x < 0.0 {
        return Err(MathError::DomainError { name: "x", value: x });
    }
    Ok(())
}

fn zeta_series_coeffs() -> &'static [f64; 11] {
    static COEFFS: std::sync::OnceLock<[f64; 11]> = std::sync::OnceLock::new();
    COEFFS.get_or_init(|| {
        let mut table = [0.0_f64; 11];
        for (idx, slot) in table.iter_mut().enumerate() {
            let s = (idx + 2) as i32;
            let sf = f64::from(s);
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
            let zeta_s = 1.0 + sum + em0 + em1 + em2 + em3;
            let sign = if s % 2 == 0 { 1.0 } else { -1.0 };
            *slot = sign * zeta_s / sf;
        }
        table
    })
}

/// Cancellation-free `ln Gamma(1 + a)` for `0 < a <= 0.05`.
fn ln_gamma_1p_small(a: f64) -> f64 {
    let coeffs = zeta_series_coeffs();
    let mut poly = coeffs[10];
    for &c in coeffs[..10].iter().rev() {
        poly = poly.mul_add(a, c);
    }
    (-std::f64::consts::EULER_GAMMA).mul_add(a, a * a * poly)
}

/// Series deviance `bd0(a, x) = a * ln(a / x) + x - a` keeping full accuracy for `|a - x| < 0.45 (a + x)`.
fn gamma_deviance(a: f64, x: f64) -> f64 {
    let m = a.max(x);
    if m == 0.0 || !m.is_finite() {
        return deviance_bd0(a, x);
    }
    let diff_s = (a - x) / m;
    let a_s = a / m;
    let sum_s = a_s + x / m;
    if diff_s.abs() < 0.45 * sum_s {
        let v = diff_s / sum_s;
        let v2 = v * v;
        let mut s = diff_s * v;
        let mut ej = 2.0 * a_s * v * v2;
        for j in 1..64_u32 {
            let term = ej / f64::from(2 * j + 1);
            let next = s + term;
            if term.abs() <= 0.25 * f64::EPSILON * next.abs() {
                return (m * next).max(0.0);
            }
            s = next;
            ej *= v2;
        }
        return (m * s).max(0.0);
    }
    deviance_bd0(a, x)
}

/// Cancellation-free evaluation of `ln(x^a * exp(-x) / Gamma(a))` for `a > 0, x > 0`.
fn ln_gamma_kernel(a: f64, x: f64) -> Result<f64, MathError> {
    if a >= 12.0 { Ok(-gamma_deviance(a, x) + 0.5 * (a / std::f64::consts::TAU).ln() - stirling_err(a)) } else { Ok(a * x.ln() - x - ln_gamma(a)?) }
}

/// Cancellation-free `ln P(a, x)` for small `a <= 0.05` and `x < a + 1`:
/// `P(a, x) = (x^a / Gamma(1 + a)) * (1 + a * sum_{k>=1} (-x)^k / (k! (a + k)))`.
fn ln_p_small_a_series(a: f64, x: f64) -> Result<f64, MathError> {
    let mut sum = 0.0_f64;
    let mut pow_neg_x_over_fact = 1.0_f64;
    for k in 1..=MAX_GAMMA_ITERS {
        let kf = k as f64;
        pow_neg_x_over_fact *= -x / kf;
        let term = pow_neg_x_over_fact / (a + kf);
        let next = sum + term;
        if term.abs() <= 0.25 * f64::EPSILON * sum.abs().max(f64::MIN_POSITIVE) && k > 4 {
            let delta = (a * next).max(-0.999_999_999_999_999_9);
            return Ok((a * x.ln() - ln_gamma_1p_small(a) + delta.ln_1p()).min(0.0));
        }
        sum = next;
    }
    Err(MathError::NotConverged { algorithm: "inc_gamma_small_a_series", iterations: MAX_GAMMA_ITERS })
}

fn ln_p_series(a: f64, x: f64) -> Result<f64, MathError> {
    if a <= 0.05 { ln_p_small_a_series(a, x) } else { Ok((ln_gamma_kernel(a, x)? + ln_series_sum(a, x)?).min(0.0)) }
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

/// Shared Sterbenz-exact `(z^2, z, corr)` evaluation for Temme's (1979) uniform asymptotic
/// expansion inside `|x - a| <= 0.25 * a`, where `Q(a, x) ~ e^{-z^2} (0.5 * erfcx(z) + corr)` for
/// `z >= 0` and `P(a, x) ~ e^{-z^2} (0.5 * erfcx(-z) - corr)` for `z < 0`.
fn temme_corr_and_z(a: f64, x: f64) -> (f64, f64, f64) {
    let dx = x - a;
    let mu = dx / a;
    let lambda = x / a;
    let z_sq = gamma_deviance(a, x);
    let z = z_sq.sqrt().copysign(mu);
    let eta = z * (2.0 / a).sqrt();

    let (c0, c1, c2) = if eta.abs() < 0.02 {
        let e = eta;
        // Exact rational Taylor expansions of c0(eta), c1(eta), c2(eta) around eta = 0 (Temme 1979, §3; DLMF §8.11(iii)).
        let c0 =
            -1.0 / 3.0 + e * (1.0 / 12.0 + e * (-2.0 / 135.0 + e * (1.0 / 864.0 + e * (1.0 / 2835.0 + e * (-139.0 / 777_600.0 + e / 25_515.0)))));
        let c1 = -1.0 / 540.0 + e * (-1.0 / 288.0 + e * (1.0 / 378.0 + e * (-77.0 / 77_760.0)));
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
    let corr = series / (std::f64::consts::TAU.sqrt() * a.sqrt());
    (z_sq, z, corr)
}

/// Temme's (1979) uniform asymptotic expansion `(P(a, x), Q(a, x))` for large `a` across `|x - a| <= 0.25 * a`.
pub(crate) fn temme_uniform_pq(a: f64, x: f64) -> (f64, f64) {
    let (z_sq, z, corr) = temme_corr_and_z(a, x);
    if z >= 0.0 {
        if z <= 2.0 {
            let r_a = exp_neg_sq(z) * corr;
            let q = (0.5 * erfc(z) + r_a).clamp(0.0, 1.0);
            let p = (0.5 * erfc(-z) - r_a).clamp(0.0, 1.0);
            (p, q)
        } else {
            let scaled_q = (0.5 * erfcx(z) + corr).max(0.0);
            if scaled_q == 0.0 {
                return (1.0, 0.0);
            }
            let ln_q = (-z_sq + scaled_q.ln()).min(0.0);
            ((-ln_q.exp_m1()).clamp(0.0, 1.0), ln_q.exp().clamp(0.0, 1.0))
        }
    } else {
        let w = -z;
        if w <= 2.0 {
            let r_a = exp_neg_sq(w) * corr;
            let p = (0.5 * erfc(w) - r_a).clamp(0.0, 1.0);
            let q = (0.5 * erfc(-w) + r_a).clamp(0.0, 1.0);
            (p, q)
        } else {
            let scaled_p = (0.5 * erfcx(w) - corr).max(0.0);
            if scaled_p == 0.0 {
                return (0.0, 1.0);
            }
            let ln_p = (-z_sq + scaled_p.ln()).min(0.0);
            (ln_p.exp().clamp(0.0, 1.0), (-ln_p.exp_m1()).clamp(0.0, 1.0))
        }
    }
}

fn temme_ln_p(a: f64, x: f64) -> f64 {
    let (z_sq, z, corr) = temme_corr_and_z(a, x);
    if z < 0.0 {
        let scaled_p = (0.5 * erfcx(-z) - corr).max(f64::MIN_POSITIVE);
        (-z_sq + scaled_p.ln()).min(0.0)
    } else {
        let scaled_q = (0.5 * erfcx(z) + corr).max(f64::MIN_POSITIVE);
        let ln_q = (-z_sq + scaled_q.ln()).min(0.0);
        ln_one_minus_exp(ln_q)
    }
}

fn temme_ln_q(a: f64, x: f64) -> f64 {
    let (z_sq, z, corr) = temme_corr_and_z(a, x);
    if z >= 0.0 {
        let scaled_q = (0.5 * erfcx(z) + corr).max(f64::MIN_POSITIVE);
        (-z_sq + scaled_q.ln()).min(0.0)
    } else {
        let scaled_p = (0.5 * erfcx(-z) - corr).max(f64::MIN_POSITIVE);
        let ln_p = (-z_sq + scaled_p.ln()).min(0.0);
        ln_one_minus_exp(ln_p)
    }
}

/// Use Temme's uniform expansion across the entire proportional band `|x - a| <= 0.25 * a` for
/// `a >= 4096` (`>= ±16 * sqrt(a)` at `a = 4096`, `>= ±559 * sqrt(a)` at `a = 5e6`), so outside
/// the band the power series (`x / a < 0.75`) and Legendre CF (`x - a > 0.25 * a`) always converge
/// in `< 160` iterations regardless of how large `a` is.
fn use_temme(a: f64, x: f64) -> bool {
    a >= TEMME_A_THRESHOLD && (x - a).abs() <= 0.25 * a
}

/// Cancellation-free evaluation of `ln(1 - exp(ln_val))` for `ln_val <= 0`.
fn ln_one_minus_exp(ln_val: f64) -> f64 {
    if ln_val >= 0.0 {
        f64::NEG_INFINITY
    } else if ln_val > -std::f64::consts::LN_2 {
        (-ln_val.exp_m1()).ln()
    } else {
        (-ln_val.exp()).ln_1p()
    }
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
        let ln_p = ln_p_series(a, x)?;
        Ok(ln_p.exp().clamp(0.0, 1.0))
    } else {
        let ln_q = ln_gamma_kernel(a, x)? + ln_legendre_cf_reciprocal(a, x)?;
        Ok((-ln_q.min(0.0).exp_m1()).clamp(0.0, 1.0))
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
        let ln_p = ln_p_series(a, x)?;
        Ok((-ln_p.min(0.0).exp_m1()).clamp(0.0, 1.0))
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
        return Ok(temme_ln_p(a, x));
    }
    if x < a + 1.0 {
        ln_p_series(a, x)
    } else {
        let ln_q = ln_gamma_kernel(a, x)? + ln_legendre_cf_reciprocal(a, x)?;
        Ok(ln_one_minus_exp(ln_q))
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
        return Ok(temme_ln_q(a, x));
    }
    if x >= a + 1.0 {
        Ok((ln_gamma_kernel(a, x)? + ln_legendre_cf_reciprocal(a, x)?).min(0.0))
    } else {
        let ln_p = ln_p_series(a, x)?;
        Ok(ln_one_minus_exp(ln_p))
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

#[derive(Clone, Copy)]
enum GammaTarget {
    LowerProb(f64),
    UpperProb(f64),
    DeepLowerLn(f64),
    DeepUpperLn(f64),
}

impl GammaTarget {
    fn is_increasing(self) -> bool {
        matches!(self, Self::LowerProb(_) | Self::DeepLowerLn(_))
    }

    fn is_root_below_one(self, f_at_one: f64) -> bool {
        if self.is_increasing() { f_at_one > 0.0 } else { f_at_one < 0.0 }
    }

    fn is_below_root(self, fx: f64) -> bool {
        if self.is_increasing() { fx < 0.0 } else { fx > 0.0 }
    }
}

fn eval_gamma_objective(a: f64, x: f64, target: GammaTarget) -> Result<(f64, f64, f64), MathError> {
    let ln_k = ln_gamma_kernel(a, x)?;
    match target {
        GammaTarget::LowerProb(p) => Ok((reg_gamma_p(a, x)? - p, ln_k.exp(), a - x)),
        GammaTarget::UpperProb(q) => Ok((reg_gamma_q(a, x)? - q, -ln_k.exp(), a - x)),
        GammaTarget::DeepLowerLn(target_ln) => {
            let ln_p = ln_reg_gamma_p(a, x)?;
            let df_du = if ln_p.is_finite() { (ln_k - ln_p).exp() } else { 0.0 };
            Ok((ln_p - target_ln, df_du, a - x - df_du))
        }
        GammaTarget::DeepUpperLn(target_ln) => {
            let ln_q = ln_reg_gamma_q(a, x)?;
            let df_du = if ln_q.is_finite() { -(ln_k - ln_q).exp() } else { 0.0 };
            Ok((ln_q - target_ln, df_du, a - x - df_du))
        }
    }
}

fn initial_gamma_quantile_guess_ln(a: f64, p: f64, q: f64) -> Result<f64, MathError> {
    let ln_p = if p <= 0.5 { p.ln() } else { (-q).ln_1p() };
    let ln_q = if q <= 0.5 { q.ln() } else { (-p).ln_1p() };
    let lg_a1 = ln_gamma(a + 1.0)?;
    let u_small = (ln_p + lg_a1) / a;

    // Small-x asymptotic: P(a, x) ~ x^a / Gamma(a + 1).
    if u_small < (0.35 * (a + 1.0)).ln() {
        if u_small > -40.0 {
            let x_small = u_small.exp();
            return Ok(u_small + (x_small / (a + 1.0)).ln_1p());
        }
        return Ok(u_small);
    }
    // Large-x asymptotic when a <= 1.2 and q is very small: Q(a, x) ~ x^{a-1} e^{-x} / Gamma(a).
    if a <= 1.2 && q < 1e-3 {
        let lg_a = lg_a1 - a.ln();
        let u = -ln_q - lg_a;
        let x0 = (u + (a - 1.0) * u.max(1.0).ln()).max(1e-12);
        return Ok(x0.ln());
    }
    // Wilson-Hilferty cube-root normal approximation using tail-safe z.
    let z = if q <= p { norm_isf_raw(q)? } else { -norm_isf_raw(p)? };
    let w = 1.0 - 1.0 / (9.0 * a) + z / (3.0 * a.sqrt());
    if w > 0.15 {
        return Ok((a.ln() + 3.0 * w.ln()).max(MIN_LN_X));
    }
    if p <= q {
        Ok(u_small)
    } else {
        let lg_a = lg_a1 - a.ln();
        let u = (-ln_q - lg_a).max(1.0);
        Ok((a + u + (a - 1.0) * u.ln()).ln())
    }
}

fn invert_reg_gamma_pair(a: f64, p: f64, q: f64) -> Result<f64, MathError> {
    if p == 0.0 {
        return Ok(0.0);
    }
    if q == 0.0 {
        return Ok(f64::INFINITY);
    }
    let target = if p < 1e-8 {
        GammaTarget::DeepLowerLn(p.ln())
    } else if q < 1e-8 {
        GammaTarget::DeepUpperLn(q.ln())
    } else if q < p {
        GammaTarget::UpperProb(q)
    } else {
        GammaTarget::LowerProb(p)
    };

    let u_guess = initial_gamma_quantile_guess_ln(a, p, q)?;
    let (f1, _, _) = eval_gamma_objective(a, 1.0, target)?;
    if f1 == 0.0 {
        return Ok(1.0);
    }
    if target.is_root_below_one(f1) { solve_gamma_log_space(a, u_guess, target) } else { solve_gamma_lin_space(a, u_guess, target) }
}

fn polish_gamma_log_root(a: f64, x_init: f64, mut x_lo: f64, mut x_hi: f64, target: GammaTarget) -> Result<f64, MathError> {
    if !x_init.is_finite() || x_init < f64::MIN_POSITIVE || !x_lo.is_finite() || !x_hi.is_finite() || x_lo >= x_hi {
        return Ok(x_init);
    }
    let mut x = x_init.clamp(x_lo, x_hi);
    for _ in 0..4 {
        let (fx, df_du, d2_du) = eval_gamma_objective(a, x, target)?;
        if fx == 0.0 {
            return Ok(x);
        }
        if target.is_below_root(fx) {
            x_lo = x_lo.max(x);
        } else {
            x_hi = x_hi.min(x);
        }
        if !df_du.is_finite() || df_du.abs() <= f64::MIN_POSITIVE {
            break;
        }
        let rel_newton = fx / df_du;
        let denom = (1.0 - 0.5 * rel_newton * (d2_du - 1.0)).clamp(0.2, 5.0);
        let step = x * (rel_newton / denom);
        let cand = x - step;
        if !cand.is_finite() || cand <= x_lo || cand >= x_hi {
            break;
        }
        if step.abs() <= 1.5 * f64::EPSILON * cand {
            return Ok(cand);
        }
        x = cand;
    }
    Ok(x)
}

fn solve_gamma_log_space(a: f64, u_guess: f64, target: GammaTarget) -> Result<f64, MathError> {
    let mut u_lo = MIN_LN_X;
    let mut u_hi = 0.0_f64;
    let x_min = u_lo.exp();
    let (f_lo, _, _) = eval_gamma_objective(a, x_min, target)?;
    if f_lo == 0.0 {
        return Ok(x_min);
    }
    if !target.is_below_root(f_lo) {
        return Err(MathError::NotConverged { algorithm: "solve_gamma_log_space", iterations: 0 });
    }

    let mut u = if u_guess.is_finite() && u_guess > u_lo && u_guess < u_hi { u_guess } else { 0.5 * (u_lo + u_hi) };

    for _ in 0..MAX_GAMMA_INV_ITERS {
        let x = u.exp();
        if !x.is_finite() || x <= 0.0 {
            return Err(MathError::NotConverged { algorithm: "solve_gamma_log_space", iterations: MAX_GAMMA_INV_ITERS });
        }
        let (fx, df_du, d2_du) = eval_gamma_objective(a, x, target)?;
        if fx == 0.0 {
            return Ok(x);
        }
        if target.is_below_root(fx) {
            u_lo = u_lo.max(u);
        } else {
            u_hi = u_hi.min(u);
        }
        if (u_hi - u_lo) <= 2.0 * f64::EPSILON * u.abs().max(1.0) {
            let u_mid = 0.5 * (u_lo + u_hi);
            return polish_gamma_log_root(a, u_mid.exp(), u_lo.exp(), u_hi.exp(), target);
        }
        let mut next_u = safeguarded_halley_step(u, fx, df_du, d2_du, u_lo, u_hi);
        let used_halley = next_u > u_lo && next_u < u_hi;
        if !used_halley {
            next_u = 0.5 * (u_lo + u_hi);
            if next_u <= u_lo || next_u >= u_hi {
                return polish_gamma_log_root(a, next_u.exp(), u_lo.exp(), u_hi.exp(), target);
            }
        } else if (next_u - u).abs() <= 1.5 * f64::EPSILON * next_u.abs().max(1.0) {
            return polish_gamma_log_root(a, next_u.exp(), u_lo.exp(), u_hi.exp(), target);
        }
        u = next_u;
    }

    Err(MathError::NotConverged { algorithm: "solve_gamma_log_space", iterations: MAX_GAMMA_INV_ITERS })
}

fn solve_gamma_lin_space(a: f64, u_guess: f64, target: GammaTarget) -> Result<f64, MathError> {
    let x_guess = if u_guess.is_finite() && u_guess > 0.0 && u_guess < MAX_LN_X { u_guess.exp() } else { 2.0 };
    let mut lo = 1.0_f64;
    let mut hi = (x_guess * 2.0).clamp(2.0, f64::MAX);
    let mut bracketed = false;

    for _ in 0..MAX_GAMMA_INV_ITERS {
        let (f_hi, _, _) = eval_gamma_objective(a, hi, target)?;
        if f_hi == 0.0 {
            return Ok(hi);
        }
        if !target.is_below_root(f_hi) {
            bracketed = true;
            break;
        }
        lo = hi;
        if hi >= f64::MAX {
            break;
        }
        hi = if hi <= f64::MAX / 4.0 { hi * 4.0 } else { f64::MAX };
    }
    if !bracketed {
        return Err(MathError::NotConverged { algorithm: "solve_gamma_lin_space", iterations: MAX_GAMMA_INV_ITERS });
    }

    let mut x = if x_guess > lo && x_guess < hi { x_guess } else { 0.5 * (lo + hi) };
    for _ in 0..MAX_GAMMA_INV_ITERS {
        let (fx, df_du, d2_du) = eval_gamma_objective(a, x, target)?;
        if fx == 0.0 {
            return Ok(x);
        }
        if target.is_below_root(fx) {
            lo = lo.max(x);
        } else {
            hi = hi.min(x);
        }
        if (hi - lo) <= 2.0 * f64::EPSILON * x.max(1.0) {
            return Ok(0.5 * (lo + hi));
        }
        let df_dx = df_du / x;
        let d2_dx = (d2_du - 1.0) / x;
        let mut next = safeguarded_halley_step(x, fx, df_dx, d2_dx, lo, hi);
        let used_halley = next > lo && next < hi;
        if !used_halley {
            next = 0.5 * (lo + hi);
            if next <= lo || next >= hi {
                return Ok(next);
            }
        } else if (next - x).abs() <= 1.5 * f64::EPSILON * next.max(1.0) {
            return Ok(next);
        }
        x = next;
    }

    Err(MathError::NotConverged { algorithm: "solve_gamma_lin_space", iterations: MAX_GAMMA_INV_ITERS })
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

        // GAP-03-04: verify Temme c1(eta) Taylor expansion (`1/378` eta^2 term, Temme 1979 §3 / DLMF 8.11)
        // is continuous across the `|eta| = 0.02` branch boundary. The two points straddle eta = 0.02 at
        // a relative offset of 1e-12 (`mu_seam` solves `mu - ln(1 + mu) = 2e-4`, i.e. `eta(mu) = 0.02`), so
        // the smooth change of `corr` between them (`|d corr / d eta| ~ 1 / (12 sqrt(2 pi a))`) is ~1e-17.
        let a_seam = 4096.0_f64;
        let mu_seam = 0.020_133_554_963_707_46_f64;
        let (_, z_below, corr_below) = super::temme_corr_and_z(a_seam, a_seam * (1.0 + mu_seam * (1.0 - 1e-12)));
        let (_, z_above, corr_above) = super::temme_corr_and_z(a_seam, a_seam * (1.0 + mu_seam * (1.0 + 1e-12)));
        let eta_below = z_below * (2.0 / a_seam).sqrt();
        let eta_above = z_above * (2.0 / a_seam).sqrt();
        assert!(eta_below < 0.02 && eta_above >= 0.02, "points must straddle the seam: {eta_below} / {eta_above}");
        // `corr = (c0 + c1 / a + c2 / a^2) / sqrt(2 pi a)`, so a wrong eta^2 coefficient in c1 (the old
        // `25/12096` instead of `1/378`) opens a seam jump of `|1/378 - 25/12096| eta^2 / (a sqrt(2 pi a))`
        // = 3.5e-13 (measured 3.3e-13). With the correct coefficients the residual jump comes from holding
        // c2 at its constant term `25/6048` in the Taylor branch: `|c2'(0)| eta / (a^2 sqrt(2 pi a))` with
        // `c2'(0) = -139/51840` (Temme 1979) = 2.0e-14 (measured 2.0e-14). The 1e-13 tolerance sits 5x
        // above the expected residual and 3.5x below the wrong-coefficient jump.
        let seam_scale = 1.0 / (a_seam * (std::f64::consts::TAU * a_seam).sqrt());
        let wrong_coeff_jump = (1.0 / 378.0 - 25.0 / 12_096.0_f64).abs() * 0.02 * 0.02 * seam_scale;
        let c2_truncation_jump = (139.0 / 51_840.0) * 0.02 / a_seam * seam_scale;
        let seam_tol = 1e-13;
        assert!(seam_tol < wrong_coeff_jump / 3.0 && seam_tol > 4.0 * c2_truncation_jump);
        assert!((corr_above - corr_below).abs() < seam_tol, "Temme seam jump: {corr_below} vs {corr_above}");
        // Direct check of the exact c1(eta) identity against the Taylor polynomial at mu = 0.05
        // using Sterbenz-exact `gamma_deviance` (no truncation error in `lambda(eta)`). The buggy
        // `25/12096` coefficient would deviate by ~1.4e-6 here, while exact `1/378` matches to ~1e-9.
        let mu_hi = 0.05_f64;
        let lam_hi = 1.0 + mu_hi;
        let eta_hi = (2.0 * super::gamma_deviance(1.0, lam_hi)).sqrt();
        let c1_exact = eta_hi.powi(-3) - lam_hi * mu_hi.powi(-3) - 1.0 / (12.0 * mu_hi);
        let c1_taylor = -1.0 / 540.0 + eta_hi * (-1.0 / 288.0 + eta_hi * (1.0 / 378.0 + eta_hi * (-77.0 / 77_760.0)));
        assert!((c1_taylor - c1_exact).abs() < 1e-8, "c1_taylor={c1_taylor} vs c1_exact={c1_exact}");
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
