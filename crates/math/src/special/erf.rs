//! Error function (`erf`), tail-safe complementary error function (`erfc`), scaled complementary
//! error function (`erfcx`), and inverse normal/error quantiles (`norm_ppf`, `norm_isf`, `erfc_inv`).
//!
//! References:
//! - NIST Digital Library of Mathematical Functions, Ch. 7 (Error Functions, Dawson's and Fresnel
//!   Integrals): 7.6.2 (Kummer positive series), 7.9.3 (Stieltjes/Laplace even continued fraction).
//! - Blair, J. M., Edwards, C. A., & Johnson, J. H. (1976). Rational Chebyshev approximations for
//!   the inverse of the error function. *Mathematics of Computation*, 30(136), 827-830.

use crate::error::MathError;

const TWO_OVER_SQRT_PI: f64 = std::f64::consts::FRAC_2_SQRT_PI;
const INV_SQRT_PI: f64 = 0.564_189_583_547_756_3;
const SQRT_2_OVER_PI: f64 = 0.797_884_560_802_865_4;
const HALF_LN_2PI: f64 = 0.918_938_533_204_672_7;

/// Compute `exp(-x^2)` with extended exponent precision by splitting `x` into exact 25-bit upper
/// and residual parts so `x^2` never loses 10 bits of precision in the deep Gaussian tail.
#[inline]
pub(crate) fn exp_neg_sq(x: f64) -> f64 {
    let ax = x.abs();
    if ax > 27.5 {
        return 0.0;
    }
    let hi = f64::from_bits(ax.to_bits() & 0xFFFF_FFFF_F000_0000);
    let rem = (ax - hi) * (ax + hi);
    (-hi * hi).exp() * (-rem).exp()
}

/// Kummer's strictly positive series `erf(x) = (2/sqrt(pi)) * x * exp(-x^2) * sum (2x^2)^n / (2n+1)!!`
/// for `0 <= x <= 2.2`.
fn erf_kummer_positive(x: f64) -> f64 {
    if x < 1e-8 {
        return TWO_OVER_SQRT_PI * x * (1.0 - (x * x) / 3.0);
    }
    let two_x2 = 2.0 * x * x;
    let mut sum = 1.0_f64;
    let mut term = 1.0_f64;
    for n in 1..64_u32 {
        term *= two_x2 / f64::from(2 * n + 1);
        let next = sum + term;
        if term <= 0.25 * f64::EPSILON * next {
            sum = next;
            break;
        }
        sum = next;
    }
    TWO_OVER_SQRT_PI * x * exp_neg_sq(x) * sum
}

/// Even contraction of Laplace's Stieltjes continued fraction for `erfcx(x) = exp(x^2) erfc(x)`
/// on `x >= 0.85` using modified Lentz's method:
/// `sqrt(pi) * erfcx(x) = x / (x^2 + 0.5 - (1 * 0.5) / (x^2 + 2.5 - (2 * 1.5) / (x^2 + 4.5 - ...)))`.
fn erfcx_continued_fraction(x: f64) -> f64 {
    if x > 1e8 {
        let inv_x2 = 1.0 / (x * x);
        return (INV_SQRT_PI / x) * (1.0 - 0.5 * inv_x2 * (1.0 - 1.5 * inv_x2));
    }
    let x2 = x * x;
    let tiny = 1e-300_f64;
    let mut f = (x2 + 0.5).max(tiny);
    let mut c = f;
    let mut d = 0.0_f64;
    for k in 1..256_u32 {
        let kf = f64::from(k);
        let a_k = -kf * (kf - 0.5);
        let b_k = x2 + 2.0 * kf + 0.5;

        d = b_k + a_k * d;
        if d.abs() < tiny {
            d = tiny;
        }
        c = b_k + a_k / c;
        if c.abs() < tiny {
            c = tiny;
        }
        d = 1.0 / d;
        let delta = c * d;
        f *= delta;
        if (delta - 1.0).abs() <= 0.5 * f64::EPSILON {
            break;
        }
    }
    (INV_SQRT_PI * x) / f
}

/// Gauss error function `erf(x) = (2 / sqrt(pi)) * int_0^x exp(-t^2) dt`.
#[must_use]
pub fn erf(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x == 0.0 {
        return x;
    }
    let ax = x.abs();
    let mag = if ax <= 2.0 {
        erf_kummer_positive(ax)
    } else if ax >= 6.2 {
        1.0
    } else {
        1.0 - exp_neg_sq(ax) * erfcx_continued_fraction(ax)
    };
    mag.copysign(x)
}

/// Complementary error function `erfc(x) = 1 - erf(x)` evaluated directly in the upper tail
/// without `1.0 - erf(x)` cancellation.
#[must_use]
pub fn erfc(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x.is_infinite() {
        return if x.is_sign_positive() { 0.0 } else { 2.0 };
    }
    let ax = x.abs();
    if ax <= 0.85 {
        return if x >= 0.0 { 1.0 - erf_kummer_positive(ax) } else { 1.0 + erf_kummer_positive(ax) };
    }
    let upper = exp_neg_sq(ax) * erfcx_continued_fraction(ax);
    if x >= 0.0 { upper } else { 2.0 - upper }
}

/// Scaled complementary error function `erfcx(x) = exp(x^2) * erfc(x)`, remaining finite and
/// accurate for arbitrarily large positive `x` where `erfc(x)` underflows `f64`.
#[must_use]
pub fn erfcx(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x.is_infinite() {
        return if x.is_sign_positive() { 0.0 } else { f64::INFINITY };
    }
    if x >= 0.85 {
        return erfcx_continued_fraction(x);
    }
    if x >= 0.0 {
        return (x * x).exp() * (1.0 - erf_kummer_positive(x));
    }
    let ax = -x;
    if ax > 26.6 {
        return f64::INFINITY;
    }
    let exp_pos = (ax * ax).exp();
    let tail = if ax >= 0.85 { erfcx_continued_fraction(ax) } else { exp_pos * (1.0 - erf_kummer_positive(ax)) };
    2.0 * exp_pos - tail
}

/// Upper-tail standard normal quantile `z = Phi^{-1}(1 - q)` computed directly from upper-tail
/// probability `q in [0, 1]` without `1.0 - q` subtraction cancellation.
///
/// # Errors
/// Returns [`MathError::DomainError`] if `q` is outside `[0.0, 1.0]` or `NaN`.
pub fn norm_isf_raw(q: f64) -> Result<f64, MathError> {
    if !q.is_finite() || !(0.0..=1.0).contains(&q) {
        return Err(MathError::DomainError { name: "q", value: q });
    }
    if q == 0.0 {
        return Ok(f64::INFINITY);
    }
    if q == 1.0 {
        return Ok(f64::NEG_INFINITY);
    }
    if q == 0.5 {
        return Ok(0.0);
    }
    let (tail, sign) = if q < 0.5 { (q, 1.0_f64) } else { (1.0 - q, -1.0_f64) };
    let z_pos = invert_normal_upper_tail(tail);
    Ok(sign * z_pos)
}

/// Lower-tail standard normal quantile `z = Phi^{-1}(p)` (`probit`).
///
/// # Errors
/// Returns [`MathError::DomainError`] if `p` is outside `[0.0, 1.0]` or `NaN`.
pub fn norm_ppf_raw(p: f64) -> Result<f64, MathError> {
    if !p.is_finite() || !(0.0..=1.0).contains(&p) {
        return Err(MathError::DomainError { name: "p", value: p });
    }
    if p == 0.0 {
        return Ok(f64::NEG_INFINITY);
    }
    if p == 1.0 {
        return Ok(f64::INFINITY);
    }
    if p == 0.5 {
        return Ok(0.0);
    }
    let (tail, sign) = if p < 0.5 { (p, -1.0_f64) } else { (1.0 - p, 1.0_f64) };
    Ok(sign * invert_normal_upper_tail(tail))
}

/// Inverse complementary error function `erfc_inv(q)` solving `erfc(x) = q` for `q in [0, 2]`.
///
/// # Errors
/// Returns [`MathError::DomainError`] if `q` is outside `[0.0, 2.0]` or `NaN`.
pub fn erfc_inv(q: f64) -> Result<f64, MathError> {
    if !q.is_finite() || !(0.0..=2.0).contains(&q) {
        return Err(MathError::DomainError { name: "q", value: q });
    }
    if q == 0.0 {
        return Ok(f64::INFINITY);
    }
    if q == 2.0 {
        return Ok(f64::NEG_INFINITY);
    }
    if q == 1.0 {
        return Ok(0.0);
    }
    let (half_tail, sign) = if q < 1.0 { (0.5 * q, 1.0) } else { (0.5 * (2.0 - q), -1.0) };
    Ok(sign * invert_normal_upper_tail(half_tail) * std::f64::consts::FRAC_1_SQRT_2)
}

/// Solve `0.5 * erfc(z / sqrt(2)) = r` for `r in (0, 0.5)` to full 53-bit `f64` precision.
fn invert_normal_upper_tail(r: f64) -> f64 {
    let mut z = if r < 0.05 {
        let t = -2.0 * r.ln();
        (t - (std::f64::consts::TAU * t).ln()).max(0.1).sqrt()
    } else {
        let d = 0.5 - r;
        let d2 = d * d;
        (std::f64::consts::TAU).sqrt() * d * (1.0 + d2 * (std::f64::consts::PI / 3.0 + d2 * 1.4))
    };

    if r >= 1e-12 {
        // Direct probability Halley steps: f(z) = 0.5 * erfc(z / sqrt(2)) - r.
        for _ in 0..4 {
            let sf = 0.5 * erfc(z * std::f64::consts::FRAC_1_SQRT_2);
            let pdf = (-HALF_LN_2PI).exp() * exp_neg_sq(z * std::f64::consts::FRAC_1_SQRT_2);
            let delta = (sf - r) / pdf;
            let step = delta / (1.0 - 0.5 * z * delta).clamp(0.25, 4.0);
            z = (z + step).max(0.0);
            if step.abs() <= 0.5 * f64::EPSILON * z {
                break;
            }
        }
    } else {
        // Log-probability Halley steps via erfcx (never underflows down to r = 2.2e-308).
        let target_ln = r.ln();
        let ln_half = -std::f64::consts::LN_2;
        for _ in 0..5 {
            let w = z * std::f64::consts::FRAC_1_SQRT_2;
            let ex = erfcx(w);
            let ln_sf = ln_half - 0.5 * z * z + ex.ln();
            let g = ln_sf - target_ln;
            let g1 = -SQRT_2_OVER_PI / ex;
            let newton = g / g1;
            let d2_over_d1 = z + g1;
            let step = newton / (1.0 - 0.5 * newton * d2_over_d1).clamp(0.25, 4.0);
            z = (z - step).max(0.0);
            if step.abs() <= 0.5 * f64::EPSILON * z {
                break;
            }
        }
    }
    z
}

#[cfg(test)]
mod tests {
    use super::{erf, erfc, erfc_inv, erfcx, norm_isf_raw, norm_ppf_raw};

    #[test]
    fn erf_and_erfc_match_nist_reference_values() {
        // Abramowitz & Stegun / NIST DLMF Table values:
        assert_eq!(erf(0.0), 0.0);
        assert!((erf(0.5) - 0.520_499_877_813_046_5).abs() < 1e-15);
        assert!((erf(1.0) - 0.842_700_792_949_714_9).abs() < 1e-15);
        assert!((erfc(1.0) - 0.157_299_207_050_285_1).abs() < 1e-15);
        assert!((erfc(2.0) - 0.004_677_734_981_047_266).abs() < 1e-17);
        assert!((erfc(5.0) - 1.537_459_794_428_035e-12).abs() / 1.537e-12 < 1e-14);
        assert!((erfc(10.0) - 2.088_487_583_762_545e-45).abs() / 2.088e-45 < 1e-14);
        assert!((erfcx(100.0) - 0.005_641_613_782_989_433).abs() < 1e-16);
    }

    #[test]
    fn norm_isf_and_ppf_roundtrip_into_extreme_tails() {
        for &q in &[0.5, 0.25, 0.05, 0.025, 1e-5, 1e-16, 1e-50, 1e-200, 1e-300] {
            let z = norm_isf_raw(q).unwrap();
            let w = z * std::f64::consts::FRAC_1_SQRT_2;
            let ln_q_back = -std::f64::consts::LN_2 - 0.5 * z * z + erfcx(w).ln();
            let tol = 1e-13_f64.max(4.0 * f64::EPSILON * q.ln().abs());
            assert!((ln_q_back - q.ln()).abs() <= tol, "q={q}: z={z}, ln_q_back={ln_q_back} vs {}", q.ln());
        }
        assert!((norm_ppf_raw(0.975).unwrap() - 1.959_963_984_540_054).abs() < 1e-14);
        assert!((erfc(erfc_inv(1e-30).unwrap()) - 1e-30).abs() / 1e-30 < 1e-13);
    }
}
