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
use crate::special::erf::norm_isf_raw;
use crate::special::gamma::{aln1p_minus_arg, deviance_bd0, ln_beta, ln_gamma, stirling_err};

const MAX_BETA_CF_ITERS: usize = 20_000;

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
        let s = a + b;
        let dev = -deviance_bd0(a, s * x) - deviance_bd0(b, s * y);
        let scale = 0.5 * ((a * b) / (std::f64::consts::TAU * s)).ln();
        let corr = stirling_err(s) - stirling_err(a) - stirling_err(b);
        return Ok(dev + scale + corr);
    }
    if a >= 12.0 {
        // Imbalanced: a >= 12 > b. Cancellation happens when y is small (x ~ 1).
        let ln_x = if y < 0.5 { (-y).ln_1p() } else { x.ln() };
        let term_x = a * ln_x;
        let term_ay = b * (a * y).ln();
        let ratio_corr = aln1p_minus_arg(a, b) + (b - 0.5) * (b / a).ln_1p() + stirling_err(a + b) - stirling_err(a);
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
/// cancellation in `1 - (a + b) x / (a + 1)` and `1 + d_{2m+1}` is eliminated analytically.
fn ln_beta_continued_fraction(x: f64, y: f64, a: f64, b: f64) -> Result<f64, MathError> {
    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let tiny = 1e-300_f64;

    let mut c = 1.0_f64;
    // Exact identity: 1 - (a + b) * x / (a + 1) = ((a + b) * y + (1 - b)) / (a + 1).
    let mut d = if x > 0.5 { (qab * y + (1.0 - b)) / qap } else { 1.0 - qab * x / qap };
    if d.abs() < tiny {
        d = tiny;
    }
    d = 1.0 / d;
    let mut h = d;

    for m in 1..=MAX_BETA_CF_ITERS {
        let mf = m as f64;
        let m2 = 2.0 * mf;

        // Even step: d_{2m} = m (b - m) x / ((a + 2m - 1)(a + 2m)).
        let aa_even = mf * (b - mf) * x / ((qam + m2) * (a + m2));
        let d_prev = d;
        let c_prev = c;
        let denom_even = 1.0 + aa_even * d_prev;
        d = if denom_even.abs() < tiny { 1.0 / tiny } else { 1.0 / denom_even };
        let c_even = 1.0 + aa_even / c_prev;
        c = if c_even.abs() < tiny { tiny } else { c_even };
        h *= d * c;

        // Odd step: d_{2m+1} = -(a + m)(a + b + m) x / ((a + 2m)(a + 2m + 1)).
        let odd_denom = (a + m2) * (qap + m2);
        let aa_odd = -(a + mf) * (qab + mf) * x / odd_denom;
        if x > 0.5 {
            // Exact cancellation-free 1 + d_{2m+1} when x = 1 - y is close to 1:
            let one_plus_aa_odd = ((m2 + 1.0 - b) * a + mf * (3.0 * mf + 2.0 - b) + (a + mf) * (qab + mf) * y) / odd_denom;
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

    if xn * (a + b + 2.0) <= a + 1.0 {
        let ln_val = ln_beta_kernel(xn, yn, a, b)? - a.ln() + ln_beta_continued_fraction(xn, yn, a, b)?;
        Ok(ln_val.exp().clamp(0.0, 1.0))
    } else {
        let ln_comp = ln_beta_kernel(yn, xn, b, a)? - b.ln() + ln_beta_continued_fraction(yn, xn, b, a)?;
        Ok((1.0 - ln_comp.exp()).clamp(0.0, 1.0))
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

    if xn * (a + b + 2.0) <= a + 1.0 {
        Ok((ln_beta_kernel(xn, yn, a, b)? - a.ln() + ln_beta_continued_fraction(xn, yn, a, b)?).min(0.0))
    } else {
        let ln_comp = ln_beta_kernel(yn, xn, b, a)? - b.ln() + ln_beta_continued_fraction(yn, xn, b, a)?;
        Ok((-ln_comp.exp()).ln_1p())
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
    // Always solve for the smaller-probability variable via symmetry I_x(a, b) = p <=> I_y(b, a) = q.
    if q < p {
        let (y, x) = solve_lower_beta_quantile(b, a, q)?;
        Ok((x, y))
    } else {
        solve_lower_beta_quantile(a, b, p)
    }
}

fn initial_beta_lower_guess(a: f64, b: f64, p: f64) -> Result<f64, MathError> {
    let lb = ln_beta(a, b)?;
    if a >= 1.0 && b >= 1.0 {
        // Carter (1947) / Cran-Martin-Thomas normal-F approximation.
        let y = norm_isf_raw(p)?; // since p <= 0.5, y >= 0, and we want -y below mean.
        let z = -y;
        let lam = (z * z - 3.0) / 6.0;
        let h = 2.0 / (1.0 / (2.0 * a - 1.0) + 1.0 / (2.0 * b - 1.0));
        let w = (z * (h + lam).sqrt() / h) - (1.0 / (2.0 * b - 1.0) - 1.0 / (2.0 * a - 1.0)) * (lam + 5.0 / 6.0 - 2.0 / (3.0 * h));
        let exp_2w = (2.0 * w).clamp(-600.0, 600.0).exp();
        let guess = a / (a + b * exp_2w);
        if guess > 0.0 && guess < 1.0 {
            return Ok(guess);
        }
    }
    // Power-law asymptotic tail: I_x(a, b) ~ x^a / (a B(a, b)) as x -> 0.
    let ln_x0 = ((p * a).ln() + lb) / a;
    if ln_x0 < -0.2 {
        let x0 = ln_x0.exp();
        // First-order correction for (1 - x)^{b - 1}:
        let corr = (1.0 + ((b - 1.0) * a / (a + 1.0)) * x0 / a).max(0.5);
        return Ok((x0 * corr).clamp(f64::MIN_POSITIVE, 0.9));
    }
    let t1 = (a * p).powf(1.0 / a);
    let t2 = (b * (1.0 - p)).powf(1.0 / b);
    Ok((t1 / (t1 + t2)).clamp(f64::MIN_POSITIVE, 1.0 - f64::EPSILON))
}

fn solve_lower_beta_quantile(a: f64, b: f64, p: f64) -> Result<(f64, f64), MathError> {
    if p < 1e-8 {
        return solve_deep_lower_beta_quantile(a, b, p);
    }
    let mut x = initial_beta_lower_guess(a, b, p)?;
    let mut lo = 0.0_f64;
    let mut hi = 1.0_f64;

    for _ in 0..40 {
        let y = 1.0 - x;
        let val = reg_beta_xy(x, y, a, b)?;
        let fx = val - p;
        if fx == 0.0 {
            return Ok((x, 1.0 - x));
        }
        if fx < 0.0 {
            lo = lo.max(x);
        } else {
            hi = hi.min(x);
        }
        if (hi - lo) <= 2.0 * f64::EPSILON * x.max(f64::MIN_POSITIVE) {
            let xm = 0.5 * (lo + hi);
            return Ok((xm, 1.0 - xm));
        }
        let ln_pdf = ln_beta_kernel(x, y, a, b)? - x.ln() - y.ln();
        let pdf = ln_pdf.exp();
        let d2_over_d1 = (a - 1.0) / x - (b - 1.0) / y;
        let next = safeguarded_halley_step(x, fx, pdf, d2_over_d1, lo, hi);
        if (next - x).abs() <= 1.5 * f64::EPSILON * next.max(f64::MIN_POSITIVE) {
            return Ok((next, 1.0 - next));
        }
        x = next;
    }
    let xm = 0.5 * (lo + hi);
    Ok((xm, 1.0 - xm))
}

/// Deep lower-tail solver parameterized in `u = ln(x) < 0` so `h(u) = ln I_{e^u}(a, b) - ln p` is
/// nearly linear (`h'(u) ~ a`), completely free of overflow/underflow down to `x = 1e-308`.
fn solve_deep_lower_beta_quantile(a: f64, b: f64, p: f64) -> Result<(f64, f64), MathError> {
    let target_ln = p.ln();
    let lb = ln_beta(a, b)?;
    let u_asymp = (target_ln + a.ln() + lb) / a;
    let x_init = initial_beta_lower_guess(a, b, p)?;
    let mut u = if u_asymp < -0.5 { u_asymp } else { x_init.ln() }.clamp(-744.0, -f64::EPSILON);

    let mut u_lo = (u - 8.0).max(-744.0);
    let mut u_hi = (u + 8.0).min(-f64::EPSILON);
    while u_lo > -744.0 {
        let xl = u_lo.exp();
        if ln_reg_beta_xy(xl, 1.0 - xl, a, b)? < target_ln {
            break;
        }
        u_hi = u_lo;
        u_lo = (u_lo - 16.0).max(-744.0);
    }
    while u_hi < -f64::EPSILON {
        let xh = u_hi.exp();
        if ln_reg_beta_xy(xh, 1.0 - xh, a, b)? > target_ln {
            break;
        }
        u_lo = u_hi;
        u_hi = (0.5 * u_hi).min(-f64::EPSILON);
        if (u_hi - u_lo).abs() <= f64::EPSILON {
            break;
        }
    }

    for _ in 0..48 {
        let x = u.exp();
        if x == 0.0 {
            return Ok((0.0, 1.0));
        }
        let y = (1.0 - x).max(f64::MIN_POSITIVE);
        let ln_i = ln_reg_beta_xy(x, y, a, b)?;
        let h = ln_i - target_ln;
        if h == 0.0 {
            return Ok((x, 1.0 - x));
        }
        if h < 0.0 {
            u_lo = u_lo.max(u);
        } else {
            u_hi = u_hi.min(u);
        }
        if (u_hi - u_lo) <= 2.0 * f64::EPSILON * u.abs().max(1.0) {
            let xm = (0.5 * (u_lo + u_hi)).exp();
            return Ok((xm, 1.0 - xm));
        }
        let h1 = (ln_beta_kernel(x, y, a, b)? - y.ln() - ln_i).exp();
        let d2_over_d1 = a - (b - 1.0) * (x / y) - h1;
        let next_u = safeguarded_halley_step(u, h, h1, d2_over_d1, u_lo, u_hi);
        if (next_u - u).abs() <= 1.5 * f64::EPSILON * next_u.abs().max(1.0) {
            let xm = next_u.exp();
            return Ok((xm, 1.0 - xm));
        }
        u = next_u;
    }
    let xm = (0.5 * (u_lo + u_hi)).exp();
    Ok((xm, 1.0 - xm))
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
