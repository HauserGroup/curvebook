//! Safeguarded 1D root-finding primitives (Brent-Dekker method and bracketed Halley/Newton steps).
//!
//! Reference:
//! - Brent, R. P. (1973). *Algorithms for Minimization without Derivatives* (Ch. 4). Prentice-Hall.

use crate::error::MathError;

/// Hard ceiling on Brent-Dekker root-finding iterations regardless of caller configuration.
pub const MAX_BRENT_ITERS: usize = 4_096;

/// Configuration tolerances and iteration cap for 1D root-finding.
#[derive(Debug, Clone, Copy)]
pub struct RootOptions {
    /// Relative tolerance on bracket width `|b - a|`.
    pub rel_tol: f64,
    /// Absolute tolerance on bracket half-width.
    pub abs_tol: f64,
    /// Hard iteration limit (clamped internally to `1..=4_096`).
    pub max_iters: usize,
}

impl Default for RootOptions {
    fn default() -> Self {
        Self { rel_tol: 4.0 * f64::EPSILON, abs_tol: 1e-15, max_iters: 128 }
    }
}

/// Find a root of continuous scalar function `f` inside `[a, b]` where `f(a)` and `f(b)` have
/// opposite (or zero) signs, using Brent's safeguarded secant / inverse-quadratic / bisection method.
///
/// # Errors
/// Returns [`MathError::DomainError`] if `f(a)` and `f(b)` are non-finite or strictly share the same
/// sign, or [`MathError::NotConverged`] if `options.max_iters` is exhausted.
pub fn brent_root<F>(mut f: F, a: f64, b: f64, options: RootOptions) -> Result<f64, MathError>
where
    F: FnMut(f64) -> f64,
{
    if !a.is_finite() || !b.is_finite() {
        return Err(MathError::DomainError { name: "bracket", value: a + b });
    }
    let mut x_a = a;
    let mut x_b = b;
    let mut f_a = f(x_a);
    let mut f_b = f(x_b);
    if !f_a.is_finite() || !f_b.is_finite() {
        return Err(MathError::DomainError { name: "f(bracket)", value: f_a + f_b });
    }
    if f_a == 0.0 {
        return Ok(x_a);
    }
    if f_b == 0.0 {
        return Ok(x_b);
    }
    if (f_a > 0.0) == (f_b > 0.0) {
        return Err(MathError::DomainError { name: "bracket_sign", value: f_a * f_b });
    }

    let max_iters = options.max_iters.clamp(1, MAX_BRENT_ITERS);
    let mut x_c = x_a;
    let mut f_c = f_a;
    let init_span = x_b - x_a;
    let mut step = if init_span.is_finite() { init_span } else { 0.5 * x_b - 0.5 * x_a };
    let mut prev_step = step;

    for _ in 0..max_iters {
        if (f_b > 0.0) == (f_c > 0.0) {
            x_c = x_a;
            f_c = f_a;
            let span = x_b - x_a;
            step = if span.is_finite() { span } else { 0.5 * x_b - 0.5 * x_a };
            prev_step = step;
        }
        if f_c.abs() < f_b.abs() {
            x_a = x_b;
            x_b = x_c;
            x_c = x_a;
            f_a = f_b;
            f_b = f_c;
            f_c = f_a;
        }

        let tol = 2.0 * options.rel_tol * x_b.abs() + 0.5 * options.abs_tol;
        let mid = 0.5 * x_c - 0.5 * x_b;
        if mid.abs() <= tol || f_b == 0.0 {
            return Ok(x_b);
        }

        let can_interp =
            (x_c - x_b).is_finite() && (x_b - x_a).is_finite() && (x_a - x_c).is_finite() && prev_step.abs() >= tol && f_a.abs() > f_b.abs();
        if can_interp {
            let (p, q) = interp_numerator_denominator(x_a, x_b, x_c, f_a, f_b, f_c);
            let two_p = 2.0 * p;
            let three_mq = 3.0 * mid * q;
            let prev_q = prev_step * q;
            let tol_q = tol * q;
            if p.is_finite() && q.is_finite() && two_p.is_finite() && three_mq.is_finite() && prev_q.is_finite() && tol_q.is_finite() {
                let limit = (three_mq - tol_q.abs()).min(prev_q.abs());
                if two_p < limit {
                    prev_step = step;
                    step = p / q;
                } else {
                    step = mid;
                    prev_step = mid;
                }
            } else {
                step = mid;
                prev_step = mid;
            }
        } else {
            step = mid;
            prev_step = mid;
        }

        x_a = x_b;
        f_a = f_b;
        if step.is_finite() && step.abs() > tol {
            x_b += step;
        } else if mid > 0.0 {
            x_b += tol;
        } else {
            x_b -= tol;
        }
        f_b = f(x_b);
        if !f_b.is_finite() {
            return Err(MathError::DomainError { name: "f(x)", value: f_b });
        }
    }

    Err(MathError::NotConverged { algorithm: "brent_root", iterations: max_iters })
}

fn interp_numerator_denominator(x_a: f64, x_b: f64, x_c: f64, f_a: f64, f_b: f64, f_c: f64) -> (f64, f64) {
    let s = f_b / f_a;
    let half_cb = 0.5 * x_c - 0.5 * x_b;
    let (mut p, mut q) = if (x_a - x_c).abs() <= f64::EPSILON * x_c.abs().max(1.0) {
        // Linear / secant interpolation.
        (2.0 * half_cb * s, 1.0 - s)
    } else {
        // Inverse quadratic interpolation.
        let q0 = f_a / f_c;
        let r = f_b / f_c;
        (s * (2.0 * half_cb * q0 * (q0 - r) - (x_b - x_a) * (r - 1.0)), (q0 - 1.0) * (r - 1.0) * (s - 1.0))
    };
    if p > 0.0 {
        q = -q;
    } else {
        p = -p;
    }
    (p, q)
}

/// Take a single safeguarded Halley step inside `(lo, hi)` given `f(x)`, `f'(x)`, and `f''(x)/f'(x)`.
/// Falls back cleanly to a bisection step if the Halley proposal leaves `(lo, hi)` or overflows.
#[must_use]
pub fn safeguarded_halley_step(x: f64, fx: f64, dfx: f64, d2_over_d1: f64, lo: f64, hi: f64) -> f64 {
    if dfx.is_finite() && dfx.abs() > f64::MIN_POSITIVE {
        let newton = fx / dfx;
        let denom = (1.0 - 0.5 * newton * d2_over_d1).clamp(0.2, 5.0);
        let step = newton / denom;
        let candidate = x - step;
        if step.is_finite() && candidate.is_finite() {
            if step.abs() <= 2.0 * f64::EPSILON * x.abs().max(f64::MIN_POSITIVE) {
                return candidate.clamp(lo, hi);
            }
            if candidate > lo && candidate < hi {
                return candidate;
            }
        }
    }
    0.5 * lo + 0.5 * hi
}

#[cfg(test)]
mod tests {
    use super::{RootOptions, brent_root};

    #[test]
    fn brent_solves_sqrt2_and_transcendental_equation() {
        let root = brent_root(|x| x * x - 2.0, 0.0, 2.0, RootOptions::default()).unwrap();
        assert!((root - std::f64::consts::SQRT_2).abs() < 1e-15);

        let cos_fixed = brent_root(|x| x.cos() - x, 0.0, 1.0, RootOptions::default()).unwrap();
        assert!((cos_fixed - 0.739_085_133_215_160_7).abs() < 1e-15);

        // Zero and huge max_iters are clamped into 1..=MAX_BRENT_ITERS cleanly.
        let err = brent_root(|x| x * x - 2.0, 0.0, 2.0, RootOptions { rel_tol: 0.0, abs_tol: 0.0, max_iters: 0 }).unwrap_err();
        assert!(matches!(err, crate::error::MathError::NotConverged { iterations: 1, .. }));
        let err_cap = brent_root(|x| if x < 1.0 { -1.0 } else { 1.0 }, 0.0, 2.0, RootOptions { rel_tol: 0.0, abs_tol: 0.0, max_iters: usize::MAX })
            .unwrap_err();
        assert!(matches!(err_cap, crate::error::MathError::NotConverged { iterations: super::MAX_BRENT_ITERS, .. }));
    }
}
