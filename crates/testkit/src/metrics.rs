//! Numerical accuracy metrics, ULP distances, NIST Log Relative Error (LRE), and test assertions.
//!
//! Reference:
//! - McCullough, B. D. (1998). Assessing the reliability of statistical software: Part I.
//!   *The American Statistician*, 52(4), 358-366.

/// Maximum decimal precision representable in IEEE-754 binary64 (`53 * log10(2) ~ 15.955`).
pub const MAX_F64_LRE_DIGITS: f64 = 15.954_589_770_191_003;

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

/// Ordered IEEE-754 `f64` ULP distance between two finite values (`+0.0` and `-0.0` have distance `0`).
/// Returns `None` if either value is non-finite.
#[must_use]
pub fn ulp_distance(actual: f64, expected: f64) -> Option<u64> {
    if !actual.is_finite() || !expected.is_finite() {
        return None;
    }
    if actual == 0.0 && expected == 0.0 {
        return Some(0);
    }
    let map_ordered = |x: f64| -> u64 {
        let bits = x.to_bits();
        if (bits >> 63) != 0 { !bits } else { bits ^ (1_u64 << 63) }
    };
    let oa = map_ordered(actual);
    let oe = map_ordered(expected);
    Some(oa.abs_diff(oe))
}

/// NIST StRD Log Relative Error (`LRE`) measuring accurate significant decimal digits in `[0, 15.95]`:
/// - For `certified != 0`: `LRE = clamp(-log10(|actual - certified| / |certified|), 0.0, 15.955)`.
/// - For `certified == 0`: `LRE = clamp(-log10(|actual|), 0.0, 15.955)`.
/// - Returns `0.0` when `actual` is `NaN`, infinite, or off by `>= 100%`.
#[must_use]
pub fn log_relative_error(actual: f64, certified: f64) -> f64 {
    if !actual.is_finite() || !certified.is_finite() {
        return 0.0;
    }
    let err = (actual - certified).abs();
    if err == 0.0 {
        return MAX_F64_LRE_DIGITS;
    }
    if certified == 0.0 {
        return (-err.log10()).clamp(0.0, MAX_F64_LRE_DIGITS);
    }
    let rel = err / certified.abs();
    if rel >= 1.0 { 0.0 } else { (-rel.log10()).clamp(0.0, MAX_F64_LRE_DIGITS) }
}

/// Assert that `actual` and `expected` agree within `abs_tol` or `rel_tol`, reporting ULP & LRE on failure.
#[track_caller]
#[allow(clippy::panic)]
pub fn assert_approx_eq(actual: f64, expected: f64, abs_tol: f64, rel_tol: f64, label: &str) {
    if !approx_eq(actual, expected, abs_tol, rel_tol) {
        let lre = log_relative_error(actual, expected);
        let ulps = ulp_distance(actual, expected);
        panic!(
            "{label}: actual={actual:.17e} != expected={expected:.17e} (abs_diff={:.3e}, LRE={lre:.2} digits, ulps={ulps:?}, abs_tol={abs_tol:.2e}, rel_tol={rel_tol:.2e})",
            (actual - expected).abs()
        );
    }
}

/// Assert that `actual` achieves at least `min_digits` of NIST StRD Log Relative Error (`LRE`) vs `certified`.
#[track_caller]
#[allow(clippy::panic)]
pub fn assert_lre_at_least(actual: f64, certified: f64, min_digits: f64, label: &str) {
    let lre = log_relative_error(actual, certified);
    if lre.is_nan() || lre < min_digits {
        panic!(
            "{label}: insufficient NIST LRE accuracy: got {lre:.2} digits < required {min_digits:.2} digits (actual={actual:.17e}, certified={certified:.17e})"
        );
    }
}

/// Assert that two finite `f64` values lie within `max_ulps` ULPs of each other.
#[track_caller]
#[allow(clippy::panic)]
pub fn assert_ulp_le(actual: f64, expected: f64, max_ulps: u64, label: &str) {
    let Some(dist) = ulp_distance(actual, expected) else {
        panic!("{label}: non-finite float in ULP comparison (actual={actual}, expected={expected})");
    };
    if dist > max_ulps {
        panic!("{label}: ULP distance {dist} exceeds limit {max_ulps} (actual={actual:.17e}, expected={expected:.17e})");
    }
}
