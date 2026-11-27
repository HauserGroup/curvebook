//! Integer-order Bessel functions (`J_n`, `Y_n`, `I_n`, `K_n`) in pure IEEE-754 `f64`.
//!
//! Combines power series for small arguments, Miller's downward recurrence with Neumann
//! normalization for `J_n` and `Y_0`/`Y_1` in the intermediate region, stable upward recurrence
//! for `Y_n` and `K_n`, Hankel asymptotic expansions for large arguments, and the exponentially
//! convergent trapezoidal rule on the hyperbolic integral representation of `K_0`/`K_1`.
//!
//! References:
//! - NIST Digital Library of Mathematical Functions (DLMF), Ch. 10 (Bessel Functions),
//!   `https://dlmf.nist.gov/10`.
//! - Abramowitz, M., & Stegun, I. A. (1964). *Handbook of Mathematical Functions* (Ch. 9).
//!   National Bureau of Standards.
//! - Press, W. H., Teukolsky, S. A., Vetterling, W. T., & Flannery, B. P. (2007).
//!   *Numerical Recipes: The Art of Scientific Computing* (3rd ed., §6.5). Cambridge University Press.
//! - Trefethen, L. N., & Weideman, J. A. C. (2014). The exponentially convergent trapezoidal rule.
//!   *SIAM Review*, 56(3), 385-458. `doi:10.1137/130932132`.

use std::f64::consts::{EULER_GAMMA, FRAC_2_PI, FRAC_PI_4, PI, TAU};

use crate::error::MathError;
use crate::special::gamma::ln_factorial;

const MAX_SERIES_ITERS: usize = 2_048;
const MAX_MILLER_ORDER: u32 = 200_000;
const RESCALE_LIMIT: f64 = 1e100;
const INV_RESCALE_LIMIT: f64 = 1e-100;
const LN_RESCALE_LIMIT: f64 = 230.258_509_299_404_58; // 100 * ln(10)

/// Bessel function of the first kind `J_n(x)` for integer order `n` and finite real `x`.
///
/// Uses symmetry `J_{-n}(x) = (-1)^n J_n(x)` and `J_n(-x) = (-1)^n J_n(x)` (DLMF §10.4.1, §10.11.1).
///
/// # Errors
/// Returns [`MathError::DomainError`] if `x` is `NaN` or infinite, or [`MathError::NotConverged`]
/// if an iterative evaluation exceeds its iteration budget.
pub fn bessel_j(n: i32, x: f64) -> Result<f64, MathError> {
    if !x.is_finite() {
        return Err(MathError::DomainError { name: "x", value: x });
    }
    let m = n.unsigned_abs();
    let mut sign = 1.0_f64;
    if n < 0 && (m & 1) == 1 {
        sign = -sign;
    }
    let ax = if x < 0.0 {
        if (m & 1) == 1 {
            sign = -sign;
        }
        -x
    } else {
        x
    };
    if ax == 0.0 {
        return Ok(if m == 0 { 1.0 } else { 0.0 });
    }
    let val = bessel_j_nonneg(m, ax)?;
    Ok(sign * val)
}

/// Bessel function of the second kind `Y_n(x)` (Neumann/Weber function) for integer order `n`
/// and strictly positive finite real `x > 0` (DLMF §10.2.3).
///
/// Uses symmetry `Y_{-n}(x) = (-1)^n Y_n(x)` (DLMF §10.4.1).
///
/// # Errors
/// Returns [`MathError::DomainError`] if `x <= 0.0` or `x` is non-finite, or
/// [`MathError::NotConverged`] if an iterative evaluation exceeds its budget.
pub fn bessel_y(n: i32, x: f64) -> Result<f64, MathError> {
    if !x.is_finite() || x <= 0.0 {
        return Err(MathError::DomainError { name: "x", value: x });
    }
    let m = n.unsigned_abs();
    let sign = if n < 0 && (m & 1) == 1 { -1.0 } else { 1.0 };
    let val = bessel_y_nonneg(m, x)?;
    Ok(sign * val)
}

/// Modified Bessel function of the first kind `I_n(x)` for integer order `n` and finite real `x`
/// (DLMF §10.25.2).
///
/// Uses symmetry `I_{-n}(x) = I_n(x)` and `I_n(-x) = (-1)^n I_n(x)` (DLMF §10.27.1, §10.34.1).
///
/// # Errors
/// Returns [`MathError::DomainError`] if `x` is `NaN` or infinite, or [`MathError::NotConverged`]
/// if an iterative evaluation exceeds its budget.
pub fn bessel_i(n: i32, x: f64) -> Result<f64, MathError> {
    if !x.is_finite() {
        return Err(MathError::DomainError { name: "x", value: x });
    }
    let m = n.unsigned_abs();
    let (sign, ax) = if x < 0.0 { (if (m & 1) == 1 { -1.0 } else { 1.0 }, -x) } else { (1.0, x) };
    if ax == 0.0 {
        return Ok(if m == 0 { 1.0 } else { 0.0 });
    }
    let val = bessel_i_nonneg(m, ax)?;
    Ok(sign * val)
}

/// Modified Bessel function of the second kind `K_n(x)` (Macdonald function) for integer order `n`
/// and strictly positive finite real `x > 0` (DLMF §10.27.4).
///
/// Uses symmetry `K_{-n}(x) = K_n(x)` (DLMF §10.27.3).
///
/// # Errors
/// Returns [`MathError::DomainError`] if `x <= 0.0` or `x` is non-finite, or
/// [`MathError::NotConverged`] if an iterative evaluation exceeds its budget.
pub fn bessel_k(n: i32, x: f64) -> Result<f64, MathError> {
    if !x.is_finite() || x <= 0.0 {
        return Err(MathError::DomainError { name: "x", value: x });
    }
    let m = n.unsigned_abs();
    bessel_k_nonneg(m, x)
}

fn bessel_j_nonneg(m: u32, x: f64) -> Result<f64, MathError> {
    let mf = f64::from(m);
    if x <= 4.0 || x * x <= 2.0 * (mf + 1.0) {
        return bessel_j_series(m, x);
    }
    if x >= 35.0 && mf * mf <= 0.5 * x {
        return Ok(hankel_j_y(m, x).0);
    }
    let (_, j_m, _, _, _) = miller_j_and_y_sums(m, x)?;
    Ok(j_m)
}

fn bessel_j_series(m: u32, x: f64) -> Result<f64, MathError> {
    let mf = f64::from(m);
    let half_x = 0.5 * x;
    let ln_pref = if m == 0 { 0.0 } else { mf * half_x.ln() - ln_factorial(u64::from(m)) };
    if ln_pref < -750.0 {
        return Ok(0.0);
    }
    let neg_q = -(half_x * half_x);
    let mut term = 1.0_f64;
    let mut sum = 1.0_f64;
    for k in 1..=MAX_SERIES_ITERS {
        let kf = k as f64;
        term *= neg_q / (kf * (mf + kf));
        let next = sum + term;
        if term.abs() <= 0.25 * f64::EPSILON * next.abs().max(f64::MIN_POSITIVE) {
            return Ok(ln_pref.exp() * next);
        }
        sum = next;
    }
    Err(MathError::NotConverged { algorithm: "bessel_j_series", iterations: MAX_SERIES_ITERS })
}

/// Hankel's large-argument asymptotic expansion for `(J_m(x), Y_m(x))` (DLMF §10.17.3–4).
fn hankel_j_y(m: u32, x: f64) -> (f64, f64) {
    let mf = f64::from(m);
    let mu = 4.0 * mf * mf;
    let inv_8x = 1.0 / (8.0 * x);
    let mut p_sum = 1.0_f64;
    let mut q_sum = 0.0_f64;
    let mut a_k = 1.0_f64;
    let mut prev_abs = f64::INFINITY;
    for k in 1..=36_u32 {
        let odd = f64::from(2 * k - 1);
        let next_a = a_k * (mu - odd * odd) * inv_8x / f64::from(k);
        let next_abs = next_a.abs();
        if !next_a.is_finite() || (k > 2 && next_abs > prev_abs) {
            break;
        }
        a_k = next_a;
        prev_abs = next_abs;
        if (k & 1) == 0 {
            let sign = if ((k / 2) & 1) == 0 { 1.0 } else { -1.0 };
            p_sum += sign * a_k;
        } else {
            let sign = if (((k - 1) / 2) & 1) == 0 { 1.0 } else { -1.0 };
            q_sum += sign * a_k;
        }
        if next_abs <= 0.25 * f64::EPSILON * p_sum.abs().max(q_sum.abs()) {
            break;
        }
    }
    // Reduce phase modulo 2*pi cleanly: omega = x - (m % 4) * pi/2 - pi/4.
    let quad = f64::from(m & 3) * (0.5 * PI);
    let omega = x - quad - FRAC_PI_4;
    let (s, c) = omega.sin_cos();
    let amp = (FRAC_2_PI / x).sqrt();
    (amp * (p_sum * c - q_sum * s), amp * (p_sum * s + q_sum * c))
}

/// Miller's downward recurrence returning `(J_0, J_m, J_1, y0_neumann_sum, y1_neumann_sum)`.
fn miller_j_and_y_sums(m: u32, x: f64) -> Result<(f64, f64, f64, f64, f64), MathError> {
    let base = f64::from(m).max(x.ceil());
    let margin = 44.0 + (2.6 * base.sqrt()).ceil();
    let m_top_f = base + margin;
    if !m_top_f.is_finite() || m_top_f > f64::from(MAX_MILLER_ORDER) {
        return Err(MathError::NotConverged { algorithm: "bessel_j_miller", iterations: MAX_MILLER_ORDER as usize });
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let mut start = m_top_f as u32;
    if (start & 1) == 1 {
        start = start.saturating_add(1);
    }
    let inv_x = 1.0 / x;
    let mut y_next = 0.0_f64;
    let mut y_curr = 1.0_f64;
    let mut target = if m == start { 1.0 } else { 0.0 };
    let mut norm = 0.0_f64;
    let mut y0_sum = 0.0_f64;
    let mut y1_sum = 0.0_f64;
    let mut y1_val = 0.0_f64;

    for k in (1..=start).rev() {
        let kf = f64::from(k);
        if (k & 1) == 0 {
            let j = k / 2;
            let jf = f64::from(j);
            norm += 2.0 * y_curr;
            let alt = if (j & 1) == 0 { 1.0 } else { -1.0 };
            y0_sum += (alt / jf) * y_curr;
        } else if k >= 3 {
            let j = (k - 1) / 2;
            let jf = f64::from(j);
            let alt = if (j & 1) == 0 { 1.0 } else { -1.0 };
            y1_sum += (alt * (2.0 * jf + 1.0) / (jf * (jf + 1.0))) * y_curr;
        }
        if k == 1 {
            y1_val = y_curr;
        }
        let y_prev = (2.0 * kf * inv_x) * y_curr - y_next;
        y_next = y_curr;
        y_curr = y_prev;
        if k - 1 == m {
            target = y_curr;
        }
        if y_curr.abs() > RESCALE_LIMIT {
            y_curr *= INV_RESCALE_LIMIT;
            y_next *= INV_RESCALE_LIMIT;
            target *= INV_RESCALE_LIMIT;
            norm *= INV_RESCALE_LIMIT;
            y0_sum *= INV_RESCALE_LIMIT;
            y1_sum *= INV_RESCALE_LIMIT;
            y1_val *= INV_RESCALE_LIMIT;
        }
    }
    let y0_val = y_curr;
    norm += y0_val;
    if !norm.is_finite() || norm == 0.0 {
        return Err(MathError::NotConverged { algorithm: "bessel_j_miller", iterations: start as usize });
    }
    let inv_norm = 1.0 / norm;
    Ok((y0_val * inv_norm, target * inv_norm, y1_val * inv_norm, y0_sum * inv_norm, y1_sum * inv_norm))
}

fn bessel_y_nonneg(m: u32, x: f64) -> Result<f64, MathError> {
    let mf = f64::from(m);
    if x >= 35.0 && mf * mf <= 0.5 * x {
        return Ok(hankel_j_y(m, x).1);
    }
    let (y0, y1) = if x >= 35.0 {
        (hankel_j_y(0, x).1, hankel_j_y(1, x).1)
    } else if x <= 4.0 {
        (bessel_y0_series(x)?, bessel_y1_series(x)?)
    } else {
        let (j0, _, j1, y0_sum, y1_sum) = miller_j_and_y_sums(1, x)?;
        let log_term = (0.5 * x).ln() + EULER_GAMMA;
        let y0 = FRAC_2_PI * (log_term * j0 - 2.0 * y0_sum);
        let y1 = FRAC_2_PI * (log_term * j1 - j0 / x - j1 - y1_sum);
        (y0, y1)
    };
    if m == 0 {
        return Ok(y0);
    }
    if m == 1 {
        return Ok(y1);
    }
    // Upward recurrence is unconditionally stable for Y_n(x).
    let inv_x = 1.0 / x;
    let mut y_prev = y0;
    let mut y_curr = y1;
    for k in 1..m {
        let y_next = (2.0 * f64::from(k) * inv_x) * y_curr - y_prev;
        if !y_next.is_finite() {
            return Ok(y_next);
        }
        y_prev = y_curr;
        y_curr = y_next;
    }
    Ok(y_curr)
}

fn bessel_y0_series(x: f64) -> Result<f64, MathError> {
    let half_x = 0.5 * x;
    let q = half_x * half_x;
    let j0 = bessel_j_series(0, x)?;
    let mut term = 1.0_f64;
    let mut harmonic = 0.0_f64;
    let mut sum = 0.0_f64;
    for k in 1..=MAX_SERIES_ITERS {
        let kf = k as f64;
        term *= -q / (kf * kf);
        harmonic += 1.0 / kf;
        let contrib = term * harmonic;
        let next = sum + contrib;
        if contrib.abs() <= 0.25 * f64::EPSILON * next.abs().max(f64::MIN_POSITIVE) {
            return Ok(FRAC_2_PI * ((half_x.ln() + EULER_GAMMA) * j0 - next));
        }
        sum = next;
    }
    Err(MathError::NotConverged { algorithm: "bessel_y0_series", iterations: MAX_SERIES_ITERS })
}

fn bessel_y1_series(x: f64) -> Result<f64, MathError> {
    let half_x = 0.5 * x;
    let q = half_x * half_x;
    let j1 = bessel_j_series(1, x)?;
    let mut term = half_x;
    let mut harmonic = 0.0_f64;
    let mut sum = term; // k = 0: 2*H_0 + 1/1 = 1
    for k in 1..=MAX_SERIES_ITERS {
        let kf = k as f64;
        term *= -q / (kf * (kf + 1.0));
        harmonic += 1.0 / kf;
        let coeff = 2.0 * harmonic + 1.0 / (kf + 1.0);
        let contrib = term * coeff;
        let next = sum + contrib;
        if contrib.abs() <= 0.25 * f64::EPSILON * next.abs().max(f64::MIN_POSITIVE) {
            return Ok(FRAC_2_PI * ((half_x.ln() + EULER_GAMMA) * j1 - 1.0 / x - 0.5 * next));
        }
        sum = next;
    }
    Err(MathError::NotConverged { algorithm: "bessel_y1_series", iterations: MAX_SERIES_ITERS })
}

fn bessel_i_nonneg(m: u32, x: f64) -> Result<f64, MathError> {
    let mf = f64::from(m);
    if x >= 40.0 && mf * mf <= 0.5 * x {
        return Ok(bessel_i_asymptotic(m, x));
    }
    bessel_i_series(m, x)
}

fn bessel_i_asymptotic(m: u32, x: f64) -> f64 {
    let mf = f64::from(m);
    let mu = 4.0 * mf * mf;
    let inv_8x = 1.0 / (8.0 * x);
    let mut sum = 1.0_f64;
    let mut term = 1.0_f64;
    let mut prev_abs = f64::INFINITY;
    for k in 1..=36_u32 {
        let odd = f64::from(2 * k - 1);
        let next = -term * (mu - odd * odd) * inv_8x / f64::from(k);
        let next_abs = next.abs();
        if !next.is_finite() || (k > 2 && next_abs > prev_abs) {
            break;
        }
        term = next;
        prev_abs = next_abs;
        sum += term;
        if next_abs <= 0.25 * f64::EPSILON * sum.abs() {
            break;
        }
    }
    let ln_val = x - 0.5 * (TAU * x).ln() + sum.ln();
    ln_val.exp()
}

/// Strictly positive power series for `I_m(x)` with dynamic log-rescaling (zero cancellation).
fn bessel_i_series(m: u32, x: f64) -> Result<f64, MathError> {
    let mf = f64::from(m);
    let half_x = 0.5 * x;
    let ln_pref = if m == 0 { 0.0 } else { mf * half_x.ln() - ln_factorial(u64::from(m)) };
    let q = half_x * half_x;
    let mut term = 1.0_f64;
    let mut sum = 1.0_f64;
    let mut log_scale = 0.0_f64;
    for k in 1..=MAX_SERIES_ITERS {
        let kf = k as f64;
        term *= q / (kf * (mf + kf));
        sum += term;
        if sum > RESCALE_LIMIT || term > RESCALE_LIMIT {
            sum *= INV_RESCALE_LIMIT;
            term *= INV_RESCALE_LIMIT;
            log_scale += LN_RESCALE_LIMIT;
        }
        if term <= 0.25 * f64::EPSILON * sum && kf > half_x {
            let ln_res = ln_pref + log_scale + sum.ln();
            return Ok(ln_res.exp());
        }
    }
    Err(MathError::NotConverged { algorithm: "bessel_i_series", iterations: MAX_SERIES_ITERS })
}

fn bessel_k_nonneg(m: u32, x: f64) -> Result<f64, MathError> {
    if x <= 2.0 {
        let k0 = bessel_k0_small(x)?;
        if m == 0 {
            return Ok(k0);
        }
        let k1 = bessel_k1_small(x)?;
        if m == 1 {
            return Ok(k1);
        }
        return bessel_k_upward_scaled(m, x, k0, k1, 0.0);
    }
    let (ek0, ek1) = bessel_k0_k1_scaled_large(x);
    if m == 0 {
        return Ok(ek0 * (-x).exp());
    }
    if m == 1 {
        return Ok(ek1 * (-x).exp());
    }
    bessel_k_upward_scaled(m, x, ek0, ek1, -x)
}

fn bessel_k0_small(x: f64) -> Result<f64, MathError> {
    let half_x = 0.5 * x;
    let q = half_x * half_x;
    let i0 = bessel_i_series(0, x)?;
    let mut term = 1.0_f64;
    let mut harmonic = 0.0_f64;
    let mut sum = 0.0_f64;
    for k in 1..=MAX_SERIES_ITERS {
        let kf = k as f64;
        term *= q / (kf * kf);
        harmonic += 1.0 / kf;
        let contrib = term * harmonic;
        let next = sum + contrib;
        if contrib <= 0.25 * f64::EPSILON * next.max(f64::MIN_POSITIVE) {
            return Ok(-(half_x.ln() + EULER_GAMMA) * i0 + next);
        }
        sum = next;
    }
    Err(MathError::NotConverged { algorithm: "bessel_k0_small", iterations: MAX_SERIES_ITERS })
}

fn bessel_k1_small(x: f64) -> Result<f64, MathError> {
    let half_x = 0.5 * x;
    let q = half_x * half_x;
    let i1 = bessel_i_series(1, x)?;
    let mut term = half_x;
    let mut harmonic = 0.0_f64;
    let mut sum = term; // k = 0: 2*H_0 + 1 = 1
    for k in 1..=MAX_SERIES_ITERS {
        let kf = k as f64;
        term *= q / (kf * (kf + 1.0));
        harmonic += 1.0 / kf;
        let contrib = term * (2.0 * harmonic + 1.0 / (kf + 1.0));
        let next = sum + contrib;
        if contrib <= 0.25 * f64::EPSILON * next.max(f64::MIN_POSITIVE) {
            return Ok(1.0 / x + (half_x.ln() + EULER_GAMMA) * i1 - 0.5 * next);
        }
        sum = next;
    }
    Err(MathError::NotConverged { algorithm: "bessel_k1_small", iterations: MAX_SERIES_ITERS })
}

/// Evaluate `(e^x K_0(x), e^x K_1(x))` for `x > 2` via the exponentially convergent trapezoidal rule
/// on `e^x K_nu(x) = int_0^inf exp(-2 x sinh^2(t / 2)) cosh(nu t) dt` (DLMF §10.32.9; Trefethen & Weideman 2014).
fn bessel_k0_k1_scaled_large(x: f64) -> (f64, f64) {
    if x >= 40.0 {
        return (bessel_k_scaled_asymptotic(0, x), bessel_k_scaled_asymptotic(1, x));
    }
    let h = 0.12_f64 / (0.5 * x).sqrt().max(1.0);
    let mut s0 = 0.5_f64;
    let mut s1 = 0.5_f64;
    for k in 1..=2_048_u32 {
        let t = f64::from(k) * h;
        let sh = (0.5 * t).sinh();
        let exponent = -2.0 * x * sh * sh;
        if exponent < -45.0 {
            break;
        }
        let w = exponent.exp();
        s0 += w;
        s1 += w * t.cosh();
    }
    (h * s0, h * s1)
}

fn bessel_k_scaled_asymptotic(m: u32, x: f64) -> f64 {
    let mf = f64::from(m);
    let mu = 4.0 * mf * mf;
    let inv_8x = 1.0 / (8.0 * x);
    let mut sum = 1.0_f64;
    let mut term = 1.0_f64;
    let mut prev_abs = f64::INFINITY;
    for k in 1..=36_u32 {
        let odd = f64::from(2 * k - 1);
        let next = term * (mu - odd * odd) * inv_8x / f64::from(k);
        let next_abs = next.abs();
        if !next.is_finite() || (k > 2 && next_abs > prev_abs) {
            break;
        }
        term = next;
        prev_abs = next_abs;
        sum += term;
        if next_abs <= 0.25 * f64::EPSILON * sum.abs() {
            break;
        }
    }
    (0.5 * PI / x).sqrt() * sum
}

/// Strictly positive upward recurrence for `K_m(x)` carrying an explicit `log_scale` offset.
fn bessel_k_upward_scaled(m: u32, x: f64, mut k_prev: f64, mut k_curr: f64, mut log_scale: f64) -> Result<f64, MathError> {
    let inv_x = 1.0 / x;
    for k in 1..m {
        let k_next = k_prev + (2.0 * f64::from(k) * inv_x) * k_curr;
        k_prev = k_curr;
        k_curr = k_next;
        if k_curr > RESCALE_LIMIT {
            k_prev *= INV_RESCALE_LIMIT;
            k_curr *= INV_RESCALE_LIMIT;
            log_scale += LN_RESCALE_LIMIT;
        }
        if !k_curr.is_finite() {
            return Ok(f64::INFINITY);
        }
    }
    Ok((log_scale + k_curr.ln()).exp())
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use super::{bessel_i, bessel_j, bessel_k, bessel_y};

    #[test]
    fn bessel_j_and_y_satisfy_wronskian_and_dlmf_reference_values() {
        // DLMF §10.2 / Abramowitz & Stegun Table 9.1 reference values:
        // J_0(1) = 0.7651976865579666, J_1(1) = 0.4400505857449335,
        // Y_0(1) = 0.08825696421567696, Y_1(1) = -0.7812128213002887.
        assert!((bessel_j(0, 1.0).unwrap() - 0.765_197_686_557_966_6).abs() < 1e-14);
        assert!((bessel_j(1, 1.0).unwrap() - 0.440_050_585_744_933_5).abs() < 1e-14);
        assert!((bessel_y(0, 1.0).unwrap() - 0.088_256_964_215_676_96).abs() < 1e-14);
        assert!((bessel_y(1, 1.0).unwrap() - (-0.781_212_821_300_288_7)).abs() < 1e-14);

        // Exact Wronskian J_{n+1}(x) Y_n(x) - J_n(x) Y_{n+1}(x) = 2 / (pi * x) across all branches.
        for &x in &[0.05, 0.5, 2.5, 4.0, 4.5, 10.0, 20.0, 34.5, 35.5, 100.0] {
            let expected = 2.0 / (PI * x);
            for n in [0, 1, 2, 5, 10] {
                let jn = bessel_j(n, x).unwrap();
                let jn1 = bessel_j(n + 1, x).unwrap();
                let yn = bessel_y(n, x).unwrap();
                let yn1 = bessel_y(n + 1, x).unwrap();
                let w = jn1 * yn - jn * yn1;
                let scale = (jn1 * yn).abs().max((jn * yn1).abs()).max(expected);
                assert!((w - expected).abs() <= 1e-12 * scale, "n={n}, x={x}: W={w} vs {expected}");
            }
        }
        // Negative orders and negative arguments.
        assert!((bessel_j(-3, 2.5).unwrap() + bessel_j(3, 2.5).unwrap()).abs() < 1e-15);
        assert!((bessel_j(3, -2.5).unwrap() + bessel_j(3, 2.5).unwrap()).abs() < 1e-15);
        assert!((bessel_y(-2, 2.5).unwrap() - bessel_y(2, 2.5).unwrap()).abs() < 1e-14);
        assert_eq!(bessel_j(0, 0.0).unwrap(), 1.0);
        assert_eq!(bessel_j(2, 0.0).unwrap(), 0.0);
        assert!(bessel_y(0, 0.0).is_err());
        assert!(bessel_y(0, -1.0).is_err());
    }

    #[test]
    fn bessel_i_and_k_satisfy_wronskian_and_dlmf_reference_values() {
        // Abramowitz & Stegun Table 9.8:
        // I_0(1) = 1.2660658777520084, I_1(1) = 0.5651591039924851,
        // K_0(1) = 0.4210244382407083, K_1(1) = 0.6019072301972346.
        assert!((bessel_i(0, 1.0).unwrap() - 1.266_065_877_752_008_4).abs() < 1e-14);
        assert!((bessel_i(1, 1.0).unwrap() - 0.565_159_103_992_485_1).abs() < 1e-14);
        assert!((bessel_k(0, 1.0).unwrap() - 0.421_024_438_240_708_3).abs() < 1e-14);
        assert!((bessel_k(1, 1.0).unwrap() - 0.601_907_230_197_234_6).abs() < 1e-14);

        // Exact Wronskian I_n(x) K_{n+1}(x) + I_{n+1}(x) K_n(x) = 1 / x across small, intermediate, and large x.
        for &x in &[0.05, 0.5, 1.9, 2.0, 2.1, 5.0, 15.0, 39.5, 40.5, 100.0] {
            let expected = 1.0 / x;
            for n in [0, 1, 3, 8] {
                let in_v = bessel_i(n, x).unwrap();
                let in1 = bessel_i(n + 1, x).unwrap();
                let kn = bessel_k(n, x).unwrap();
                let kn1 = bessel_k(n + 1, x).unwrap();
                let w = in_v * kn1 + in1 * kn;
                assert!((w - expected).abs() <= 1e-12 * expected, "n={n}, x={x}: W={w} vs {expected}");
            }
        }
        assert!((bessel_i(-3, 2.0).unwrap() - bessel_i(3, 2.0).unwrap()).abs() < 1e-15);
        assert!((bessel_i(3, -2.0).unwrap() + bessel_i(3, 2.0).unwrap()).abs() < 1e-15);
        assert!((bessel_k(-3, 2.0).unwrap() - bessel_k(3, 2.0).unwrap()).abs() < 1e-15);
        assert_eq!(bessel_i(0, 0.0).unwrap(), 1.0);
        assert_eq!(bessel_i(1, 0.0).unwrap(), 0.0);
        assert!(bessel_k(0, 0.0).is_err());
        assert!(bessel_k(0, -2.0).is_err());
    }
}
