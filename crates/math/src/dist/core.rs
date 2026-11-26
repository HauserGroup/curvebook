//! Tail-safe continuous probability distributions: Standard Normal, Student's `t` (real df),
//! Snedecor's `F` (real df1, df2), and Chi-Square `chi^2` (real df).
//!
//! Every distribution exposes direct lower-tail `cdf`, upper-tail `sf`, log-survival `ln_sf`, and
//! both lower-tail (`ppf`) and upper-tail (`isf`) quantile evaluators without `1.0 - cdf(x)` or
//! `1.0 - q` subtraction cancellation.
//!
//! Each distribution is evaluated through its standard relation to a special function (equation
//! numbers from Abramowitz & Stegun 1964, ch. 26):
//! - Normal: density 26.2.1 and tails 26.2.2-26.2.3, evaluated as `P(z) = erfc(-z / sqrt(2)) / 2`
//!   with the error functions of `special::erf`.
//! - Chi-square: density 26.4.1; `P(chi^2 | nu)` and `Q(chi^2 | nu)` are the regularized incomplete
//!   gamma functions `P(nu / 2, chi^2 / 2)` and `Q(nu / 2, chi^2 / 2)` (26.4.19).
//! - Snedecor `F`: density 26.6.1; `Q(F | nu1, nu2) = I_x(nu2 / 2, nu1 / 2)` with
//!   `x = nu2 / (nu2 + nu1 F)` (26.6.2). The lower tail follows from the symmetry
//!   `I_x(a, b) = 1 - I_{1-x}(b, a)` (NIST DLMF 8.17.4).
//! - Student `t`: `A(t | nu) = P(|T| <= t) = 1 - I_x(nu / 2, 1 / 2)` with `x = nu / (nu + t^2)`
//!   (26.7.1). One-sided tails are half the two-sided tail, by symmetry.
//!
//! Quantiles invert these relations with the incomplete beta and gamma inverses in `special`,
//! whose algorithms and references are documented there.
//!
//! References:
//! - Abramowitz, M., & Stegun, I. A. (Eds.). (1964). *Handbook of Mathematical Functions with
//!   Formulas, Graphs, and Mathematical Tables* (NBS Applied Mathematics Series 55), ch. 26
//!   (Probability Functions), 26.2.1-26.2.3, 26.4.1, 26.4.19, 26.6.1-26.6.2, 26.7.1. U.S.
//!   Government Printing Office.
//! - NIST Digital Library of Mathematical Functions, §8.17 (Incomplete Beta Functions).

use crate::error::MathError;
use crate::special::erf::exp_neg_sq;
use crate::special::gamma::deviance_bd0;
use crate::special::gamma::stirling_err;
use crate::special::inc_beta::ln_beta_kernel;
use crate::special::{
    erfc, erfcx, ln_beta, ln_gamma, ln_reg_beta_xy, ln_reg_gamma_p, ln_reg_gamma_q, norm_isf_raw, norm_ppf_raw, reg_beta_inv_pair, reg_beta_xy,
    reg_gamma_p, reg_gamma_p_inv, reg_gamma_q, reg_gamma_q_inv,
};

const HALF_LN_2PI: f64 = 0.918_938_533_204_672_7;

/// Degrees-of-freedom cutoff above which Student's `t` functions delegate to the standard normal
/// limit (Abramowitz & Stegun 1964, §26.7.5; Wallace 1959).
///
/// Verified leading-order error bounds for `df >= 1e12`:
/// - One-sided CDF/SF: `|F_t(t; df) - Phi(t)| = phi(t) * |t^3 + t| / (4 * df) + O(df^{-2}) <= 1.59e-13`
///   uniformly for all `t in R` (`<= 3.17e-13` for two-sided `p_2`).
/// - Quantiles (`isf`/`ppf`): `|t_{p, df} - z_p| = (|z_p|^3 + |z_p|) / (4 * df) + O(df^{-2})`
///   (relative error `<= 1.5e-11` for `p >= 1e-12`, `<= 1.2e-10` even at `p = 1e-100`).
/// - Log-density (`student_t_ln_pdf`) and log two-sided tail (`student_t_ln_two_sided_p`):
///   `<= 2.5e-9` for `|t| <= 10`, `<= 5.3e-7` across the non-underflowing support `|t| <= 38`.
const HUGE_DF_NORMAL_CUTOFF: f64 = 1e12;

fn check_pos_param(name: &'static str, v: f64) -> Result<(), MathError> {
    if v.is_nan() || v <= 0.0 {
        return Err(MathError::InvalidParameter { name, value: v });
    }
    Ok(())
}

fn check_prob(name: &'static str, p: f64) -> Result<(), MathError> {
    if !p.is_finite() || !(0.0..=1.0).contains(&p) {
        return Err(MathError::DomainError { name, value: p });
    }
    Ok(())
}

/// Split finite `v > 0.0` into `(m, e)` such that `v == m * 2^e` and `m in [0.5, 1.0)`.
fn split_pow2(mut v: f64) -> (f64, i32) {
    let mut exp_adj = 0_i32;
    if v < f64::MIN_POSITIVE {
        // Exact multiplication by 2^64 normalizes all positive subnormals without rounding.
        v *= f64::from_bits(0x43f0_0000_0000_0000);
        exp_adj = -64;
    }
    let bits = v.to_bits();
    let exp_bits = ((bits >> 52) & 0x7ff) as i32;
    let mant = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3fe0_0000_0000_0000);
    (mant, exp_bits - 1022 + exp_adj)
}

/// Reconstruct `m * 2^exp` for positive normal `m`, clamping cleanly to `0.0` / `+inf`.
fn ldexp_pos(mut m: f64, mut exp: i32) -> f64 {
    while exp > 1023 {
        m *= f64::from_bits(0x7fe0_0000_0000_0000);
        exp -= 1023;
        if !m.is_finite() {
            return f64::INFINITY;
        }
    }
    while exp < -1022 {
        m *= f64::from_bits(0x0010_0000_0000_0000);
        exp += 1022;
        if m == 0.0 {
            return 0.0;
        }
    }
    let scale = f64::from_bits(((1023 + exp) as u64) << 52);
    m * scale
}

/// Compute `(a * b) / c` for finite `a, b, c > 0.0` without intermediate overflow or underflow.
pub(crate) fn mul_div3(a: f64, b: f64, c: f64) -> f64 {
    let (ma, ea) = split_pow2(a);
    let (mb, eb) = split_pow2(b);
    let (mc, ec) = split_pow2(c);
    ldexp_pos((ma * mb) / mc, ea + eb - ec)
}

/// Compute `(a * b) / (c * d)` for finite `a, b, c, d > 0.0` without intermediate overflow/underflow.
fn mul_div4(a: f64, b: f64, c: f64, d: f64) -> f64 {
    let (ma, ea) = split_pow2(a);
    let (mb, eb) = split_pow2(b);
    let (mc, ec) = split_pow2(c);
    let (md, ed) = split_pow2(d);
    ldexp_pos((ma * mb) / (mc * md), ea + eb - ec - ed)
}

// ============================================================================
// Standard Normal Distribution N(0, 1)
// ============================================================================

/// Standard normal probability density `phi(z) = exp(-z^2 / 2) / sqrt(2 pi)`.
#[must_use]
pub fn norm_pdf(z: f64) -> f64 {
    if z.is_nan() {
        return f64::NAN;
    }
    if z.is_infinite() {
        return 0.0;
    }
    (-HALF_LN_2PI).exp() * exp_neg_sq(z * std::f64::consts::FRAC_1_SQRT_2)
}

/// Natural logarithm of the standard normal density `ln phi(z) = -0.5 z^2 - 0.5 ln(2 pi)`.
#[must_use]
pub fn norm_ln_pdf(z: f64) -> f64 {
    if z.is_nan() {
        return f64::NAN;
    }
    -HALF_LN_2PI - 0.5 * z * z
}

/// Standard normal lower-tail cumulative probability `Phi(z) = P(Z <= z)`.
#[must_use]
pub fn norm_cdf(z: f64) -> f64 {
    0.5 * erfc(-z * std::f64::consts::FRAC_1_SQRT_2)
}

/// Standard normal upper-tail survival probability `S(z) = P(Z > z) = 0.5 * erfc(z / sqrt(2))`.
#[must_use]
pub fn norm_sf(z: f64) -> f64 {
    0.5 * erfc(z * std::f64::consts::FRAC_1_SQRT_2)
}

/// Natural logarithm of the standard normal upper-tail probability `ln P(Z > z)`, remaining finite
/// for arbitrarily large positive `z` via `erfcx`.
#[must_use]
pub fn norm_ln_sf(z: f64) -> f64 {
    if z.is_nan() {
        return f64::NAN;
    }
    if z.is_infinite() {
        return if z.is_sign_positive() { f64::NEG_INFINITY } else { 0.0 };
    }
    if z >= 0.0 {
        let w = z * std::f64::consts::FRAC_1_SQRT_2;
        -std::f64::consts::LN_2 - 0.5 * z * z + erfcx(w).ln()
    } else {
        let s = norm_sf(-z);
        if s == 0.0 { 0.0 } else { (-s).ln_1p() }
    }
}

/// Two-sided standard normal probability `P(|Z| >= |z|) = erfc(|z| / sqrt(2))`.
#[must_use]
pub fn norm_two_sided_p(z: f64) -> f64 {
    erfc(z.abs() * std::f64::consts::FRAC_1_SQRT_2)
}

/// Natural logarithm of the two-sided standard normal probability `ln P(|Z| >= |z|)`.
#[must_use]
pub fn norm_ln_two_sided_p(z: f64) -> f64 {
    if z.is_nan() {
        return f64::NAN;
    }
    let az = z.abs();
    if az.is_infinite() {
        return f64::NEG_INFINITY;
    }
    let w = az * std::f64::consts::FRAC_1_SQRT_2;
    -0.5 * az * az + erfcx(w).ln()
}

/// Standard normal lower-tail quantile `Phi^{-1}(p)`.
///
/// # Errors
/// Returns [`MathError::DomainError`] if `p` is outside `[0, 1]` or `NaN`.
pub fn norm_ppf(p: f64) -> Result<f64, MathError> {
    norm_ppf_raw(p)
}

/// Standard normal upper-tail quantile `Phi^{-1}(1 - q)` directly from `q in [0, 1]`.
///
/// # Errors
/// Returns [`MathError::DomainError`] if `q` is outside `[0, 1]` or `NaN`.
pub fn norm_isf(q: f64) -> Result<f64, MathError> {
    norm_isf_raw(q)
}

/// Two-sided standard normal critical value `z_{alpha / 2}` solving `P(|Z| >= z) = alpha`.
///
/// # Errors
/// Returns [`MathError::DomainError`] if `alpha` is outside `[0, 1]` or `NaN`.
pub fn norm_two_sided_isf(alpha: f64) -> Result<f64, MathError> {
    check_prob("alpha", alpha)?;
    if alpha > 0.0 && alpha < 1e-300 {
        return Ok(crate::special::erf::invert_normal_upper_tail_ln(alpha.ln() - std::f64::consts::LN_2));
    }
    norm_isf_raw(0.5 * alpha)
}

// ============================================================================
// Student's t Distribution (real degrees of freedom df > 0 or +inf)
// ============================================================================

/// Natural logarithm of the Student `t` probability density with `df > 0`.
///
/// For `df >= 1e12` (or `df = +inf`), delegates to [`norm_ln_pdf`] (Abramowitz & Stegun 1964,
/// §26.7.5) with leading discrepancy `|t^4 - 2 t^2 - 1| / (4 * df) + O(t^6 / df^2)` (`<= 2.5e-9`
/// for `|t| <= 10`, `<= 5.3e-7` for `|t| <= 38`).
///
/// # Errors
/// Returns [`MathError::InvalidParameter`] if `df <= 0` or [`MathError::DomainError`] if `t` is `NaN`.
pub fn student_t_ln_pdf(t: f64, df: f64) -> Result<f64, MathError> {
    check_pos_param("df", df)?;
    if t.is_nan() {
        return Err(MathError::DomainError { name: "t", value: t });
    }
    if df.is_infinite() || df >= HUGE_DF_NORMAL_CUTOFF {
        return Ok(norm_ln_pdf(t));
    }
    if t.is_infinite() {
        return Ok(f64::NEG_INFINITY);
    }
    let r = t.abs() / df.sqrt();
    let ln_kernel = if r <= 1.0 { -0.5 * (df + 1.0) * (r * r).ln_1p() } else { -0.5 * (df + 1.0) * (2.0 * r.ln() + (1.0 / (r * r)).ln_1p()) };
    Ok(ln_kernel - 0.5 * df.ln() - ln_beta(0.5 * df, 0.5)?)
}

/// Student `t` probability density `f(t; df)` for real `df > 0`.
///
/// For `df >= 1e12`, delegates to the standard normal limit (see [`student_t_ln_pdf`]).
///
/// # Errors
/// Returns an error if `df <= 0` or `t` is `NaN`.
pub fn student_t_pdf(t: f64, df: f64) -> Result<f64, MathError> {
    Ok(student_t_ln_pdf(t, df)?.exp())
}

/// Exact two-sided Student `t` *p*-value `p_2(t, df) = I_{df / (df + t^2)}(df / 2, 1 / 2)`
/// supporting fractional Welch-Satterthwaite `df > 0` as well as `df = +inf`.
///
/// For `df >= 1e12`, delegates to [`norm_two_sided_p`] (Abramowitz & Stegun 1964, §26.7.5) with
/// uniform absolute error `<= 3.17e-13` over all `t in R` (`<= 1.59e-13` for one-sided tails).
///
/// # Errors
/// Returns [`MathError::InvalidParameter`] if `df <= 0` or [`MathError::DomainError`] if `t` is `NaN`.
pub fn student_t_two_sided_p(t: f64, df: f64) -> Result<f64, MathError> {
    check_pos_param("df", df)?;
    if t.is_nan() {
        return Err(MathError::DomainError { name: "t", value: t });
    }
    if t == 0.0 {
        return Ok(1.0);
    }
    if t.is_infinite() {
        return Ok(0.0);
    }
    if df.is_infinite() || df >= HUGE_DF_NORMAL_CUTOFF {
        return Ok(norm_two_sided_p(t));
    }
    let r = t.abs() / df.sqrt();
    if r <= 1.0 {
        reg_beta_xy(1.0, r * r, 0.5 * df, 0.5)
    } else {
        let inv_r2 = 1.0 / (r * r);
        if inv_r2 < f64::MIN_POSITIVE {
            // Asymptotic power tail when r^2 overflows or inv_r2 enters subnormals:
            // I_x(a, 0.5) ~ x^a / (a B(a, 0.5)), x = df / t^2.
            let a = 0.5 * df;
            let log_r = t.abs().ln() - 0.5 * df.ln();
            return Ok((-df * log_r - a.ln() - ln_beta(a, 0.5)?).exp());
        }
        reg_beta_xy(inv_r2, 1.0, 0.5 * df, 0.5)
    }
}

/// Natural logarithm of the two-sided Student `t` probability `ln P(|T| >= |t|)`.
///
/// For `df >= 1e12`, delegates to [`norm_ln_two_sided_p`] (Abramowitz & Stegun 1964, §26.7.5) with
/// leading discrepancy `~ t^4 / (4 * df)` (`<= 2.5e-9` for `|t| <= 10`, `<= 5.3e-7` for `|t| <= 38`).
///
/// # Errors
/// Returns an error if `df <= 0` or `t` is `NaN`.
pub fn student_t_ln_two_sided_p(t: f64, df: f64) -> Result<f64, MathError> {
    check_pos_param("df", df)?;
    if t.is_nan() {
        return Err(MathError::DomainError { name: "t", value: t });
    }
    if t == 0.0 {
        return Ok(0.0);
    }
    if t.is_infinite() {
        return Ok(f64::NEG_INFINITY);
    }
    if df.is_infinite() || df >= HUGE_DF_NORMAL_CUTOFF {
        return Ok(norm_ln_two_sided_p(t));
    }
    let r = t.abs() / df.sqrt();
    if r <= 1.0 {
        ln_reg_beta_xy(1.0, r * r, 0.5 * df, 0.5)
    } else {
        let inv_r2 = 1.0 / (r * r);
        if inv_r2 >= f64::MIN_POSITIVE {
            ln_reg_beta_xy(inv_r2, 1.0, 0.5 * df, 0.5)
        } else {
            // Asymptotic power tail when r^2 overflows or inv_r2 enters subnormals:
            // I_x(a, 0.5) ~ x^a / (a B(a, 0.5)), x = df / t^2.
            let a = 0.5 * df;
            let log_r = t.abs().ln() - 0.5 * df.ln();
            Ok(-df * log_r - a.ln() - ln_beta(a, 0.5)?)
        }
    }
}

/// Student `t` upper-tail survival probability `P(T > t)` without `1.0 - cdf(t)` cancellation.
///
/// # Errors
/// Returns an error if `df <= 0` or `t` is `NaN`.
pub fn student_t_sf(t: f64, df: f64) -> Result<f64, MathError> {
    let p2 = student_t_two_sided_p(t, df)?;
    if t >= 0.0 { Ok(0.5 * p2) } else { Ok(1.0 - 0.5 * p2) }
}

/// Student `t` lower-tail cumulative probability `P(T <= t)`.
///
/// # Errors
/// Returns an error if `df <= 0` or `t` is `NaN`.
pub fn student_t_cdf(t: f64, df: f64) -> Result<f64, MathError> {
    student_t_sf(-t, df)
}

/// Natural logarithm of the Student `t` upper-tail probability `ln P(T > t)`.
///
/// # Errors
/// Returns an error if `df <= 0` or `t` is `NaN`.
pub fn student_t_ln_sf(t: f64, df: f64) -> Result<f64, MathError> {
    if t >= 0.0 {
        Ok(-std::f64::consts::LN_2 + student_t_ln_two_sided_p(t, df)?)
    } else {
        let half_p2 = 0.5 * student_t_two_sided_p(t, df)?;
        Ok((-half_p2).ln_1p())
    }
}

/// Two-sided Student `t` critical value `t_{alpha / 2, df} >= 0` solving `P(|T| >= t) = alpha`.
///
/// For `df >= 1e12`, delegates to [`norm_two_sided_isf`] (Abramowitz & Stegun 1964, §26.7.5) with
/// Cornish-Fisher error `(|z|^3 + |z|) / (4 * df) + O(df^{-2})` (relative error `<= 1.5e-11` for
/// `alpha >= 1e-12`, `<= 1.2e-10` even at `alpha = 1e-100`). When the inverse-beta quantile `x`
/// underflows `exp(-700)`, falls back to the exact power tail `I_x(a, 0.5) ~ x^a / (a B(a, 0.5))`
/// (relative error `O(x) <= 1e-304`).
///
/// # Errors
/// Returns an error if `df <= 0` or `alpha` is outside `[0, 1]`.
pub fn student_t_two_sided_isf(alpha: f64, df: f64) -> Result<f64, MathError> {
    check_pos_param("df", df)?;
    check_prob("alpha", alpha)?;
    if alpha == 0.0 {
        return Ok(f64::INFINITY);
    }
    if alpha == 1.0 {
        return Ok(0.0);
    }
    if df.is_infinite() || df >= HUGE_DF_NORMAL_CUTOFF {
        return norm_two_sided_isf(alpha);
    }
    // Exact Cauchy identity for df == 1: P(|T| >= t) = (2/pi) * atan(1 / t) = alpha.
    if (df - 1.0).abs() == 0.0 {
        return Ok(if alpha < 1e-8 {
            (2.0 / std::f64::consts::PI) / alpha
        } else if alpha <= 0.5 {
            1.0 / (0.5 * std::f64::consts::PI * alpha).tan()
        } else {
            (0.5 * std::f64::consts::PI * (1.0 - alpha)).tan()
        });
    }
    let a = 0.5 * df;
    if alpha < 1e-80 && df < 2.0 {
        let ln_x = (alpha.ln() + a.ln() + ln_beta(a, 0.5)?) / a;
        if ln_x < -690.0 {
            let ln_t = 0.5 * (df.ln() - ln_x);
            return Ok(if ln_t > 709.78 { f64::INFINITY } else { ln_t.exp() });
        }
    }
    match reg_beta_inv_pair(a, 0.5, alpha, 1.0 - alpha) {
        Ok((x, y)) => Ok(df.sqrt() * (y.sqrt() / x.sqrt())),
        Err(MathError::NotConverged { .. }) => {
            let ln_x = (alpha.ln() + a.ln() + ln_beta(a, 0.5)?) / a;
            if alpha < 1e-80 || ln_x <= -700.0 {
                let ln_t = 0.5 * (df.ln() - ln_x);
                return Ok(if ln_t > 709.78 { f64::INFINITY } else { ln_t.exp() });
            }
            Err(MathError::NotConverged { algorithm: "student_t_two_sided_isf", iterations: 0 })
        }
        Err(e) => Err(e),
    }
}

/// Student `t` upper-tail quantile `S^{-1}(q)` solving `P(T > t) = q` directly from `q in [0, 1]`.
///
/// # Errors
/// Returns an error if `df <= 0` or `q` is outside `[0, 1]`.
pub fn student_t_isf(q: f64, df: f64) -> Result<f64, MathError> {
    check_pos_param("df", df)?;
    check_prob("q", q)?;
    if q == 0.0 {
        return Ok(f64::INFINITY);
    }
    if q == 1.0 {
        return Ok(f64::NEG_INFINITY);
    }
    if q == 0.5 {
        return Ok(0.0);
    }
    if q < 0.5 { student_t_two_sided_isf(2.0 * q, df) } else { Ok(-student_t_two_sided_isf(2.0 * (1.0 - q), df)?) }
}

/// Student `t` lower-tail quantile `F^{-1}(p)` solving `P(T <= t) = p`.
///
/// # Errors
/// Returns an error if `df <= 0` or `p` is outside `[0, 1]`.
pub fn student_t_ppf(p: f64, df: f64) -> Result<f64, MathError> {
    check_prob("p", p)?;
    Ok(-student_t_isf(p, df)?)
}

// ============================================================================
// Snedecor's F Distribution (df1 > 0, df2 > 0)
// ============================================================================

/// Natural logarithm of the Snedecor `F(df1, df2)` probability density at `f >= 0`.
///
/// # Errors
/// Returns an error if `df1 <= 0`, `df2 <= 0`, or `f < 0` / `NaN`.
pub fn f_ln_pdf(f: f64, df1: f64, df2: f64) -> Result<f64, MathError> {
    check_pos_param("df1", df1)?;
    check_pos_param("df2", df2)?;
    if f.is_nan() || f < 0.0 {
        return Err(MathError::DomainError { name: "f", value: f });
    }
    if df1.is_infinite() && df2.is_infinite() {
        return Ok(if f == 1.0 { f64::INFINITY } else { f64::NEG_INFINITY });
    }
    if f == 0.0 {
        return if df1 < 2.0 {
            Ok(f64::INFINITY)
        } else if df1 == 2.0 {
            Ok(0.0)
        } else {
            Ok(f64::NEG_INFINITY)
        };
    }
    if f.is_infinite() {
        return Ok(f64::NEG_INFINITY);
    }
    if df2.is_infinite() {
        let a = 0.5 * df1;
        let ax = mul_div3(a, f, 1.0);
        if a >= 12.0 {
            return Ok(if ax.is_finite() && ax > 0.0 {
                -deviance_bd0(a, ax) - f.ln() + 0.5 * a.ln() - HALF_LN_2PI - stirling_err(a)
            } else {
                f64::NEG_INFINITY
            });
        }
        return Ok(a * a.ln() + (a - 1.0) * f.ln() - ax - ln_gamma(a)?);
    }
    if df1.is_infinite() {
        let b = 0.5 * df2;
        let bx = mul_div3(b, 1.0, f);
        if b >= 12.0 {
            return Ok(if bx.is_finite() && bx > 0.0 {
                -deviance_bd0(b, bx) - f.ln() + 0.5 * b.ln() - HALF_LN_2PI - stirling_err(b)
            } else {
                f64::NEG_INFINITY
            });
        }
        return Ok(b * b.ln() - (b + 1.0) * f.ln() - bx - ln_gamma(b)?);
    }
    f_ln_pdf_finite(f, df1, df2)
}

fn f_ln_pdf_finite(f: f64, df1: f64, df2: f64) -> Result<f64, MathError> {
    let a = 0.5 * df1;
    let b = 0.5 * df2;
    let ratio = mul_div3(df1, f, df2);
    if (1e-100..=1e100).contains(&ratio) {
        let (x, y) = if ratio <= 1.0 {
            (ratio / (1.0 + ratio), 1.0 / (1.0 + ratio))
        } else {
            let inv = 1.0 / ratio;
            (1.0 / (1.0 + inv), inv / (1.0 + inv))
        };
        return Ok(ln_beta_kernel(x, y, a, b)? - f.ln());
    }
    let ln_ratio = df1.ln() - df2.ln() + f.ln();
    if a >= 1e250 && b >= 1e250 {
        let half_a = 0.5 * a;
        let half_b = 0.5 * b;
        let half_s = half_a + half_b;
        let ln_s = std::f64::consts::LN_2 + half_s.ln();
        let scale = 0.5 * (a.ln() + b.ln() - ln_s - std::f64::consts::TAU.ln());
        let corr = -stirling_err(a) - stirling_err(b);
        let (half_sx, half_sy) = if ratio <= 1.0 {
            let sx_ln = half_s.ln() + ln_ratio - if ratio > 0.0 { ratio.ln_1p() } else { 0.0 };
            let hsx = if sx_ln < -700.0 { 0.0 } else { sx_ln.exp() };
            (hsx, half_s - hsx)
        } else {
            let sy_ln = half_s.ln() - ln_ratio - if ratio.is_finite() { (1.0 / ratio).ln_1p() } else { 0.0 };
            let hsy = if sy_ln < -700.0 { 0.0 } else { sy_ln.exp() };
            (half_s - hsy, hsy)
        };
        let dev_a = if half_sx == 0.0 { f64::NEG_INFINITY } else { -2.0 * deviance_bd0(half_a, half_sx) };
        let dev_b = if half_sy == 0.0 { f64::NEG_INFINITY } else { -2.0 * deviance_bd0(half_b, half_sy) };
        return Ok(dev_a + dev_b + scale + corr - f.ln());
    }
    let s = a + b;
    if ratio <= 1.0 {
        let ab_ln_1p = if ratio >= f64::MIN_POSITIVE {
            s * ratio.ln_1p()
        } else {
            let prod_ln = s.ln() + ln_ratio;
            if prod_ln < -745.0 { 0.0 } else { prod_ln.exp() }
        };
        Ok(a * ln_ratio - f.ln() - ab_ln_1p - ln_beta(a, b)?)
    } else {
        let inv = mul_div4(df2, 1.0, df1, f);
        let ab_ln_1p_inv = if inv >= f64::MIN_POSITIVE {
            s * inv.ln_1p()
        } else {
            let prod_ln = s.ln() - ln_ratio;
            if prod_ln < -745.0 { 0.0 } else { prod_ln.exp() }
        };
        Ok(-b * ln_ratio - f.ln() - ab_ln_1p_inv - ln_beta(a, b)?)
    }
}

/// Snedecor `F(df1, df2)` probability density at `f >= 0`.
///
/// # Errors
/// Returns an error if `df1 <= 0`, `df2 <= 0`, or `f < 0` / `NaN`.
pub fn f_pdf(f: f64, df1: f64, df2: f64) -> Result<f64, MathError> {
    Ok(f_ln_pdf(f, df1, df2)?.exp())
}

/// Snedecor `F(df1, df2)` lower-tail cumulative probability `P(F <= f)`.
///
/// # Errors
/// Returns an error if `df1 <= 0`, `df2 <= 0`, or `f < 0` / `NaN`.
pub fn f_cdf(f: f64, df1: f64, df2: f64) -> Result<f64, MathError> {
    check_pos_param("df1", df1)?;
    check_pos_param("df2", df2)?;
    if f.is_nan() || f < 0.0 {
        return Err(MathError::DomainError { name: "f", value: f });
    }
    if f == 0.0 {
        return Ok(0.0);
    }
    if df1.is_infinite() && df2.is_infinite() {
        return Ok(if f < 1.0 { 0.0 } else { 1.0 });
    }
    if f.is_infinite() {
        return Ok(1.0);
    }
    if df2.is_infinite() {
        let a = 0.5 * df1;
        let x = mul_div3(a, f, 1.0);
        if x == 0.0 {
            let ln_p = (a * (a.ln() + f.ln()) - ln_gamma(a + 1.0)?).min(0.0);
            return Ok(ln_p.exp().clamp(0.0, 1.0));
        }
        return reg_gamma_p(a, x);
    }
    if df1.is_infinite() {
        let b = 0.5 * df2;
        let x = mul_div3(b, 1.0, f);
        if x == 0.0 {
            if b < 1e-3 {
                let ln_p = (b * (b.ln() - f.ln()) - ln_gamma(b + 1.0)?).min(0.0);
                return Ok((-ln_p.exp_m1()).clamp(0.0, 1.0));
            }
            return Ok(1.0);
        }
        return reg_gamma_q(b, x);
    }
    let ratio = mul_div3(df1, f, df2);
    if ratio == 0.0 {
        let a = 0.5 * df1;
        let ln_r = df1.ln() - df2.ln() + f.ln();
        let ln_c = (a * ln_r - a.ln() - ln_beta(a, 0.5 * df2)?).min(0.0);
        return Ok(ln_c.exp().clamp(0.0, 1.0));
    }
    if ratio <= 1.0 {
        reg_beta_xy(ratio, 1.0, 0.5 * df1, 0.5 * df2)
    } else {
        let inv = mul_div4(df2, 1.0, df1, f);
        if inv == 0.0 {
            return Ok(1.0);
        }
        reg_beta_xy(1.0, inv, 0.5 * df1, 0.5 * df2)
    }
}

/// Snedecor `F(df1, df2)` upper-tail survival probability `P(F > f) = I_{df2 / (df2 + df1 f)}(df2 / 2, df1 / 2)`
/// evaluated directly without `1.0 - cdf` cancellation.
///
/// # Errors
/// Returns an error if `df1 <= 0`, `df2 <= 0`, or `f < 0` / `NaN`.
pub fn f_sf(f: f64, df1: f64, df2: f64) -> Result<f64, MathError> {
    check_pos_param("df1", df1)?;
    check_pos_param("df2", df2)?;
    if f.is_nan() || f < 0.0 {
        return Err(MathError::DomainError { name: "f", value: f });
    }
    if f == 0.0 {
        return Ok(1.0);
    }
    if df1.is_infinite() && df2.is_infinite() {
        return Ok(if f < 1.0 { 1.0 } else { 0.0 });
    }
    if f.is_infinite() {
        return Ok(0.0);
    }
    if df2.is_infinite() {
        let a = 0.5 * df1;
        let x = mul_div3(a, f, 1.0);
        if x == 0.0 {
            if a < 1e-3 {
                let ln_p = (a * (a.ln() + f.ln()) - ln_gamma(a + 1.0)?).min(0.0);
                return Ok((-ln_p.exp_m1()).clamp(0.0, 1.0));
            }
            return Ok(1.0);
        }
        return reg_gamma_q(a, x);
    }
    if df1.is_infinite() {
        let b = 0.5 * df2;
        let x = mul_div3(b, 1.0, f);
        if x == 0.0 {
            let ln_p = (b * (b.ln() - f.ln()) - ln_gamma(b + 1.0)?).min(0.0);
            return Ok(ln_p.exp().clamp(0.0, 1.0));
        }
        return reg_gamma_p(b, x);
    }
    let ratio = mul_div3(df1, f, df2);
    if ratio <= 1.0 {
        reg_beta_xy(1.0, ratio, 0.5 * df2, 0.5 * df1)
    } else {
        let inv = mul_div4(df2, 1.0, df1, f);
        if inv == 0.0 {
            let b = 0.5 * df2;
            let ln_inv = df2.ln() - df1.ln() - f.ln();
            let ln_s = (b * ln_inv - b.ln() - ln_beta(b, 0.5 * df1)?).min(0.0);
            return Ok(ln_s.exp().clamp(0.0, 1.0));
        }
        reg_beta_xy(inv, 1.0, 0.5 * df2, 0.5 * df1)
    }
}

/// Natural logarithm of the Snedecor `F` upper-tail probability `ln P(F > f)`.
///
/// # Errors
/// Returns an error if `df1 <= 0`, `df2 <= 0`, or `f < 0` / `NaN`.
pub fn f_ln_sf(f: f64, df1: f64, df2: f64) -> Result<f64, MathError> {
    check_pos_param("df1", df1)?;
    check_pos_param("df2", df2)?;
    if f.is_nan() || f < 0.0 {
        return Err(MathError::DomainError { name: "f", value: f });
    }
    if f == 0.0 {
        return Ok(0.0);
    }
    if df1.is_infinite() && df2.is_infinite() {
        return Ok(if f < 1.0 { 0.0 } else { f64::NEG_INFINITY });
    }
    if f.is_infinite() {
        return Ok(f64::NEG_INFINITY);
    }
    if df2.is_infinite() {
        let a = 0.5 * df1;
        let x = mul_div3(a, f, 1.0);
        if x == 0.0 {
            if a < 1e-3 {
                let ln_p = (a * (a.ln() + f.ln()) - ln_gamma(a + 1.0)?).min(0.0);
                return Ok(if ln_p >= 0.0 {
                    f64::NEG_INFINITY
                } else if ln_p > -std::f64::consts::LN_2 {
                    (-ln_p.exp_m1()).ln()
                } else {
                    (-ln_p.exp()).ln_1p()
                });
            }
            return Ok(0.0);
        }
        return ln_reg_gamma_q(a, x);
    }
    if df1.is_infinite() {
        let b = 0.5 * df2;
        let x = mul_div3(b, 1.0, f);
        if x == 0.0 {
            return Ok((b * (b.ln() - f.ln()) - ln_gamma(b + 1.0)?).min(0.0));
        }
        return ln_reg_gamma_p(b, x);
    }
    let ratio = mul_div3(df1, f, df2);
    if ratio <= 1.0 {
        ln_reg_beta_xy(1.0, ratio, 0.5 * df2, 0.5 * df1)
    } else {
        let inv = mul_div4(df2, 1.0, df1, f);
        if inv >= f64::MIN_POSITIVE {
            ln_reg_beta_xy(inv, 1.0, 0.5 * df2, 0.5 * df1)
        } else {
            let b = 0.5 * df2;
            let ln_inv = df2.ln() - df1.ln() - f.ln();
            Ok((b * ln_inv - b.ln() - ln_beta(b, 0.5 * df1)?).min(0.0))
        }
    }
}

/// Exact small-`x` asymptotic inversion of `P(a, x) = exp(ln_p)` when `x <= exp(-700)`:
/// `P(a, x) = x^a / Gamma(a + 1) * (1 - a x / (a + 1) + O(x^2))`, so `ln x = (ln p + ln Gamma(a + 1)) / a`
/// matches the true root to `O(1e-304)` (full IEEE-754 `f64` precision; NIST DLMF §8.7.1).
fn gamma_lower_tail_ln_x(a: f64, ln_p: f64) -> Result<Option<f64>, MathError> {
    if !ln_p.is_finite() || ln_p >= 0.0 {
        return Ok(None);
    }
    let ln_x = (ln_p + ln_gamma(a + 1.0)?) / a;
    if ln_x <= -700.0 { Ok(Some(ln_x)) } else { Ok(None) }
}

fn beta_small_tail_ln_x(a: f64, b: f64, ln_p: f64) -> Result<Option<f64>, MathError> {
    if !ln_p.is_finite() || ln_p >= 0.0 {
        return Ok(None);
    }
    let ln_x = (ln_p + a.ln() + ln_beta(a, b)?) / a;
    if ln_x <= -700.0 && b.ln() + ln_x < -40.0 { Ok(Some(ln_x)) } else { Ok(None) }
}

/// Snedecor `F(df1, df2)` upper-tail critical value solving `P(F > f) = q` directly from `q in [0, 1]`.
///
/// # Errors
/// Returns an error if `df1 <= 0`, `df2 <= 0`, or `q` lies outside `[0, 1]`.
pub fn f_isf(q: f64, df1: f64, df2: f64) -> Result<f64, MathError> {
    check_pos_param("df1", df1)?;
    check_pos_param("df2", df2)?;
    check_prob("q", q)?;
    if q == 0.0 {
        return Ok(f64::INFINITY);
    }
    if q == 1.0 {
        return Ok(0.0);
    }
    if df1.is_infinite() && df2.is_infinite() {
        return Ok(1.0);
    }
    if df2.is_infinite() {
        let a = 0.5 * df1;
        return match reg_gamma_q_inv(a, q) {
            Ok(x) => Ok(x / a),
            Err(MathError::NotConverged { .. }) => {
                if let Some(ln_x) = gamma_lower_tail_ln_x(a, (-q).ln_1p())? {
                    let ln_f = ln_x - a.ln();
                    return Ok(if ln_f < -745.2 { 0.0 } else { ln_f.exp() });
                }
                Err(MathError::NotConverged { algorithm: "f_isf", iterations: 0 })
            }
            Err(e) => Err(e),
        };
    }
    if df1.is_infinite() {
        let b = 0.5 * df2;
        return match reg_gamma_p_inv(b, q) {
            Ok(x) => Ok(b / x),
            Err(MathError::NotConverged { .. }) => {
                if let Some(ln_x) = gamma_lower_tail_ln_x(b, q.ln())? {
                    let ln_f = b.ln() - ln_x;
                    return Ok(if ln_f > 709.78 { f64::INFINITY } else { ln_f.exp() });
                }
                Err(MathError::NotConverged { algorithm: "f_isf", iterations: 0 })
            }
            Err(e) => Err(e),
        };
    }
    let b = 0.5 * df2;
    let a = 0.5 * df1;
    if q < 1e-80 && df2 < 2.0 {
        let ln_x = (q.ln() + b.ln() + ln_beta(b, a)?) / b;
        if ln_x < -690.0 && a.ln() + ln_x < -40.0 {
            let ln_f = df2.ln() - df1.ln() - ln_x;
            return Ok(if ln_f > 709.78 { f64::INFINITY } else { ln_f.exp() });
        }
    }
    match reg_beta_inv_pair(b, a, q, 1.0 - q) {
        Ok((x, y)) => Ok(mul_div4(df2, y, df1, x)),
        Err(MathError::NotConverged { .. }) => {
            if q < 1e-80 || beta_small_tail_ln_x(b, a, q.ln())?.is_some() {
                let ln_x = (q.ln() + b.ln() + ln_beta(b, a)?) / b;
                let ln_f = df2.ln() - df1.ln() - ln_x;
                return Ok(if ln_f > 709.78 { f64::INFINITY } else { ln_f.exp() });
            }
            if let Some(ln_y) = beta_small_tail_ln_x(a, b, (-q).ln_1p())? {
                let ln_f = df2.ln() - df1.ln() + ln_y;
                return Ok(if ln_f < -745.2 { 0.0 } else { ln_f.exp() });
            }
            Err(MathError::NotConverged { algorithm: "f_isf", iterations: 0 })
        }
        Err(e) => Err(e),
    }
}

/// Snedecor `F(df1, df2)` lower-tail quantile solving `P(F <= f) = p`.
///
/// # Errors
/// Returns an error if `df1 <= 0`, `df2 <= 0`, or `p` lies outside `[0, 1]`.
pub fn f_ppf(p: f64, df1: f64, df2: f64) -> Result<f64, MathError> {
    check_pos_param("df1", df1)?;
    check_pos_param("df2", df2)?;
    check_prob("p", p)?;
    if p == 0.0 {
        return Ok(0.0);
    }
    if p == 1.0 {
        return Ok(f64::INFINITY);
    }
    if df1.is_infinite() && df2.is_infinite() {
        return Ok(1.0);
    }
    if df2.is_infinite() {
        let a = 0.5 * df1;
        return match reg_gamma_p_inv(a, p) {
            Ok(x) => Ok(x / a),
            Err(MathError::NotConverged { .. }) => {
                if let Some(ln_x) = gamma_lower_tail_ln_x(a, p.ln())? {
                    let ln_f = ln_x - a.ln();
                    return Ok(if ln_f < -745.2 { 0.0 } else { ln_f.exp() });
                }
                Err(MathError::NotConverged { algorithm: "f_ppf", iterations: 0 })
            }
            Err(e) => Err(e),
        };
    }
    if df1.is_infinite() {
        let b = 0.5 * df2;
        return match reg_gamma_q_inv(b, p) {
            Ok(x) => Ok(b / x),
            Err(MathError::NotConverged { .. }) => {
                if let Some(ln_x) = gamma_lower_tail_ln_x(b, (-p).ln_1p())? {
                    let ln_f = b.ln() - ln_x;
                    return Ok(if ln_f > 709.78 { f64::INFINITY } else { ln_f.exp() });
                }
                Err(MathError::NotConverged { algorithm: "f_ppf", iterations: 0 })
            }
            Err(e) => Err(e),
        };
    }
    let a = 0.5 * df1;
    let b = 0.5 * df2;
    if p < 1e-80 && df1 < 2.0 {
        let ln_x = (p.ln() + a.ln() + ln_beta(a, b)?) / a;
        if ln_x < -690.0 && b.ln() + ln_x < -40.0 {
            let ln_f = df2.ln() - df1.ln() + ln_x;
            return Ok(if ln_f < -745.2 { 0.0 } else { ln_f.exp() });
        }
    }
    match reg_beta_inv_pair(a, b, p, 1.0 - p) {
        Ok((x, y)) => Ok(mul_div4(df2, x, df1, y)),
        Err(MathError::NotConverged { .. }) => {
            if p < 1e-80 || beta_small_tail_ln_x(a, b, p.ln())?.is_some() {
                let ln_x = (p.ln() + a.ln() + ln_beta(a, b)?) / a;
                let ln_f = df2.ln() - df1.ln() + ln_x;
                return Ok(if ln_f < -745.2 { 0.0 } else { ln_f.exp() });
            }
            if let Some(ln_y) = beta_small_tail_ln_x(b, a, (-p).ln_1p())? {
                let ln_f = df2.ln() - df1.ln() - ln_y;
                return Ok(if ln_f > 709.78 { f64::INFINITY } else { ln_f.exp() });
            }
            Err(MathError::NotConverged { algorithm: "f_ppf", iterations: 0 })
        }
        Err(e) => Err(e),
    }
}

// ============================================================================
// Chi-Square Distribution chi^2(df) (real df > 0)
// ============================================================================

/// Natural logarithm of the Chi-Square `chi^2(df)` probability density at `x >= 0`.
///
/// # Errors
/// Returns an error if `df <= 0` or `x < 0` / `NaN`.
pub fn chi2_ln_pdf(x: f64, df: f64) -> Result<f64, MathError> {
    check_pos_param("df", df)?;
    if !df.is_finite() {
        return Err(MathError::InvalidParameter { name: "df", value: df });
    }
    if x.is_nan() || x < 0.0 {
        return Err(MathError::DomainError { name: "x", value: x });
    }
    if x == 0.0 {
        return if df < 2.0 {
            Ok(f64::INFINITY)
        } else if df == 2.0 {
            Ok(-std::f64::consts::LN_2)
        } else {
            Ok(f64::NEG_INFINITY)
        };
    }
    if x.is_infinite() {
        return Ok(f64::NEG_INFINITY);
    }
    let k = 0.5 * df;
    Ok((k - 1.0) * x.ln() - 0.5 * x - k * std::f64::consts::LN_2 - ln_gamma(k)?)
}

/// Chi-Square `chi^2(df)` probability density at `x >= 0`.
///
/// # Errors
/// Returns an error if `df <= 0` or `x < 0` / `NaN`.
pub fn chi2_pdf(x: f64, df: f64) -> Result<f64, MathError> {
    Ok(chi2_ln_pdf(x, df)?.exp())
}

/// Chi-Square lower-tail cumulative probability `P(X <= x) = P(df / 2, x / 2)`.
///
/// # Errors
/// Returns an error if `df <= 0` or `x < 0` / `NaN`.
pub fn chi2_cdf(x: f64, df: f64) -> Result<f64, MathError> {
    check_pos_param("df", df)?;
    reg_gamma_p(0.5 * df, 0.5 * x)
}

/// Chi-Square upper-tail survival probability `P(X > x) = Q(df / 2, x / 2)` without `1 - cdf` cancellation.
///
/// # Errors
/// Returns an error if `df <= 0` or `x < 0` / `NaN`.
pub fn chi2_sf(x: f64, df: f64) -> Result<f64, MathError> {
    check_pos_param("df", df)?;
    reg_gamma_q(0.5 * df, 0.5 * x)
}

/// Natural logarithm of the Chi-Square upper-tail probability `ln P(X > x) = ln Q(df / 2, x / 2)`.
///
/// # Errors
/// Returns an error if `df <= 0` or `x < 0` / `NaN`.
pub fn chi2_ln_sf(x: f64, df: f64) -> Result<f64, MathError> {
    check_pos_param("df", df)?;
    ln_reg_gamma_q(0.5 * df, 0.5 * x)
}

/// Chi-Square lower-tail quantile `F^{-1}(p) = 2 * P^{-1}(df / 2, p)`.
///
/// When the true quantile underflows `exp(-700)` (`solve_gamma_log_space` bracket floor), falls
/// back to the exact small-`x` incomplete-gamma power tail `P(a, x) ~ x^a / Gamma(a + 1)`
/// (NIST DLMF §8.7.1, relative error `O(x) <= 1e-304`), returning the exact subnormal or `0.0`.
///
/// # Errors
/// Returns an error if `df <= 0` or `p` lies outside `[0, 1]`.
pub fn chi2_ppf(p: f64, df: f64) -> Result<f64, MathError> {
    check_pos_param("df", df)?;
    let a = 0.5 * df;
    match reg_gamma_p_inv(a, p) {
        Ok(x) => Ok(2.0 * x),
        Err(MathError::NotConverged { .. }) if p > 0.0 && p < 1.0 => {
            if let Some(ln_x) = gamma_lower_tail_ln_x(a, p.ln())? {
                let ln_chi2 = std::f64::consts::LN_2 + ln_x;
                return Ok(if ln_chi2 < -745.2 { 0.0 } else { ln_chi2.exp() });
            }
            Err(MathError::NotConverged { algorithm: "chi2_ppf", iterations: 0 })
        }
        Err(e) => Err(e),
    }
}

/// Chi-Square upper-tail critical value `S^{-1}(q) = 2 * Q^{-1}(df / 2, q)` directly from `q in [0, 1]`.
///
/// When the true quantile underflows `exp(-700)` (e.g. `q` close to `1` with very small `df`),
/// falls back to the exact small-`x` power tail `P(a, x) = 1 - q ~ x^a / Gamma(a + 1)`
/// (NIST DLMF §8.7.1, relative error `O(x) <= 1e-304`).
///
/// # Errors
/// Returns an error if `df <= 0` or `q` lies outside `[0, 1]`.
pub fn chi2_isf(q: f64, df: f64) -> Result<f64, MathError> {
    check_pos_param("df", df)?;
    let a = 0.5 * df;
    match reg_gamma_q_inv(a, q) {
        Ok(x) => Ok(2.0 * x),
        Err(MathError::NotConverged { .. }) if q > 0.0 && q < 1.0 => {
            if let Some(ln_x) = gamma_lower_tail_ln_x(a, (-q).ln_1p())? {
                let ln_chi2 = std::f64::consts::LN_2 + ln_x;
                return Ok(if ln_chi2 < -745.2 { 0.0 } else { ln_chi2.exp() });
            }
            Err(MathError::NotConverged { algorithm: "chi2_isf", iterations: 0 })
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        chi2_cdf, chi2_isf, chi2_ln_sf, chi2_ppf, chi2_sf, f_cdf, f_isf, f_ppf, f_sf, norm_cdf, norm_ln_sf, norm_sf, norm_two_sided_p, student_t_isf,
        student_t_ln_pdf, student_t_ln_sf, student_t_ln_two_sided_p, student_t_sf, student_t_two_sided_isf, student_t_two_sided_p,
    };

    #[test]
    fn student_t_cauchy_df1_and_f_relation_exact() {
        // Cauchy df = 1: p_2(t, 1) = 1 - (2/pi) atan(|t|) = (2/pi) atan(1/|t|).
        for &t in &[0.25, 1.0, 12.706_204_736_432_095, 1e6, 1e20] {
            let p2 = student_t_two_sided_p(t, 1.0).unwrap();
            let exact = (2.0 / std::f64::consts::PI) * (1.0 / t).atan();
            assert!((p2 - exact).abs() <= 2e-15 * exact, "t={t}: {p2} vs {exact}");
        }
        // T^2 ~ F(1, df) for fractional Welch df = 7.35!
        let t = 4.25;
        let df = 7.35;
        let pt = student_t_two_sided_p(t, df).unwrap();
        let pf = f_sf(t * t, 1.0, df).unwrap();
        assert!((pt - pf).abs() <= 2e-15 * pt);
        assert!(student_t_ln_sf(80.0, 5.0).unwrap() < -15.0);
    }

    #[test]
    fn small_upper_tails_never_collapse_to_zero() {
        // Naive `1.0 - cdf(x)` collapses to 0.0 below 1.11e-16; direct `sf` stays accurate to 1e-300!
        let p_norm = norm_sf(10.0);
        assert!((p_norm - 7.619_853_024_160_527e-24).abs() / p_norm < 1e-14);
        assert!((norm_ln_sf(50.0) - (-1250.0 - 0.5 * std::f64::consts::TAU.ln() - 50.0_f64.ln())).abs() < 1e-3);
        assert_eq!(norm_cdf(0.0) + norm_sf(0.0), 1.0);
        assert!((norm_two_sided_p(1.959_963_984_540_054) - 0.05).abs() < 1e-14);

        let p_chi2 = chi2_sf(200.0, 2.0).unwrap(); // df=2: exact exp(-100)!
        let exact_exp = (-100.0_f64).exp();
        assert!((p_chi2 - exact_exp).abs() / exact_exp < 1e-14);
        assert!((chi2_ln_sf(2000.0, 2.0).unwrap() - (-1000.0)).abs() < 1e-12);
        assert!((chi2_cdf(2.0, 2.0).unwrap() - (1.0 - (-1.0_f64).exp())).abs() < 1e-15);

        // Quantile roundtrips at 99.9% CI (alpha = 0.001) and 99.99% CI (alpha = 0.0001):
        let t_crit = student_t_two_sided_isf(1e-4, 14.6).unwrap();
        assert!((student_t_two_sided_p(t_crit, 14.6).unwrap() - 1e-4).abs() < 1e-17);
        assert!((student_t_sf(student_t_isf(1e-20, 8.0).unwrap(), 8.0).unwrap() - 1e-20).abs() < 1e-32);

        let f_crit = f_isf(1e-10, 3.0, 24.0).unwrap();
        assert!((f_sf(f_crit, 3.0, 24.0).unwrap() - 1e-10).abs() < 1e-22);
        assert!(((f_cdf(2.5, 4.0, 15.0).unwrap() + f_sf(2.5, 4.0, 15.0).unwrap()) - 1.0).abs() < 1e-15);

        let c_crit = chi2_isf(1e-15, 5.0).unwrap();
        assert!((chi2_sf(c_crit, 5.0).unwrap() - 1e-15).abs() < 1e-27);
    }

    #[test]
    fn gap_03_02_extreme_lower_tail_quantile_asymptotic_fallbacks() {
        // Exact analytical small-x power tail for chi2(1): P(chi2 <= x) = erf(sqrt(x/2)) ~ sqrt(2 x / pi),
        // so x ~ (pi / 2) * p^2. At p = 1e-200, true quantile is ~1.57e-400 -> exact f64 0.0.
        assert_eq!(chi2_ppf(1e-200, 1.0).unwrap(), 0.0);
        // At p = 1e-161, true quantile is (pi / 2) * 1e-322 ≈ 1.5708e-322 (subnormal f64), whereas
        // x = chi2 / 2 ≈ 7.85e-323 < exp(-740) ≈ 4.21e-322 triggers the asymptotic fallback cleanly.
        let sub = chi2_ppf(1e-161, 1.0).unwrap();
        let expected_sub = (std::f64::consts::FRAC_PI_2.ln() + 2.0 * (1e-161_f64).ln()).exp();
        assert!(sub > 0.0 && (sub - expected_sub).abs() <= 2.0 * f64::from_bits(1) + 1e-12 * expected_sub, "sub={sub:.3e} vs {expected_sub:.3e}");

        // Tiny-df underflowing lower tails in chi2_ppf, chi2_isf, f_ppf, f_isf, and student_t_two_sided_isf:
        assert_eq!(chi2_ppf(0.1, 0.002).unwrap(), 0.0);
        assert_eq!(chi2_isf(0.6, 0.002).unwrap(), 0.0);
        assert_eq!(f_ppf(1e-200, 1.0, f64::INFINITY).unwrap(), 0.0);
        assert_eq!(f_isf(1e-200, f64::INFINITY, 1.0).unwrap(), f64::INFINITY);
        assert_eq!(f_ppf(0.1, 0.002, 10.0).unwrap(), 0.0);
        assert_eq!(f_isf(0.9, 0.002, 10.0).unwrap(), 0.0);
        assert_eq!(f_ppf(0.9, 10.0, 0.002).unwrap(), f64::INFINITY);
        assert_eq!(student_t_two_sided_isf(0.1, 0.002).unwrap(), f64::INFINITY);
    }

    #[test]
    fn gap_03_03_student_t_huge_df_normal_shortcut_bounds() {
        // Verify documented Abramowitz & Stegun §26.7.5 error bounds across the df = 1e12 seam.
        let df_below = 9.99e11_f64;
        let df_at = 1e12_f64;
        for &t in &[0.0_f64, 0.5, 1.0, 1.96, 3.0, 5.0, 10.0] {
            let p2_below = student_t_two_sided_p(t, df_below).unwrap();
            let p2_at = student_t_two_sided_p(t, df_at).unwrap();
            assert!((p2_below - p2_at).abs() <= 3.2e-13, "t={t}: p2 diff={:.3e}", (p2_below - p2_at).abs());

            let sf_below = student_t_sf(t, df_below).unwrap();
            let sf_at = student_t_sf(t, df_at).unwrap();
            assert!((sf_below - sf_at).abs() <= 1.6e-13, "t={t}: sf diff={:.3e}", (sf_below - sf_at).abs());

            let lnp_below = student_t_ln_pdf(t, df_below).unwrap();
            let lnp_at = student_t_ln_pdf(t, df_at).unwrap();
            assert!((lnp_below - lnp_at).abs() <= 2.6e-9, "t={t}: ln_pdf diff={:.3e}", (lnp_below - lnp_at).abs());

            if t > 0.0 {
                let ln2_below = student_t_ln_two_sided_p(t, df_below).unwrap();
                let ln2_at = student_t_ln_two_sided_p(t, df_at).unwrap();
                assert!((ln2_below - ln2_at).abs() <= 2.6e-9, "t={t}: ln_p2 diff={:.3e}", (ln2_below - ln2_at).abs());
            }
        }
        for &alpha in &[0.05_f64, 1e-6, 1e-12] {
            let q_below = student_t_two_sided_isf(alpha, df_below).unwrap();
            let q_at = student_t_two_sided_isf(alpha, df_at).unwrap();
            let rel = (q_below - q_at).abs() / q_at;
            assert!(rel <= 1.5e-11, "alpha={alpha}: rel={rel:.3e}");
        }
    }
}
