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
use crate::special::{
    erfc, erfcx, ln_beta, ln_gamma, ln_reg_beta_xy, ln_reg_gamma_q, norm_isf_raw, norm_ppf_raw, reg_beta_inv_pair, reg_beta_xy, reg_gamma_p,
    reg_gamma_p_inv, reg_gamma_q, reg_gamma_q_inv,
};

const HALF_LN_2PI: f64 = 0.918_938_533_204_672_7;
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
    norm_isf_raw(0.5 * alpha)
}

// ============================================================================
// Student's t Distribution (real degrees of freedom df > 0 or +inf)
// ============================================================================

/// Natural logarithm of the Student `t` probability density with `df > 0`.
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
/// # Errors
/// Returns an error if `df <= 0` or `t` is `NaN`.
pub fn student_t_pdf(t: f64, df: f64) -> Result<f64, MathError> {
    Ok(student_t_ln_pdf(t, df)?.exp())
}

/// Exact two-sided Student `t` *p*-value `p_2(t, df) = I_{df / (df + t^2)}(df / 2, 1 / 2)`
/// supporting fractional Welch-Satterthwaite `df > 0` as well as `df = +inf`.
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
        if inv_r2 == 0.0 {
            return Ok(0.0);
        }
        reg_beta_xy(inv_r2, 1.0, 0.5 * df, 0.5)
    }
}

/// Natural logarithm of the two-sided Student `t` probability `ln P(|T| >= |t|)`.
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
        if inv_r2 > 0.0 {
            ln_reg_beta_xy(inv_r2, 1.0, 0.5 * df, 0.5)
        } else {
            // Asymptotic power tail when r^2 overflows f64: I_x(a, 0.5) ~ x^a / (a B(a, 0.5)), x = 1 / r^2.
            let a = 0.5 * df;
            Ok(-df * r.ln() - a.ln() - ln_beta(a, 0.5)?)
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
    let (x, y) = reg_beta_inv_pair(0.5 * df, 0.5, alpha, 1.0 - alpha)?;
    Ok((df * (y / x)).sqrt())
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

/// Natural logarithm of the Snedecor `F(df1, df2)` density at `f >= 0`.
///
/// # Errors
/// Returns an error if `df1 <= 0`, `df2 <= 0`, or `f < 0` / `NaN`.
pub fn f_ln_pdf(f: f64, df1: f64, df2: f64) -> Result<f64, MathError> {
    check_pos_param("df1", df1)?;
    check_pos_param("df2", df2)?;
    if f.is_nan() || f < 0.0 {
        return Err(MathError::DomainError { name: "f", value: f });
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
    let a = 0.5 * df1;
    let b = 0.5 * df2;
    let ratio = (df1 / df2) * f;
    let ln_one_plus = if ratio <= 1.0 { ratio.ln_1p() } else { ratio.ln() + (1.0 / ratio).ln_1p() };
    Ok(a * (df1 / df2).ln() + (a - 1.0) * f.ln() - (a + b) * ln_one_plus - ln_beta(a, b)?)
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
    if f.is_infinite() {
        return Ok(1.0);
    }
    if df2.is_infinite() {
        return chi2_cdf(df1 * f, df1);
    }
    let ratio = (df1 / df2) * f;
    if ratio <= 1.0 { reg_beta_xy(ratio, 1.0, 0.5 * df1, 0.5 * df2) } else { reg_beta_xy(1.0, 1.0 / ratio, 0.5 * df1, 0.5 * df2) }
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
    if f.is_infinite() {
        return Ok(0.0);
    }
    if df2.is_infinite() {
        return chi2_sf(df1 * f, df1);
    }
    let ratio = (df1 / df2) * f;
    if ratio <= 1.0 { reg_beta_xy(1.0, ratio, 0.5 * df2, 0.5 * df1) } else { reg_beta_xy(1.0 / ratio, 1.0, 0.5 * df2, 0.5 * df1) }
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
    if f.is_infinite() {
        return Ok(f64::NEG_INFINITY);
    }
    if df2.is_infinite() {
        return chi2_ln_sf(df1 * f, df1);
    }
    let ratio = (df1 / df2) * f;
    if ratio <= 1.0 {
        ln_reg_beta_xy(1.0, ratio, 0.5 * df2, 0.5 * df1)
    } else {
        let inv = 1.0 / ratio;
        if inv > 0.0 {
            ln_reg_beta_xy(inv, 1.0, 0.5 * df2, 0.5 * df1)
        } else {
            let b = 0.5 * df2;
            Ok(-b * ratio.ln() - b.ln() - ln_beta(b, 0.5 * df1)?)
        }
    }
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
    if df2.is_infinite() {
        return Ok(chi2_isf(q, df1)? / df1);
    }
    let (x, y) = reg_beta_inv_pair(0.5 * df2, 0.5 * df1, q, 1.0 - q)?;
    Ok((df2 / df1) * (y / x))
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
    if df2.is_infinite() {
        return Ok(chi2_ppf(p, df1)? / df1);
    }
    let (x, y) = reg_beta_inv_pair(0.5 * df1, 0.5 * df2, p, 1.0 - p)?;
    Ok((df2 / df1) * (x / y))
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
/// # Errors
/// Returns an error if `df <= 0` or `p` lies outside `[0, 1]`.
pub fn chi2_ppf(p: f64, df: f64) -> Result<f64, MathError> {
    check_pos_param("df", df)?;
    Ok(2.0 * reg_gamma_p_inv(0.5 * df, p)?)
}

/// Chi-Square upper-tail critical value `S^{-1}(q) = 2 * Q^{-1}(df / 2, q)` directly from `q in [0, 1]`.
///
/// # Errors
/// Returns an error if `df <= 0` or `q` lies outside `[0, 1]`.
pub fn chi2_isf(q: f64, df: f64) -> Result<f64, MathError> {
    check_pos_param("df", df)?;
    Ok(2.0 * reg_gamma_q_inv(0.5 * df, q)?)
}

#[cfg(test)]
mod tests {
    use super::{
        chi2_cdf, chi2_isf, chi2_ln_sf, chi2_sf, f_cdf, f_isf, f_sf, norm_cdf, norm_ln_sf, norm_sf, norm_two_sided_p, student_t_isf, student_t_ln_sf,
        student_t_sf, student_t_two_sided_isf, student_t_two_sided_p,
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
}
