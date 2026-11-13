//! Test-only numerical tolerance assertions, synthetic scientific table builders and reference
//! oracle helpers for LumenCraft.
//!
//! May only be depended on as a `[dev-dependencies]` entry (enforced by `cargo xtask layers`).
//! Governing spec: `docs/development.md` (§Tests & numerical verification, §Verified NIST StRD corpus).
#![forbid(unsafe_code)]

/// Check whether `actual` and `expected` agree within combined absolute and relative tolerance.
#[must_use]
pub fn approx_eq(actual: f64, expected: f64, abs_tol: f64, rel_tol: f64) -> bool {
    if !actual.is_finite() || !expected.is_finite() {
        return false;
    }
    let diff = (actual - expected).abs();
    let scale = actual.abs().max(expected.abs());
    diff <= abs_tol.max(rel_tol * scale)
}

#[cfg(test)]
mod tests {
    use super::approx_eq;

    #[test]
    fn approx_eq_handles_finite_and_nan() {
        assert!(approx_eq(1.000_000_1, 1.0, 1e-9, 1e-6));
        assert!(!approx_eq(f64::NAN, 1.0, 1e-3, 1e-3));
    }
}
