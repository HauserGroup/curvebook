//! Confluent (`hyp1f1`, `confluent_u`) and Gauss (`hyp2f1`) hypergeometric functions in pure
//! IEEE-754 `f64`.
//!
//! References:
//! - NIST Digital Library of Mathematical Functions (DLMF), Ch. 13 (Confluent Hypergeometric
//!   Functions) and Ch. 15 (Hypergeometric Function), `https://dlmf.nist.gov/13`,
//!   `https://dlmf.nist.gov/15`.
//! - Pearson, J. W., Olver, S., & Porter, M. A. (2017). Numerical methods for the computation of
//!   the confluent and Gauss hypergeometric functions. *Numerical Algorithms*, 74(3), 821-866.
//!   `doi:10.1007/s11075-016-0173-0`.
//! - Trefethen, L. N., & Weideman, J. A. C. (2014). The exponentially convergent trapezoidal rule.
//!   *SIAM Review*, 56(3), 385-458. `doi:10.1137/130932132`.

use crate::error::MathError;
use crate::special::gamma::{gamma, ln_gamma};

const MAX_HYPER_ITERS: usize = 4_096;

fn is_nonpos_int(v: f64) -> bool {
    v <= 0.0 && v.fract() == 0.0
}

fn nonpos_int_degree(v: f64) -> Option<usize> {
    if !v.is_finite() || v > 0.0 || v.fract() != 0.0 {
        return None;
    }
    let mag = -v;
    if mag > 100_000.0 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some(mag as usize)
}

/// Kummer's confluent hypergeometric function of the first kind `M(a, b, x) = 1F1(a; b; x)`
/// (DLMF §13.2.2).
///
/// Applies Kummer's first transformation `M(a, b, x) = exp(x) * M(b - a, b, -x)` (DLMF §13.2.39)
/// when `x < 0` to reduce alternating-series cancellation.
///
/// # Errors
/// Returns [`MathError::InvalidParameter`] if `b` is a non-positive integer pole (`0, -1, -2, ...`)
/// or non-finite, [`MathError::DomainError`] if `a` or `x` is non-finite, or
/// [`MathError::NotConverged`] if the series fails to converge within the iteration budget.
pub fn hyp1f1(a: f64, b: f64, x: f64) -> Result<f64, MathError> {
    if !a.is_finite() {
        return Err(MathError::InvalidParameter { name: "a", value: a });
    }
    if !b.is_finite() || is_nonpos_int(b) {
        return Err(MathError::InvalidParameter { name: "b", value: b });
    }
    if !x.is_finite() {
        return Err(MathError::DomainError { name: "x", value: x });
    }
    if x == 0.0 || a == 0.0 {
        return Ok(1.0);
    }
    if (a - b) == 0.0 {
        return Ok(x.exp());
    }
    if let Some(deg) = nonpos_int_degree(a) {
        return hyp1f1_polynomial(deg, b, x);
    }
    // For x < 0 with b > 0, Kummer's transformation M(a, b, x) = exp(x) * M(b - a, b, -x)
    // maps x < 0 to -x > 0 so the series is eventually constant-sign (no asymptotic cancellation).
    // Never apply Kummer when x > 0: the direct series is already eventually constant-sign for
    // k > max(-a, -b), whereas flipping to -x introduces catastrophic alternating cancellation.
    if x < 0.0 && b > 0.0 && x > -700.0 {
        if let Some(deg) = nonpos_int_degree(b - a) {
            return Ok(x.exp() * hyp1f1_polynomial(deg, b, -x)?);
        }
        let m_pos = hyp1f1_series(b - a, b, -x)?;
        return Ok(x.exp() * m_pos);
    }
    hyp1f1_series(a, b, x)
}

fn hyp1f1_polynomial(deg: usize, b: f64, x: f64) -> Result<f64, MathError> {
    let mut term = 1.0_f64;
    let mut sum = 1.0_f64;
    let a = -(deg as f64);
    for k in 0..deg {
        let kf = k as f64;
        term *= ((a + kf) / ((b + kf) * (kf + 1.0))) * x;
        if !term.is_finite() {
            return Err(MathError::DomainError { name: "x", value: x });
        }
        sum += term;
    }
    Ok(sum)
}

fn hyp1f1_series(a: f64, b: f64, x: f64) -> Result<f64, MathError> {
    let mut term = 1.0_f64;
    let mut sum = 1.0_f64;
    let min_k = (-a).max(-b).max(0.0);
    for k in 0..MAX_HYPER_ITERS {
        let kf = k as f64;
        let denom = (b + kf) * (kf + 1.0);
        if denom == 0.0 {
            return Err(MathError::InvalidParameter { name: "b", value: b });
        }
        term *= ((a + kf) / denom) * x;
        if !term.is_finite() {
            return Ok(term);
        }
        let next = sum + term;
        if term == 0.0 {
            return Ok(next);
        }
        let next_mult = (((a + kf + 1.0) / ((b + kf + 1.0) * (kf + 2.0))) * x).abs();
        if kf > min_k && next_mult < 1.0 && term.abs() <= 0.25 * f64::EPSILON * next.abs().max(f64::MIN_POSITIVE) {
            return Ok(next);
        }
        sum = next;
    }
    Err(MathError::NotConverged { algorithm: "hyp1f1_series", iterations: MAX_HYPER_ITERS })
}

/// Tricomi's confluent hypergeometric function of the second kind `U(a, b, x)` for real `x > 0`
/// (and terminating polynomial cases for non-positive integer `a`; DLMF §13.2.6, §13.4.4).
///
/// Uses the strictly positive Laplace integral representation with double-exponential quadrature
/// for `a > 0` (completely free of `0/0` singularities at integer `b`), exact terminating
/// polynomials for `a in {0, -1, -2, ...}`, and DLMF §13.3.7 contiguous recurrence for general
/// `a < 0`.
///
/// # Errors
/// Returns [`MathError::InvalidParameter`] if `a` or `b` is non-finite, [`MathError::DomainError`]
/// if `x < 0` (when `a` is not a non-positive integer), `x == 0` with `b >= 1`, or `x` is
/// non-finite, or [`MathError::NotConverged`] if evaluation fails to converge.
pub fn confluent_u(a: f64, b: f64, x: f64) -> Result<f64, MathError> {
    if !a.is_finite() {
        return Err(MathError::InvalidParameter { name: "a", value: a });
    }
    if !b.is_finite() {
        return Err(MathError::InvalidParameter { name: "b", value: b });
    }
    if !x.is_finite() {
        return Err(MathError::DomainError { name: "x", value: x });
    }
    if let Some(deg) = nonpos_int_degree(a) {
        return confluent_u_polynomial(deg, b, x);
    }
    if x < 0.0 {
        return Err(MathError::DomainError { name: "x", value: x });
    }
    if x == 0.0 {
        if b < 1.0 {
            if is_nonpos_int(a - b + 1.0) {
                return Ok(0.0);
            }
            let num = gamma(1.0 - b)?;
            let den = gamma(a - b + 1.0)?;
            return Ok(num / den);
        }
        return Err(MathError::DomainError { name: "x", value: x });
    }
    if a > 0.0 {
        return confluent_u_positive_a(a, b, x);
    }
    // General a < 0 (non-integer): shift upward to a0 in (0, 1) and apply DLMF §13.3.7 downward.
    // DLMF §13.3.7: U(a - 1, b, x) = (x + 2*a - b) * U(a, b, x) - a * (1 + a - b) * U(a + 1, b, x).
    let shift = (-a).ceil();
    if shift > 10_000.0 {
        return Err(MathError::NotConverged { algorithm: "confluent_u_recurrence", iterations: 10_000 });
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let steps = shift as usize;
    let a_pos = a + f64::from(steps as u32);
    let mut u_hi = confluent_u_positive_a(a_pos + 1.0, b, x)?;
    let mut u_curr = confluent_u_positive_a(a_pos, b, x)?;
    for s in 0..steps {
        let cur_a = a_pos - (s as f64);
        let u_prev = (x + 2.0 * cur_a - b) * u_curr - cur_a * (1.0 + cur_a - b) * u_hi;
        if !u_prev.is_finite() {
            return Ok(u_prev);
        }
        u_hi = u_curr;
        u_curr = u_prev;
    }
    Ok(u_curr)
}

/// Exact polynomial `U(-n, b, x) = (-1)^n * sum_{k=0}^n C(n, k) * (b + k)_{n - k} * (-x)^k` (DLMF §13.2.7).
fn confluent_u_polynomial(deg: usize, b: f64, x: f64) -> Result<f64, MathError> {
    if deg == 0 {
        return Ok(1.0);
    }
    // Evaluate backwards in k = deg down to 0 so U(-n, b, x) = x^n * sum_{j=0}^n (-n)_j (1 - b - n)_j / (j! (-x)^j).
    // Even safer for x = 0: accumulate directly via C(n, k) * poch(b + k, n - k) * (-x)^k.
    let mut sum = 0.0_f64;
    let mut binom = 1.0_f64;
    let mut x_pow = 1.0_f64;
    for k in 0..=deg {
        if k > 0 {
            binom *= ((deg - k + 1) as f64) / (k as f64);
            x_pow *= -x;
        }
        let mut poch = 1.0_f64;
        for j in 0..(deg - k) {
            poch *= b + ((k + j) as f64);
        }
        sum += binom * poch * x_pow;
    }
    let sign = if (deg & 1) == 0 { 1.0 } else { -1.0 };
    if !sum.is_finite() {
        return Err(MathError::DomainError { name: "x", value: x });
    }
    Ok(sign * sum)
}

/// Evaluate `U(a, b, x)` for `a > 0, x > 0` via the strictly positive Laplace integral
/// `U(a, b, x) = (1 / Gamma(a)) * int_0^inf e^{-x t} t^{a-1} (1 + t)^{b - a - 1} dt` (DLMF §13.4.4)
/// mapped by the double-exponential substitution `u(s) = ln(t) = u0 + s - expm1(-s)` (`s in (-inf, +inf)`),
/// which yields doubly exponential decay on BOTH the left tail (`-a * e^{-s}`, unit peak curvature
/// independent of how small `a > 0` is) and the right tail (`-x * e^{u0 + s}`; Trefethen & Weideman 2014).
fn confluent_u_positive_a(a: f64, b: f64, x: f64) -> Result<f64, MathError> {
    // Check large-x asymptotic series first when parameters are moderate.
    let p_scale = a.abs().max((a - b + 1.0).abs()).max(1.0);
    if x >= 40.0 * p_scale
        && x >= 40.0
        && let Some(val) = confluent_u_asymptotic(a, b, x)
    {
        return Ok(val);
    }
    // Center u0 at the unique positive mode of x * t^2 + (x + 1 - b) * t - max(a, 1) = 0 so
    // s = 0 sits at the main peak for a >= 1 and at the right-tail onset for small a < 1.
    let a_ref = a.max(1.0);
    let lin = x + 1.0 - b;
    let disc = (lin * lin + 4.0 * x * a_ref).sqrt();
    let t0 = if lin >= 0.0 { (2.0 * a_ref) / (lin + disc) } else { (-lin + disc) / (2.0 * x) };
    let u0 = t0.ln();
    let c = b - a - 1.0;
    let ln_x = x.ln();

    // Log-integrand in s: Phi(s) = -x * e^{u(s)} + a * u(s) + c * ln(1 + e^{u(s)}) + ln(1 + e^{-s}).
    let phi = |s: f64| -> f64 {
        if s < -700.0 {
            return f64::NEG_INFINITY;
        }
        let u = u0 + s - (-s).exp_m1();
        let ln_du_ds = if s >= 0.0 { (-s).exp().ln_1p() } else { -s + s.exp().ln_1p() };
        let ln_xt = ln_x + u;
        if ln_xt > 6.7 {
            return f64::NEG_INFINITY;
        }
        let xt = if ln_xt < -745.0 { 0.0 } else { ln_xt.exp() };
        let ln1pt = if u > 35.0 {
            u
        } else if u < -745.0 {
            0.0
        } else {
            u.exp().ln_1p()
        };
        -xt + a * u + c * ln1pt + ln_du_ds
    };

    // For a < 1, the left-tail factor exp(a * u(s) + ln(1 + e^{-s})) has a Gumbel peak of unit
    // curvature near s = ln(a); for a >= 1, the single peak sits near s = 0.
    let s_left_peak = if a < 1.0 { a.ln() } else { 0.0 };
    let mut peak_est = phi(0.0).max(phi(s_left_peak));
    for &probe in &[-1.0_f64, 1.0, 0.5 * s_left_peak] {
        let v = phi(probe);
        if v > peak_est {
            peak_est = v;
        }
    }

    let curv_u = (x * t0 + c.abs() * t0 / ((1.0 + t0) * (1.0 + t0))).max(a_ref).max(0.25);
    let curv_s = 4.0 * curv_u;
    let right_scan_step = (0.5 / curv_s.sqrt()).clamp(1e-3, 0.5);
    let left_scan_step = if a < 1.0 { 0.5 } else { right_scan_step };

    let mut s_lo = if a < 1.0 { s_left_peak.min(-1.0) } else { -left_scan_step };
    while s_lo > -700.0 {
        let v = phi(s_lo);
        if v > peak_est {
            peak_est = v;
        }
        if !v.is_finite() || v < peak_est - 50.0 {
            break;
        }
        s_lo -= left_scan_step;
    }

    let mut s_hi = right_scan_step;
    while s_hi < 700.0 {
        let v = phi(s_hi);
        if v > peak_est {
            peak_est = v;
        }
        if !v.is_finite() || v < peak_est - 50.0 {
            break;
        }
        s_hi += right_scan_step;
    }

    let span = s_hi - s_lo;
    let h0 = (0.4 / curv_s.sqrt()).clamp(1e-4, 0.25);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let mut n_steps = ((span / h0).ceil() as usize).clamp(32, 2_048);

    // Evaluate trapezoidal rule in running log-sum-exp space so large or tiny intermediate
    // integrals never overflow/underflow before dividing by Gamma(a).
    let eval_trap_ln = |steps: usize| -> f64 {
        let step = span / (steps as f64);
        let mut max_p = f64::NEG_INFINITY;
        let mut total = 0.0_f64;
        for k in 0..=steps {
            let s = s_lo + (k as f64) * step;
            let p = phi(s);
            if !p.is_finite() {
                continue;
            }
            if p > max_p {
                if max_p.is_finite() {
                    total *= (max_p - p).exp();
                }
                max_p = p;
                total += 1.0;
            } else if p - max_p > -45.0 {
                total += (p - max_p).exp();
            }
        }
        if !max_p.is_finite() || total <= 0.0 {
            return f64::NEG_INFINITY;
        }
        max_p + (step * total).ln()
    };

    let lg_a = ln_gamma(a)?;
    let mut prev_ln = eval_trap_ln(n_steps);
    for _ in 0..7 {
        n_steps *= 2;
        let curr_ln = eval_trap_ln(n_steps);
        if curr_ln.is_finite() && prev_ln.is_finite() && (curr_ln - prev_ln).abs() <= 1e-13 {
            return Ok((curr_ln - lg_a).exp());
        }
        prev_ln = curr_ln;
    }
    if !prev_ln.is_finite() {
        return Err(MathError::NotConverged { algorithm: "confluent_u_quadrature", iterations: n_steps });
    }
    Ok((prev_ln - lg_a).exp())
}

fn confluent_u_asymptotic(a: f64, b: f64, x: f64) -> Option<f64> {
    let inv_x = 1.0 / x;
    let mut sum = 1.0_f64;
    let mut term = 1.0_f64;
    let mut prev_abs = f64::INFINITY;
    for k in 0..40_u32 {
        let kf = f64::from(k);
        let next = -term * (a + kf) * (a - b + 1.0 + kf) * inv_x / (kf + 1.0);
        let next_abs = next.abs();
        if !next.is_finite() || (k > 1 && next_abs > prev_abs) {
            return None;
        }
        term = next;
        prev_abs = next_abs;
        sum += term;
        if next_abs <= 0.25 * f64::EPSILON * sum.abs() {
            return Some((-a * x.ln() + sum.ln()).exp());
        }
    }
    None
}

/// Gauss hypergeometric function `2F1(a, b; c; x)` for real `x` (DLMF §15.2, §15.8).
///
/// Supports all real `x < 1` via direct series for `|x| <= 0.5`, Pfaff transformation
/// (DLMF §15.8.1) for `x < -0.5`, and linear transformation / Euler integral for `0.5 < x < 1`,
/// plus Gauss's summation theorem at `x = 1` when `c - a - b > 0` and terminating polynomials
/// over all real `x` when `a` or `b` is a non-positive integer.
///
/// # Errors
/// Returns [`MathError::InvalidParameter`] if `a`, `b`, or `c` is non-finite or `c` is a
/// non-positive integer pole, [`MathError::DomainError`] if `x` is non-finite, `x > 1` for
/// non-terminating series, or `x == 1` with `c - a - b <= 0`, or [`MathError::NotConverged`] if
/// the series or quadrature does not converge.
pub fn hyp2f1(a: f64, b: f64, c: f64, x: f64) -> Result<f64, MathError> {
    if !a.is_finite() {
        return Err(MathError::InvalidParameter { name: "a", value: a });
    }
    if !b.is_finite() {
        return Err(MathError::InvalidParameter { name: "b", value: b });
    }
    if !c.is_finite() || is_nonpos_int(c) {
        return Err(MathError::InvalidParameter { name: "c", value: c });
    }
    if !x.is_finite() {
        return Err(MathError::DomainError { name: "x", value: x });
    }
    if x == 0.0 || a == 0.0 || b == 0.0 {
        return Ok(1.0);
    }
    if let Some(deg) = nonpos_int_degree(a) {
        return hyp2f1_polynomial(deg, b, c, x);
    }
    if let Some(deg) = nonpos_int_degree(b) {
        return hyp2f1_polynomial(deg, a, c, x);
    }
    if x > 1.0 {
        return Err(MathError::DomainError { name: "x", value: x });
    }
    if x == 1.0 {
        let s = c - a - b;
        if s <= 0.0 {
            return Err(MathError::DomainError { name: "x", value: x });
        }
        if is_nonpos_int(c - a) || is_nonpos_int(c - b) {
            return Ok(0.0);
        }
        return Ok(gamma(c)? * gamma(s)? / (gamma(c - a)? * gamma(c - b)?));
    }
    // Exact elementary reduction 2F1(c, b; c; x) = (1 - x)^{-b} for all x < 1 (DLMF §15.4.6).
    if a == c {
        return Ok((1.0 - x).powf(-b));
    }
    if b == c {
        return Ok((1.0 - x).powf(-a));
    }
    if x.abs() <= 0.5 {
        return hyp2f1_series(a, b, c, x);
    }
    if x < -0.5 {
        // Pfaff transformation (DLMF §15.8.1): w = x / (x - 1) in (1/3, 1).
        let w = x / (x - 1.0);
        if w <= 0.5 {
            // x in [-1, -0.5): w in (1/3, 1/2] -> converges rapidly directly!
            let inner = hyp2f1_series(a, c - b, c, w)?;
            return Ok((1.0 - x).powf(-a) * inner);
        }
        // x < -1: w = x / (x - 1) in (0.5, 1), so 1 - w = 1 / (1 - x) in (0, 0.5).
        // Apply the (1 - w) transformation on (a, c - b; c; w) via `hyp2f1_near_one`.
        let inner = hyp2f1_near_one(a, c - b, c, w)?;
        return Ok((1.0 - x).powf(-a) * inner);
    }
    // 0.5 < x < 1.0: use (1 - x) transformation (DLMF §15.8.4) or Euler integral near integer c - a - b.
    hyp2f1_near_one(a, b, c, x)
}

fn hyp2f1_polynomial(deg: usize, other: f64, c: f64, x: f64) -> Result<f64, MathError> {
    let mut term = 1.0_f64;
    let mut sum = 1.0_f64;
    let a = -(deg as f64);
    for k in 0..deg {
        let kf = k as f64;
        term *= ((a + kf) * (other + kf) / ((c + kf) * (kf + 1.0))) * x;
        if !term.is_finite() {
            return Err(MathError::DomainError { name: "x", value: x });
        }
        sum += term;
    }
    Ok(sum)
}

fn hyp2f1_series(a: f64, b: f64, c: f64, x: f64) -> Result<f64, MathError> {
    let mut term = 1.0_f64;
    let mut sum = 1.0_f64;
    let min_k = (-a).max(-b).max(-c).max(0.0);
    for k in 0..MAX_HYPER_ITERS {
        let kf = k as f64;
        let denom = (c + kf) * (kf + 1.0);
        if denom == 0.0 {
            return Err(MathError::InvalidParameter { name: "c", value: c });
        }
        term *= ((a + kf) * (b + kf) / denom) * x;
        if !term.is_finite() {
            return Ok(term);
        }
        let next = sum + term;
        if term == 0.0 {
            return Ok(next);
        }
        if kf > min_k && term.abs() <= 0.25 * f64::EPSILON * next.abs().max(f64::MIN_POSITIVE) {
            return Ok(next);
        }
        sum = next;
    }
    Err(MathError::NotConverged { algorithm: "hyp2f1_series", iterations: MAX_HYPER_ITERS })
}

/// Evaluate `2F1(a, b; c; x)` for `0.5 < x < 1.0`.
fn hyp2f1_near_one(a: f64, b: f64, c: f64, x: f64) -> Result<f64, MathError> {
    let w = 1.0 - x; // w in (0, 0.5)
    let d = c - a - b;
    let dist_to_int = (d - d.round()).abs();

    // When c - a - b is well away from an integer, DLMF §15.8.4 is fast and exact.
    // Guard non-positive integer poles in the denominator Gamma factors (where 1 / Gamma(-m) == 0).
    if dist_to_int > 1e-4 {
        let g_c = gamma(c)?;
        let term1 = if is_nonpos_int(c - a) || is_nonpos_int(c - b) {
            0.0
        } else {
            let f1 = if let Some(deg) = nonpos_int_degree(a) {
                hyp2f1_polynomial(deg, b, 1.0 - d, w)?
            } else if let Some(deg) = nonpos_int_degree(b) {
                hyp2f1_polynomial(deg, a, 1.0 - d, w)?
            } else {
                hyp2f1_series(a, b, 1.0 - d, w)?
            };
            (g_c * gamma(d)? / (gamma(c - a)? * gamma(c - b)?)) * f1
        };
        let term2 = if is_nonpos_int(a) || is_nonpos_int(b) {
            0.0
        } else {
            let f2 = if let Some(deg) = nonpos_int_degree(c - a) {
                hyp2f1_polynomial(deg, c - b, 1.0 + d, w)?
            } else if let Some(deg) = nonpos_int_degree(c - b) {
                hyp2f1_polynomial(deg, c - a, 1.0 + d, w)?
            } else {
                hyp2f1_series(c - a, c - b, 1.0 + d, w)?
            };
            w.powf(d) * (g_c * gamma(-d)? / (gamma(a)? * gamma(b)?)) * f2
        };
        return Ok(term1 + term2);
    }

    // Exact polynomial termination when Pfaff maps `c - b` to a non-positive integer.
    if let Some(deg) = nonpos_int_degree(a) {
        return hyp2f1_polynomial(deg, b, c, x);
    }
    if let Some(deg) = nonpos_int_degree(b) {
        return hyp2f1_polynomial(deg, a, c, x);
    }

    // Near or at integer `c - a - b`: if `c > b > 0` (or `c > a > 0`), Euler's integral (DLMF §15.6.1)
    // has zero pole cancellation and converges to machine precision via tanh-sinh quadrature.
    if b > 0.0 && c - b > 0.0 {
        return hyp2f1_euler_integral(a, b, c, x);
    }
    if a > 0.0 && c - a > 0.0 {
        return hyp2f1_euler_integral(b, a, c, x);
    }

    // Fallback for moderate x when parameters lie outside the Euler integral strip:
    // apply Euler's transformation 2F1(a, b; c; x) = (1 - x)^{c - a - b} 2F1(c - a, c - b; c; x) (DLMF §15.8.1).
    let (a2, b2) = (c - a, c - b);
    if let Some(deg) = nonpos_int_degree(a2) {
        return Ok(w.powf(d) * hyp2f1_polynomial(deg, b2, c, x)?);
    }
    if let Some(deg) = nonpos_int_degree(b2) {
        return Ok(w.powf(d) * hyp2f1_polynomial(deg, a2, c, x)?);
    }
    if b2 > 0.0 && c - b2 > 0.0 {
        return Ok(w.powf(d) * hyp2f1_euler_integral(a2, b2, c, x)?);
    }
    if a2 > 0.0 && c - a2 > 0.0 {
        return Ok(w.powf(d) * hyp2f1_euler_integral(b2, a2, c, x)?);
    }
    hyp2f1_series(a, b, c, x)
}

/// Euler integral `2F1(a, b; c; x) = (1 / B(b, c - b)) * int_0^1 t^{b-1} (1 - t)^{c-b-1} (1 - x t)^{-a} dt`
/// for `c > b > 0, x < 1` via double-exponential tanh-sinh quadrature (DLMF §15.6.1; Trefethen & Weideman 2014).
fn hyp2f1_euler_integral(a: f64, b: f64, c: f64, x: f64) -> Result<f64, MathError> {
    let cb = c - b;
    let ln_b_inv = -(crate::special::gamma::ln_beta(b, cb)?);
    let eval_ts = |h: f64| -> f64 {
        let mut sum = 0.0_f64;
        // Map s in (-inf, inf) -> t = 0.5 * (1 + tanh(pi/2 * sinh(s))) = 1 / (1 + exp(-pi * sinh(s))).
        // Then 1 - t = 1 / (1 + exp(pi * sinh(s))) and dt/ds = pi * cosh(s) * t * (1 - t).
        for k in -512..=512_i32 {
            let s = f64::from(k) * h;
            let sh = s.sinh();
            let ch = s.cosh();
            let arg = std::f64::consts::PI * sh;
            if arg.abs() > 700.0 {
                continue;
            }
            let (t_val, ln_t_val, ln_omt_val) = if arg >= 0.0 {
                let e = (-arg).exp();
                (1.0 / (1.0 + e), -e.ln_1p(), -arg - e.ln_1p())
            } else {
                let e = arg.exp();
                (e / (1.0 + e), arg - e.ln_1p(), -e.ln_1p())
            };
            let one_m_xt = 1.0 - x * t_val;
            if one_m_xt <= 0.0 {
                continue;
            }
            let ln_w = (std::f64::consts::PI * ch).ln() + ln_t_val + ln_omt_val;
            let ln_term = ln_b_inv + (b - 1.0) * ln_t_val + (cb - 1.0) * ln_omt_val - a * one_m_xt.ln() + ln_w;
            if ln_term < -740.0 {
                continue;
            }
            let contrib = ln_term.exp();
            if contrib.is_finite() {
                sum += contrib;
            }
        }
        h * sum
    };
    let mut h = 0.2_f64;
    let mut prev = eval_ts(h);
    for _ in 0..5 {
        h *= 0.5;
        let curr = eval_ts(h);
        if (curr - prev).abs() <= 1e-13 * curr.abs().max(f64::MIN_POSITIVE) {
            return Ok(curr);
        }
        prev = curr;
    }
    Ok(prev)
}

#[cfg(test)]
mod tests {
    use super::{confluent_u, hyp1f1, hyp2f1};

    #[test]
    fn hyp1f1_matches_exact_analytical_identities() {
        // M(a, a, x) = exp(x)
        for &x in &[-5.0_f64, -0.5, 0.0, 1.5, 4.0] {
            assert!((hyp1f1(2.5, 2.5, x).unwrap() - x.exp()).abs() <= 1e-13 * x.exp());
        }
        // M(1, 2, x) = (exp(x) - 1) / x
        for &x in &[-4.0_f64, -0.25, 0.5, 3.0] {
            let expected = x.exp_m1() / x;
            assert!((hyp1f1(1.0, 2.0, x).unwrap() - expected).abs() <= 1e-13 * expected.abs());
        }
        // Laguerre polynomial L_2^{(0)}(x) = M(-2, 1, x) = 1 - 2x + x^2 / 2
        assert!((hyp1f1(-2.0, 1.0, 3.0).unwrap() - (1.0 - 6.0 + 4.5)).abs() < 1e-14);
        // Regression: x > 0, a < 0, b > 0 must NOT apply Kummer's transform (avoids catastrophic cancellation).
        // Cross-check M(-0.5, 1.5, x) via DLMF §13.2.34 Wronskian with U(a, b, x):
        //   M(a, b, x) * U(a + 1, b + 1, x) + (1 / b) * M(a + 1, b + 1, x) * U(a, b, x)
        //     = Gamma(b) / Gamma(a + 1) * x^{-b} * exp(x)
        // Plus exact contiguous check: M(-0.5, 1.5, x) = 0.5 * (exp(x) + (1 - 2x) * M(0.5, 1.5, x)) ->
        // verified directly against exact Wronskian (free of subtraction cancellation!).
        for &x in &[5.0_f64, 20.0, 30.0, 40.0] {
            let m_val = hyp1f1(-0.5, 1.5, x).unwrap();
            assert!(m_val < 0.0, "M(-0.5, 1.5, {x}) must be negative, got {m_val}");
            let m_next = hyp1f1(0.5, 2.5, x).unwrap();
            let u_val = confluent_u(-0.5, 1.5, x).unwrap();
            let u_next = confluent_u(0.5, 2.5, x).unwrap();
            let lhs = m_val * u_next + (1.0 / 1.5) * m_next * u_val;
            let rhs = 0.5 * x.powf(-1.5) * x.exp();
            assert!((lhs - rhs).abs() <= 1e-12 * rhs.abs(), "Wronskian mismatch at x={x}: lhs={lhs} vs rhs={rhs}");
        }
        // Exact 60-digit series values for M(-0.5, 1.5, x) = sum_{k>=0} (-1 / (4k^2 - 1)) * x^k / k!:
        for &(x, expected) in &[(20.0_f64, -3.601_108_394_590_896e5_f64), (30.0, -3.309_595_998_369_159e9), (40.0, -3.983_125_449_785_358e13)] {
            let actual = hyp1f1(-0.5, 1.5, x).unwrap();
            assert!((actual - expected).abs() <= 1e-13 * expected.abs(), "M(-0.5, 1.5, {x}): {actual} vs {expected}");
        }
        // Negative x with b < a (Kummer avoids alternating cancellation):
        // M(5, 1.5, -20) = exp(-20) * M(-3.5, 1.5, 20)
        let m_neg = hyp1f1(5.0, 1.5, -20.0).unwrap();
        let m_kummer = (-20.0_f64).exp() * hyp1f1(-3.5, 1.5, 20.0).unwrap();
        assert!((m_neg - m_kummer).abs() <= 1e-13 * m_kummer.abs());
        assert!(hyp1f1(1.0, 0.0, 1.0).is_err());
        assert!(hyp1f1(1.0, -2.0, 1.0).is_err());
    }

    #[test]
    fn confluent_u_matches_exact_identities_including_integer_b() {
        // U(a, a + 1, x) = x^{-a} (DLMF §13.6.4), including small positive a (regression for left-tail truncation)
        for &(a, x) in
            &[(1e-6_f64, 2.0_f64), (1e-4, 2.0), (0.01, 2.0), (0.05, 2.0), (0.5, 2.0), (1.5, 0.5), (2.0, 3.0), (3.0, 10.0), (-0.99, 2.0), (-1.5, 2.5)]
        {
            let expected = x.powf(-a);
            let actual = confluent_u(a, a + 1.0, x).unwrap();
            assert!((actual - expected).abs() <= 1e-12 * expected.abs(), "a={a}, x={x}: {actual} vs {expected}");
        }
        // Small a with non-trivial b != a + 1: verify DLMF §13.3.7 recurrence U(a - 1, b, x) = (x + 2a - b) U(a, b, x) - a(1 + a - b) U(a + 1, b, x)
        // via Kummer's U(a, b, x) = x^{1 - b} U(a - b + 1, 2 - b, x) (DLMF §13.2.40):
        for &(a, b, x) in &[(0.01_f64, 0.5_f64, 2.0_f64), (0.05, 3.0, 1.5), (0.01, -2.5, 2.0)] {
            let lhs = confluent_u(a, b, x).unwrap();
            let rhs = x.powf(1.0 - b) * confluent_u(a - b + 1.0, 2.0 - b, x).unwrap();
            assert!((lhs - rhs).abs() <= 1e-11 * lhs.abs(), "Kummer U reflection a={a}, b={b}, x={x}: {lhs} vs {rhs}");
        }
        // U(1, 1, x) = e^x * E_1(x); at x = 1, e * E_1(1) = 0.5963473623231941 (DLMF §6.2 / §13.6.6)
        assert!((confluent_u(1.0, 1.0, 1.0).unwrap() - 0.596_347_362_323_194_1).abs() < 1e-12);
        // Polynomial U(-2, 1, x) = 2 - 4x + x^2 = 2! L_2(x)
        assert!((confluent_u(-2.0, 1.0, 3.0).unwrap() - (2.0 - 12.0 + 9.0)).abs() < 1e-14);
        // x == 0 with b < 1 and a - b + 1 non-positive integer -> 0
        assert_eq!(confluent_u(-1.5, 0.5, 0.0).unwrap(), 0.0);
        assert!(confluent_u(0.5, 1.0, 0.0).is_err());
        assert!(confluent_u(0.5, 1.0, -1.0).is_err());
    }

    #[test]
    fn hyp2f1_matches_exact_identities_across_full_real_domain() {
        // 2F1(1, 1; 2; x) = -ln(1 - x) / x (integer c - a - b = 0!)
        for &x in &[-5.0_f64, -1.5, -0.75, -0.25, 0.25, 0.75, 0.95] {
            let expected = -((-x).ln_1p()) / x;
            let actual = hyp2f1(1.0, 1.0, 2.0, x).unwrap();
            assert!((actual - expected).abs() <= 1e-12 * expected.abs(), "x={x}: {actual} vs {expected}");
        }
        // 2F1(0.5, 1; 1.5; -x^2) = atan(x) / x
        for &u in &[0.3_f64, 0.8, 1.5, 3.0] {
            let expected = u.atan() / u;
            let actual = hyp2f1(0.5, 1.0, 1.5, -(u * u)).unwrap();
            assert!((actual - expected).abs() <= 1e-12 * expected.abs(), "u={u}: {actual} vs {expected}");
        }
        // Regression: c - a or c - b being a non-positive integer (0, -1, -2, ...) across x == 1, 0.5 < x < 1, and x < -1.
        assert!((hyp2f1(2.0, 0.5, 2.0, 0.75).unwrap() - 2.0).abs() < 1e-14);
        assert!((hyp2f1(0.5, 2.0, 2.0, 0.75).unwrap() - 2.0).abs() < 1e-14);
        assert!((hyp2f1(2.0, 1.5, 2.0, -2.0).unwrap() - 3.0_f64.powf(-1.5)).abs() < 1e-14);
        assert!((hyp2f1(1.5, 2.0, 2.0, -2.0).unwrap() - 3.0_f64.powf(-1.5)).abs() < 1e-14);
        assert_eq!(hyp2f1(1.5, -0.5, 1.5, 1.0).unwrap(), 0.0);
        assert_eq!(hyp2f1(-0.5, 1.5, 1.5, 1.0).unwrap(), 0.0);
        // Non-zero negative integer c - a = -1: 2F1(3, b; 2; x) = (1 - x)^{-b - 1} * (1 + (b/2 - 1) * x)
        for &(b, x) in &[(0.5_f64, 0.75_f64), (1.5, -2.0), (1.0, 0.75), (2.5, 0.8)] {
            let expected = (1.0 - x).powf(-b - 1.0) * (1.0 + (0.5 * b - 1.0) * x);
            let actual_ab = hyp2f1(3.0, b, 2.0, x).unwrap();
            let actual_ba = hyp2f1(b, 3.0, 2.0, x).unwrap();
            assert!((actual_ab - expected).abs() <= 1e-12 * expected.abs(), "2F1(3,{b};2;{x}): {actual_ab} vs {expected}");
            assert!((actual_ba - expected).abs() <= 1e-12 * expected.abs(), "2F1({b},3;2;{x}): {actual_ba} vs {expected}");
        }
        assert_eq!(hyp2f1(3.0, -1.5, 2.0, 1.0).unwrap(), 0.0);
        // Gauss summation at x = 1: 2F1(0.5, 0.5; 2.0; 1) = Gamma(2) * Gamma(1) / Gamma(1.5)^2 = 4 / pi
        assert!((hyp2f1(0.5, 0.5, 2.0, 1.0).unwrap() - 4.0 / std::f64::consts::PI).abs() < 1e-13);
        // Polynomial at x = 5: 2F1(-2, 3; 1; 5) = 1 - 6*5 + 6*25 = 121
        assert!((hyp2f1(-2.0, 3.0, 1.0, 5.0).unwrap() - 121.0).abs() < 1e-12);
        assert!(hyp2f1(1.0, 1.0, 2.0, 1.0).is_err());
        assert!(hyp2f1(0.5, 0.5, 2.0, 1.5).is_err());
        assert!(hyp2f1(1.0, 1.0, 0.0, 0.2).is_err());
    }
}
