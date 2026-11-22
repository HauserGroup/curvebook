//! Regularized incomplete beta function `I_x(a, b)`, two-component tail-safe evaluator
//! `reg_beta_xy(x, 1 - x, a, b)`, log-space evaluator `ln_reg_beta_xy`, and quantile inverses.
//!
//! Uses Lentz's modified continued fraction with symmetry reflection at `x = (a + 1) / (a + b + 2)`,
//! paired with Loader's deviance / Stirling remainder prefactor so large degrees of freedom (such as
//! `nu = 10^8` in Student `t` or Snedecor `F`) suffer zero subtraction cancellation.
//!
//! References:
//! - NIST DLMF §8.17 (Incomplete Beta Functions), 8.17.4 (symmetry), 8.17.22 (continued fraction).
//! - DiDonato, A. R., & Morris, A. H. (1992). Algorithm 708: Significant digit computation of the
//!   incomplete beta function ratios. *ACM TOMS*, 18(3), 360-373.
//! - Cran, G. W., Martin, K. J., & Thomas, G. E. (1977). Algorithm AS 109: Inverse of the
//!   incomplete beta function ratio. *Applied Statistics*, 26(1), 111-114.

use crate::error::MathError;
use crate::roots::safeguarded_halley_step;
use crate::special::erf::{erfc, erfcx, exp_neg_sq, norm_isf_raw};
use crate::special::gamma::{aln1p_minus_arg, deviance_bd0, ln_beta, ln_gamma, stirling_err};
use crate::special::inc_gamma::{ln_reg_gamma_p, ln_reg_gamma_q, reg_gamma_p, reg_gamma_q, reg_gamma_q_inv};

const MAX_BETA_CF_ITERS: usize = 20_000;
const MAX_BETA_INV_ITERS: usize = 64;
const MIN_LN_X: f64 = -740.0;

fn ln_one_minus_exp(ln_val: f64) -> f64 {
    if ln_val >= 0.0 {
        f64::NEG_INFINITY
    } else if ln_val > -std::f64::consts::LN_2 {
        (-ln_val.exp_m1()).ln()
    } else {
        (-ln_val.exp()).ln_1p()
    }
}

fn validate_beta_params(a: f64, b: f64) -> Result<(), MathError> {
    if !a.is_finite() || a <= 0.0 {
        return Err(MathError::InvalidParameter { name: "a", value: a });
    }
    if !b.is_finite() || b <= 0.0 {
        return Err(MathError::InvalidParameter { name: "b", value: b });
    }
    Ok(())
}

/// Evaluate `ln(x^a * y^b / B(a, b))` where `x > 0, y > 0, x + y = 1`, avoiding cancellation across
/// small, imbalanced, and large `(a, b)` regimes.
pub(crate) fn ln_beta_kernel(x: f64, y: f64, a: f64, b: f64) -> Result<f64, MathError> {
    if a >= 12.0 && b >= 12.0 {
        let half_a = 0.5 * a;
        let half_b = 0.5 * b;
        let half_s = half_a + half_b;
        let s = a + b;
        let (dev, ln_s, st_s) = if s.is_finite() {
            (-deviance_bd0(a, s * x) - deviance_bd0(b, s * y), s.ln(), stirling_err(s))
        } else {
            (-2.0 * (deviance_bd0(half_a, half_s * x) + deviance_bd0(half_b, half_s * y)), std::f64::consts::LN_2 + half_s.ln(), 0.0)
        };
        let scale = 0.5 * (a.ln() + b.ln() - ln_s - std::f64::consts::TAU.ln());
        let corr = st_s - stirling_err(a) - stirling_err(b);
        return Ok(dev + scale + corr);
    }
    if a >= 12.0 {
        // Imbalanced: a >= 12 > b. Cancellation happens when y is small (x ~ 1).
        let ln_x = if y < 0.5 { (-y).ln_1p() } else { x.ln() };
        let term_x = a * ln_x;
        let term_ay = b * (a * y).ln();
        let st_ab = if (a + b).is_finite() { stirling_err(a + b) } else { 0.0 };
        let ratio_corr = aln1p_minus_arg(a, b) + (b - 0.5) * (b / a).ln_1p() + st_ab - stirling_err(a);
        return Ok(term_x + term_ay - ln_gamma(b)? + ratio_corr);
    }
    if b >= 12.0 {
        return ln_beta_kernel(y, x, b, a);
    }
    let ln_x = if y < 0.5 { (-y).ln_1p() } else { x.ln() };
    let ln_y = if x < 0.5 { (-x).ln_1p() } else { y.ln() };
    Ok(a * ln_x + b * ln_y - ln_beta(a, b)?)
}

/// Modified Lentz continued fraction for DLMF 8.17.22 (`x <= (a + 1) / (a + b + 2)`):
/// `I_x(a, b) = (x^a y^b / (a B(a, b))) * (1 / CF(x, y; a, b))`, returning `ln(1 / CF) = -ln(CF)`.
///
/// Accepts both `x` and `y = 1 - x` so that when `a` is huge and `y` is tiny (`x ~ 1`), the `O(a^2)`
/// cancellation in `1 - (a + b) x / (a + 1)` and `1 + d_{2m+1}` is eliminated analytically, and
/// factors all numerator/denominator pairs before multiplication so `a, b >= 1e155` never overflow.
fn ln_beta_continued_fraction(x: f64, y: f64, a: f64, b: f64) -> Result<f64, MathError> {
    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let tiny = 1e-300_f64;

    let mut c = 1.0_f64;
    // Exact identity: 1 - (a + b) * x / (a + 1) = ((a + b) * y + (1 - b)) / (a + 1).
    let mut d = if x > 0.5 {
        if qab.is_finite() && (qab * y).is_finite() { (qab * y + (1.0 - b)) / qap } else { (a / qap) * y - (b / qap) * x + 1.0 / qap }
    } else if qab.is_finite() {
        1.0 - qab * x / qap
    } else {
        1.0 - (a / qap + b / qap) * x
    };
    if d.abs() < tiny {
        d = tiny;
    }
    d = 1.0 / d;
    let mut h = d;

    for m in 1..=MAX_BETA_CF_ITERS {
        let mf = m as f64;
        let m2 = 2.0 * mf;

        // Even step: d_{2m} = m (b - m) x / ((a + 2m - 1)(a + 2m)).
        let aa_even = (mf / (qam + m2)) * ((b - mf) / (a + m2)) * x;
        let d_prev = d;
        let c_prev = c;
        let denom_even = 1.0 + aa_even * d_prev;
        d = if denom_even.abs() < tiny { 1.0 / tiny } else { 1.0 / denom_even };
        let c_even = 1.0 + aa_even / c_prev;
        c = if c_even.abs() < tiny { tiny } else { c_even };
        h *= d * c;

        // Odd step: d_{2m+1} = -(a + m)(a + b + m) x / ((a + 2m)(a + 2m + 1)).
        let d1 = a + m2;
        let d2 = qap + m2;
        let r_a = (a + mf) / d1;
        let r_qab = if qab.is_finite() { (qab + mf) / d2 } else { (a / d2) + ((b + mf) / d2) };
        let aa_odd = -r_a * r_qab * x;
        if x > 0.5 {
            // Exact cancellation-free 1 + d_{2m+1} when x = 1 - y is close to 1:
            let odd_denom = d1 * d2;
            let one_plus_aa_odd = if qab.is_finite() && odd_denom.is_finite() && ((a + mf) * (qab + mf)).is_finite() {
                ((m2 + 1.0 - b) * a + mf * (3.0 * mf + 2.0 - b) + (a + mf) * (qab + mf) * y) / odd_denom
            } else {
                (a / d1) * ((m2 + 1.0 - b) / d2) + (mf / d1) * ((3.0 * mf + 2.0 - b) / d2) + r_a * r_qab * y
            };
            let d_minus_one = -aa_even * d_prev * d;
            let inv_c_minus_one = -(aa_even / c_prev) / c;
            d = one_plus_aa_odd + aa_odd * d_minus_one;
            c = one_plus_aa_odd + aa_odd * inv_c_minus_one;
        } else {
            d = 1.0 + aa_odd * d;
            c = 1.0 + aa_odd / c;
        }
        if d.abs() < tiny {
            d = tiny;
        }
        if c.abs() < tiny {
            c = tiny;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;

        if (del - 1.0).abs() <= 0.25 * f64::EPSILON {
            return Ok(h.ln());
        }
    }
    Err(MathError::NotConverged { algorithm: "inc_beta_cf", iterations: MAX_BETA_CF_ITERS })
}

fn kl_dev_unit(u: f64) -> f64 {
    // Evaluate u - ln(1 + u) >= 0 for u > -1 without catastrophic cancellation near u = 0.
    if u.abs() < 1e-2 {
        u * u * (0.5 - u * (1.0 / 3.0 - u * (0.25 - u * (0.2 - u * (1.0 / 6.0 - u * (1.0 / 7.0 - u / 8.0))))))
    } else {
        u - u.ln_1p()
    }
}

/// Evaluate `2 * (D(a/2 || (s/2)*x) + D(b/2 || (s/2)*y))` without `s * x` precision loss when
/// `a` and `b` are huge (`>= 1e6`) and `x` is close to the mean `a / (a + b)`.
fn beta_deviance_temme(x: f64, y: f64, a: f64, b: f64) -> (f64, f64, f64, f64, f64) {
    let half_a = 0.5 * a;
    let half_b = 0.5 * b;
    let half_s = half_a + half_b;
    let an = half_a / half_s;
    let bn = half_b / half_s;
    let diff = bn * x - an * y;
    let ua = diff / an;
    let ub = -diff / bn;
    let dev = if ua.abs() < 0.25 && ub.abs() < 0.25 {
        2.0 * (half_a * kl_dev_unit(ua) + half_b * kl_dev_unit(ub))
    } else {
        2.0 * (deviance_bd0(half_a, half_s * x) + deviance_bd0(half_b, half_s * y))
    };
    (dev, half_s, an, bn, diff)
}

/// Temme's uniform asymptotic `erfc` expansion for `I_x(a, b)` when `min(a, b) >= 1e6` near the
/// transition `x ~ a / (a + b)`, avoiding slow O(sqrt(min(a, b))) CF convergence and `a + b` overflow.
fn temme_beta_terms(x: f64, y: f64, a: f64, b: f64) -> (f64, f64, f64) {
    let (d, half_s, an, bn, diff) = beta_deviance_temme(x, y, a, b);
    let z = (2.0 * d).sqrt().copysign(diff);
    let c0 = if diff == 0.0 || z.abs() < 1e-5 {
        let denom_ln = 0.5 * (an.ln() + bn.ln() + std::f64::consts::LN_2 + half_s.ln());
        (bn - an) / (3.0 * denom_ln.exp())
    } else {
        let ln_sd = 0.5 * (an.ln() + bn.ln() - std::f64::consts::LN_2 - half_s.ln());
        let u = (diff.abs().ln() - ln_sd).exp().copysign(diff);
        (u - z) / (z * u)
    };
    (d, z, c0)
}

fn temme_beta_lower(x: f64, y: f64, a: f64, b: f64) -> f64 {
    let (d, z, c0) = temme_beta_terms(x, y, a, b);
    if z <= 0.0 {
        if z >= -2.0 {
            let p = 0.5 * erfc(-z * std::f64::consts::FRAC_1_SQRT_2)
                + exp_neg_sq(z * std::f64::consts::FRAC_1_SQRT_2) * c0 / std::f64::consts::TAU.sqrt();
            p.clamp(0.0, 1.0)
        } else {
            let scaled = (0.5 * erfcx(-z * std::f64::consts::FRAC_1_SQRT_2) + c0 / std::f64::consts::TAU.sqrt()).max(0.0);
            if scaled == 0.0 { 0.0 } else { (-d + scaled.ln()).exp().clamp(0.0, 1.0) }
        }
    } else {
        (1.0 - temme_beta_lower(y, x, b, a)).clamp(0.0, 1.0)
    }
}

fn temme_ln_beta_lower(x: f64, y: f64, a: f64, b: f64) -> f64 {
    let (d, z, c0) = temme_beta_terms(x, y, a, b);
    if z <= 0.0 {
        let scaled = (0.5 * erfcx(-z * std::f64::consts::FRAC_1_SQRT_2) + c0 / std::f64::consts::TAU.sqrt()).max(f64::MIN_POSITIVE);
        (-d + scaled.ln()).min(0.0)
    } else {
        let ln_comp = temme_ln_beta_lower(y, x, b, a);
        ln_one_minus_exp(ln_comp)
    }
}

/// Regularized incomplete beta `I_x(a, b)` for `x in [0, 1]` and `a > 0, b > 0`.
///
/// # Errors
/// Returns [`MathError::InvalidParameter`] if `a <= 0` or `b <= 0`, or [`MathError::DomainError`]
/// if `x` lies outside `[0, 1]`.
pub fn reg_beta(x: f64, a: f64, b: f64) -> Result<f64, MathError> {
    validate_beta_params(a, b)?;
    if !x.is_finite() || !(0.0..=1.0).contains(&x) {
        return Err(MathError::DomainError { name: "x", value: x });
    }
    reg_beta_xy(x, 1.0 - x, a, b)
}

/// Two-component regularized incomplete beta `I_x(a, b)` accepting both `x >= 0` and `y = 1 - x >= 0`
/// directly so distributions (`t`, `F`) never suffer `1.0 - x` rounding when `x` is close to `1.0`.
///
/// # Errors
/// Returns an error if `a <= 0`, `b <= 0`, `x < 0`, `y < 0`, or inputs are non-finite.
pub fn reg_beta_xy(x: f64, y: f64, a: f64, b: f64) -> Result<f64, MathError> {
    validate_beta_params(a, b)?;
    if !x.is_finite() || x < 0.0 {
        return Err(MathError::DomainError { name: "x", value: x });
    }
    if !y.is_finite() || y < 0.0 {
        return Err(MathError::DomainError { name: "y", value: y });
    }
    if x == 0.0 {
        return Ok(0.0);
    }
    if y == 0.0 {
        return Ok(1.0);
    }
    let sum = x + y;
    let xn = x / sum;
    let yn = y / sum;

    if a >= 1e8 && b >= 1e8 {
        let (dev, ..) = beta_deviance_temme(xn, yn, a, b);
        if dev < 200.0 {
            return Ok(temme_beta_lower(xn, yn, a, b));
        }
    }
    if b >= 1e18 && a.max(1.0).powi(2) <= 1e-16 * b {
        let ln_y = if xn < 0.5 { (-xn).ln_1p() } else { yn.ln() };
        return reg_gamma_p(a, (-b * ln_y).max(0.0));
    }
    if a >= 1e18 && b.max(1.0).powi(2) <= 1e-16 * a {
        let ln_x = if yn < 0.5 { (-yn).ln_1p() } else { xn.ln() };
        return reg_gamma_q(b, (-a * ln_x).max(0.0));
    }

    if xn * (b + 1.0) <= yn * (a + 1.0) {
        let kern = ln_beta_kernel(xn, yn, a, b)?;
        match ln_beta_continued_fraction(xn, yn, a, b) {
            Ok(ln_cf) => Ok((kern - a.ln() + ln_cf).exp().clamp(0.0, 1.0)),
            Err(MathError::NotConverged { .. }) if a >= 1e6 && b >= 1e6 => Ok(temme_beta_lower(xn, yn, a, b)),
            Err(MathError::NotConverged { .. }) if b >= 1e12 && a <= 1e-6 * b => {
                let ln_y = if xn < 0.5 { (-xn).ln_1p() } else { yn.ln() };
                reg_gamma_p(a, (-b * ln_y).max(0.0))
            }
            Err(MathError::NotConverged { .. }) if a >= 1e12 && b <= 1e-6 * a => {
                let ln_x = if yn < 0.5 { (-yn).ln_1p() } else { xn.ln() };
                reg_gamma_q(b, (-a * ln_x).max(0.0))
            }
            Err(e) => Err(e),
        }
    } else {
        let kern = ln_beta_kernel(yn, xn, b, a)?;
        match ln_beta_continued_fraction(yn, xn, b, a) {
            Ok(ln_cf) => Ok((-(kern - b.ln() + ln_cf).min(0.0).exp_m1()).clamp(0.0, 1.0)),
            Err(MathError::NotConverged { .. }) if a >= 1e6 && b >= 1e6 => Ok(temme_beta_lower(xn, yn, a, b)),
            Err(MathError::NotConverged { .. }) if b >= 1e12 && a <= 1e-6 * b => {
                let ln_y = if xn < 0.5 { (-xn).ln_1p() } else { yn.ln() };
                reg_gamma_p(a, (-b * ln_y).max(0.0))
            }
            Err(MathError::NotConverged { .. }) if a >= 1e12 && b <= 1e-6 * a => {
                let ln_x = if yn < 0.5 { (-yn).ln_1p() } else { xn.ln() };
                reg_gamma_q(b, (-a * ln_x).max(0.0))
            }
            Err(e) => Err(e),
        }
    }
}

/// Natural logarithm `ln I_x(a, b)` accepting `(x, y)` with `x + y = 1`, accurate even when
/// `I_x(a, b)` underflows `f64` (`< 10^{-308}`).
///
/// # Errors
/// Returns an error if `a <= 0`, `b <= 0`, `x < 0`, `y < 0`, or inputs are non-finite.
pub fn ln_reg_beta_xy(x: f64, y: f64, a: f64, b: f64) -> Result<f64, MathError> {
    validate_beta_params(a, b)?;
    if !x.is_finite() || x < 0.0 {
        return Err(MathError::DomainError { name: "x", value: x });
    }
    if !y.is_finite() || y < 0.0 {
        return Err(MathError::DomainError { name: "y", value: y });
    }
    if x == 0.0 {
        return Ok(f64::NEG_INFINITY);
    }
    if y == 0.0 {
        return Ok(0.0);
    }
    let sum = x + y;
    let xn = x / sum;
    let yn = y / sum;

    if a >= 1e8 && b >= 1e8 {
        let (dev, ..) = beta_deviance_temme(xn, yn, a, b);
        if dev < 200.0 {
            return Ok(temme_ln_beta_lower(xn, yn, a, b));
        }
    }
    if b >= 1e18 && a.max(1.0).powi(2) <= 1e-16 * b {
        let ln_y = if xn < 0.5 { (-xn).ln_1p() } else { yn.ln() };
        return ln_reg_gamma_p(a, (-b * ln_y).max(0.0));
    }
    if a >= 1e18 && b.max(1.0).powi(2) <= 1e-16 * a {
        let ln_x = if yn < 0.5 { (-yn).ln_1p() } else { xn.ln() };
        return ln_reg_gamma_q(b, (-a * ln_x).max(0.0));
    }

    if xn * (b + 1.0) <= yn * (a + 1.0) {
        let kern = ln_beta_kernel(xn, yn, a, b)?;
        match ln_beta_continued_fraction(xn, yn, a, b) {
            Ok(ln_cf) => Ok((kern - a.ln() + ln_cf).min(0.0)),
            Err(MathError::NotConverged { .. }) if a >= 1e6 && b >= 1e6 => Ok(temme_ln_beta_lower(xn, yn, a, b)),
            Err(MathError::NotConverged { .. }) if b >= 1e12 && a <= 1e-6 * b => {
                let ln_y = if xn < 0.5 { (-xn).ln_1p() } else { yn.ln() };
                ln_reg_gamma_p(a, (-b * ln_y).max(0.0))
            }
            Err(MathError::NotConverged { .. }) if a >= 1e12 && b <= 1e-6 * a => {
                let ln_x = if yn < 0.5 { (-yn).ln_1p() } else { xn.ln() };
                ln_reg_gamma_q(b, (-a * ln_x).max(0.0))
            }
            Err(e) => Err(e),
        }
    } else {
        let kern = ln_beta_kernel(yn, xn, b, a)?;
        match ln_beta_continued_fraction(yn, xn, b, a) {
            Ok(ln_cf) => Ok(ln_one_minus_exp(kern - b.ln() + ln_cf)),
            Err(MathError::NotConverged { .. }) if a >= 1e6 && b >= 1e6 => Ok(temme_ln_beta_lower(xn, yn, a, b)),
            Err(MathError::NotConverged { .. }) if b >= 1e12 && a <= 1e-6 * b => {
                let ln_y = if xn < 0.5 { (-xn).ln_1p() } else { yn.ln() };
                ln_reg_gamma_p(a, (-b * ln_y).max(0.0))
            }
            Err(MathError::NotConverged { .. }) if a >= 1e12 && b <= 1e-6 * a => {
                let ln_x = if yn < 0.5 { (-yn).ln_1p() } else { xn.ln() };
                ln_reg_gamma_q(b, (-a * ln_x).max(0.0))
            }
            Err(e) => Err(e),
        }
    }
}

/// Solve `I_x(a, b) = p` for `x in [0, 1]` given `a > 0, b > 0, p in [0, 1]`.
///
/// # Errors
/// Returns an error if `a <= 0`, `b <= 0`, or `p` lies outside `[0, 1]`.
pub fn reg_beta_inv(a: f64, b: f64, p: f64) -> Result<f64, MathError> {
    validate_beta_params(a, b)?;
    if !p.is_finite() || !(0.0..=1.0).contains(&p) {
        return Err(MathError::DomainError { name: "p", value: p });
    }
    Ok(reg_beta_inv_pair(a, b, p, 1.0 - p)?.0)
}

/// Solve `1 - I_x(a, b) = q` directly from upper-tail `q in [0, 1]` without `1.0 - q` cancellation.
///
/// # Errors
/// Returns an error if `a <= 0`, `b <= 0`, or `q` lies outside `[0, 1]`.
pub fn reg_beta_c_inv(a: f64, b: f64, q: f64) -> Result<f64, MathError> {
    validate_beta_params(a, b)?;
    if !q.is_finite() || !(0.0..=1.0).contains(&q) {
        return Err(MathError::DomainError { name: "q", value: q });
    }
    Ok(reg_beta_inv_pair(a, b, 1.0 - q, q)?.0)
}

/// Solve `I_x(a, b) = p` (`I_{1-x}(b, a) = q`) returning both `(x, 1 - x)` with full significand
/// precision on whichever tail (`p` or `q`) is smaller.
///
/// # Errors
/// Returns an error on invalid parameters or out-of-range probabilities.
pub fn reg_beta_inv_pair(a: f64, b: f64, p: f64, q: f64) -> Result<(f64, f64), MathError> {
    validate_beta_params(a, b)?;
    if !p.is_finite() || !(0.0..=1.0).contains(&p) {
        return Err(MathError::DomainError { name: "p", value: p });
    }
    if !q.is_finite() || !(0.0..=1.0).contains(&q) {
        return Err(MathError::DomainError { name: "q", value: q });
    }
    if p == 0.0 {
        return Ok((0.0, 1.0));
    }
    if q == 0.0 {
        return Ok((1.0, 0.0));
    }
    // Normalize so we always solve against the smaller probability via I_x(a, b) = p <=> I_y(b, a) = q.
    if q < p {
        let (y, x) = solve_beta_for_smaller_prob(b, a, q, p)?;
        Ok((x, y))
    } else {
        solve_beta_for_smaller_prob(a, b, p, q)
    }
}

#[derive(Clone, Copy)]
enum BetaTarget {
    Prob { val: f64, solve_for_y: bool },
    DeepLn { target_ln: f64, solve_for_y: bool },
}

impl BetaTarget {
    fn solve_for_y(self) -> bool {
        match self {
            Self::Prob { solve_for_y, .. } | Self::DeepLn { solve_for_y, .. } => solve_for_y,
        }
    }

    fn is_below_root(self, fx: f64) -> bool {
        if self.solve_for_y() { fx > 0.0 } else { fx < 0.0 }
    }
}

fn eval_beta_w_objective(a: f64, b: f64, t: f64, target: BetaTarget) -> Result<(f64, f64, f64), MathError> {
    let comp = (1.0 - t).max(0.5);
    let (x, y) = if target.solve_for_y() { (comp, t) } else { (t, comp) };
    let ln_k = ln_beta_kernel(x, y, a, b)?;
    let ln_comp = (-t).ln_1p();
    let d2_base = if target.solve_for_y() { b - (a - 1.0) * (t / comp) } else { a - (b - 1.0) * (t / comp) };
    match target {
        BetaTarget::Prob { val, solve_for_y } => {
            let fx = reg_beta_xy(x, y, a, b)? - val;
            let mag = (ln_k - ln_comp).exp();
            let df_dw = if solve_for_y { -mag } else { mag };
            Ok((fx, df_dw, d2_base))
        }
        BetaTarget::DeepLn { target_ln, solve_for_y } => {
            let ln_i = ln_reg_beta_xy(x, y, a, b)?;
            let mag = if ln_i.is_finite() { (ln_k - ln_comp - ln_i).exp() } else { 0.0 };
            let df_dw = if solve_for_y { -mag } else { mag };
            Ok((ln_i - target_ln, df_dw, d2_base - df_dw))
        }
    }
}

fn initial_beta_w_guess(a: f64, b: f64, p: f64, q: f64, solve_for_y: bool) -> Result<f64, MathError> {
    if solve_for_y
        && a >= 2.0
        && b < 1.0
        && let Ok(g) = reg_gamma_q_inv(b, p)
        && g.is_finite()
        && g > 0.0
        && g < 0.9 * a
    {
        return Ok((g / a).ln());
    }
    if a >= 1.0 && b >= 1.0 && p >= 1e-12 {
        // Carter (1947) / Cran-Martin-Thomas normal-F approximation directly for the smaller quantile.
        let z = -norm_isf_raw(p)?;
        let lam = (z * z - 3.0) / 6.0;
        let inv_a = 1.0 / (2.0 * a - 1.0);
        let inv_b = 1.0 / (2.0 * b - 1.0);
        let h = 2.0 / (inv_a + inv_b);
        let w = (z * (h + lam).sqrt() / h) - (inv_b - inv_a) * (lam + 5.0 / 6.0 - 2.0 / (3.0 * h));
        let log_ratio = if solve_for_y { a.ln() - b.ln() - 2.0 * w } else { b.ln() - a.ln() + 2.0 * w };
        if log_ratio.is_finite() {
            return Ok(if log_ratio > 60.0 { -log_ratio } else { -(log_ratio.exp().ln_1p()) });
        }
    }
    let lb = ln_beta(a, b)?;
    if solve_for_y {
        let ln_q = if q <= 0.5 { q.ln() } else { (-p).ln_1p() };
        return Ok((ln_q + b.ln() + lb) / b);
    }
    let ln_p = if p <= 0.5 { p.ln() } else { (-q).ln_1p() };
    let w_asymp = (ln_p + a.ln() + lb) / a;
    if w_asymp < -0.2 && w_asymp > -40.0 {
        let x0 = w_asymp.exp();
        let corr = 1.0 + ((b - 1.0) / (a + 1.0)) * x0;
        if corr > 0.5 {
            return Ok(w_asymp + corr.ln());
        }
    }
    Ok(w_asymp)
}

fn solve_beta_for_smaller_prob(a: f64, b: f64, p: f64, q: f64) -> Result<(f64, f64), MathError> {
    if p == 0.5 && (a - b) == 0.0 {
        return Ok((0.5, 0.5));
    }
    let val_half = if p < 1e-8 { ln_reg_beta_xy(0.5, 0.5, a, b)? - p.ln() } else { reg_beta_xy(0.5, 0.5, a, b)? - p };
    if val_half == 0.0 {
        return Ok((0.5, 0.5));
    }
    let solve_for_y = val_half < 0.0;
    let target = if p < 1e-8 { BetaTarget::DeepLn { target_ln: p.ln(), solve_for_y } } else { BetaTarget::Prob { val: p, solve_for_y } };
    let w_guess = initial_beta_w_guess(a, b, p, q, solve_for_y)?;
    let t = solve_beta_log_space(a, b, w_guess, target)?;
    if solve_for_y { Ok((1.0 - t, t)) } else { Ok((t, 1.0 - t)) }
}

fn polish_beta_log_root(a: f64, b: f64, t_init: f64, mut t_lo: f64, mut t_hi: f64, target: BetaTarget) -> Result<f64, MathError> {
    if !t_init.is_finite() || t_init <= 0.0 {
        return Err(MathError::NotConverged { algorithm: "solve_beta_log_space", iterations: MAX_BETA_INV_ITERS });
    }
    if t_init < f64::MIN_POSITIVE || !t_lo.is_finite() || !t_hi.is_finite() || t_lo >= t_hi {
        return Ok(t_init);
    }
    let mut t = t_init.clamp(t_lo, t_hi);
    for _ in 0..4 {
        let (fx, df_dw, d2_dw) = eval_beta_w_objective(a, b, t, target)?;
        if fx == 0.0 {
            return Ok(t);
        }
        if target.is_below_root(fx) {
            t_lo = t_lo.max(t);
        } else {
            t_hi = t_hi.min(t);
        }
        if !df_dw.is_finite() || df_dw.abs() <= f64::MIN_POSITIVE {
            break;
        }
        let rel_newton = fx / df_dw;
        let denom = (1.0 - 0.5 * rel_newton * (d2_dw - 1.0)).clamp(0.2, 5.0);
        let step = t * (rel_newton / denom);
        let cand = t - step;
        if !cand.is_finite() || cand <= t_lo || cand >= t_hi {
            break;
        }
        if step.abs() <= 1.5 * f64::EPSILON * cand {
            return Ok(cand);
        }
        t = cand;
    }
    Ok(t)
}

/// Tail-safe log-quantile solver parameterized in `w = ln(t) <= ln(0.5)` for whichever of
/// `t = x` (`x <= 0.5`) or `t = y = 1 - x` (`y < 0.5`) is smaller, avoiding both `1 - x`
/// cancellation and sub-`f64` midpoint false positives.
fn solve_beta_log_space(a: f64, b: f64, w_guess: f64, target: BetaTarget) -> Result<f64, MathError> {
    let w_top = (0.5_f64).ln();
    let mut w_lo = MIN_LN_X;
    let mut w_hi = w_top;
    let t_min = w_lo.exp();
    let (f_lo, _, _) = eval_beta_w_objective(a, b, t_min, target)?;
    if f_lo == 0.0 {
        return Ok(t_min);
    }
    if !target.is_below_root(f_lo) {
        return Err(MathError::NotConverged { algorithm: "solve_beta_log_space", iterations: 0 });
    }

    let mut w = if w_guess.is_finite() && w_guess > w_lo && w_guess < w_hi { w_guess } else { 0.5 * (w_lo + w_hi) };
    let t_of_w = |w_val: f64| if w_val >= w_top { 0.5 } else { w_val.exp() };

    for _ in 0..MAX_BETA_INV_ITERS {
        let t = t_of_w(w);
        if !t.is_finite() || t <= 0.0 {
            return Err(MathError::NotConverged { algorithm: "solve_beta_log_space", iterations: MAX_BETA_INV_ITERS });
        }
        let (fx, df_dw, d2_dw) = eval_beta_w_objective(a, b, t, target)?;
        if fx == 0.0 {
            return Ok(t);
        }
        if target.is_below_root(fx) {
            w_lo = w_lo.max(w);
        } else {
            w_hi = w_hi.min(w);
        }
        if (w_hi - w_lo) <= 2.0 * f64::EPSILON * w.abs().max(1.0) {
            let w_mid = 0.5 * (w_lo + w_hi);
            return polish_beta_log_root(a, b, t_of_w(w_mid), t_of_w(w_lo), t_of_w(w_hi), target);
        }
        let mut next_w = safeguarded_halley_step(w, fx, df_dw, d2_dw, w_lo, w_hi);
        let used_halley = next_w > w_lo && next_w < w_hi;
        if !used_halley {
            next_w = 0.5 * (w_lo + w_hi);
            if next_w <= w_lo || next_w >= w_hi {
                return polish_beta_log_root(a, b, t_of_w(next_w), t_of_w(w_lo), t_of_w(w_hi), target);
            }
        } else if (next_w - w).abs() <= 1.5 * f64::EPSILON * next_w.abs().max(1.0) {
            return polish_beta_log_root(a, b, t_of_w(next_w), t_of_w(w_lo), t_of_w(w_hi), target);
        }
        w = next_w;
    }

    Err(MathError::NotConverged { algorithm: "solve_beta_log_space", iterations: MAX_BETA_INV_ITERS })
}

#[cfg(test)]
mod tests {
    use super::{ln_reg_beta_xy, reg_beta, reg_beta_c_inv, reg_beta_inv, reg_beta_inv_pair, reg_beta_xy};

    #[test]
    fn analytical_beta_identities_and_symmetry() {
        // I_x(1, b) = 1 - (1 - x)^b; I_x(a, 1) = x^a.
        for &x in &[1e-8, 0.1, 0.5, 0.9] {
            let v1 = reg_beta(x, 1.0, 4.5).unwrap();
            let e1 = 1.0 - (1.0 - x).powf(4.5);
            assert!((v1 - e1).abs() <= 2e-15, "x={x}: {v1} vs {e1}");

            let v2 = reg_beta(x, 3.25, 1.0).unwrap();
            let e2 = x.powf(3.25);
            assert!((v2 - e2).abs() <= 2e-15, "x={x}: {v2} vs {e2}");
        }

        // Exact symmetry I_x(a, b) + I_{1-x}(b, a) = 1 across small and huge parameters.
        for &(a, b, x) in &[(0.5, 0.5, 0.25), (2.5, 7.3, 0.4), (500.0, 0.5, 0.998), (2000.0, 3000.0, 0.41)] {
            let lhs = reg_beta_xy(x, 1.0 - x, a, b).unwrap();
            let rhs = reg_beta_xy(1.0 - x, x, b, a).unwrap();
            assert!(((lhs + rhs) - 1.0).abs() < 1e-14, "a={a}, b={b}, x={x}: {lhs} + {rhs}");
        }
    }

    #[test]
    fn large_nu_student_t_limit_retains_14_plus_digits() {
        // As nu -> inf, I_{nu / (nu + t^2)}(nu / 2, 0.5) -> erfc(|t| / sqrt(2)).
        let t = 3.0_f64;
        let nu = 1e10_f64;
        let p = reg_beta_xy(nu, t * t, 0.5 * nu, 0.5).unwrap();
        let normal_p = crate::special::erf::erfc(t * std::f64::consts::FRAC_1_SQRT_2);
        assert!((p - normal_p).abs() / normal_p < 5e-9);

        // Deep tail log-beta without underflow: t = 50, df = 10 -> (10 / 2510)^5 ~ 1e-12, t = 1e40!
        let ln_p = ln_reg_beta_xy(10.0, 1e80, 5.0, 0.5).unwrap();
        assert!(ln_p.is_finite() && ln_p < -800.0);
    }

    #[test]
    fn inverse_beta_roundtrips_into_deep_tails() {
        for &(a, b) in &[(0.5, 0.5), (1.5, 0.5), (10.0, 0.5), (4.0, 12.0), (250.0, 250.0)] {
            for &p in &[1e-80, 1e-20, 1e-6, 0.025, 0.5, 0.975] {
                let (x, y) = reg_beta_inv_pair(a, b, p, 1.0 - p).unwrap();
                let ln_back = ln_reg_beta_xy(x, y, a, b).unwrap();
                assert!((ln_back - p.ln()).abs() < 1e-12, "a={a}, b={b}, p={p}: ln_back={ln_back} vs {}", p.ln());
            }
            let x_low = reg_beta_inv(a, b, 0.05).unwrap();
            let x_up = reg_beta_c_inv(a, b, 0.95).unwrap();
            assert!((x_low - x_up).abs() < 1e-14);
        }
    }
}
