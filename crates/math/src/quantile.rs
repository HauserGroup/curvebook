//! Sample quantile interpolation kernels over pre-sorted slices.
//!
//! References:
//! - Hyndman, R. J., & Fan, Y. (1996). Sample quantiles in statistical packages. *The American
//!   Statistician*, 50(4), 361-365. `doi:10.1080/00031305.1996.10473566`.

/// Linearly interpolated sample quantile (`R` type 7 / Hyndman & Fan 1996, Definition 7) on a
/// non-empty, ascending-sorted slice `sorted` for probability `p` in `[0.0, 1.0]`.
///
/// Evaluates the 0-based position `h = clamp(p, 0.0, 1.0) * (n - 1)` corresponding to
/// `p_k = (k - 1) / (n - 1)` for 1-based order statistics `x_(k)` and linearly interpolates
/// between `sorted[floor(h)]` and `sorted[ceil(h)]`.
///
/// Exact integer positions (`floor(h) == ceil(h)`, including `p <= 0.0` and `p >= 1.0`) return
/// `sorted[lo]` directly without evaluating `0.0 * (hi - lo)`. When `sorted[hi] - sorted[lo]`
/// overflows `f64` for extreme finite values of opposite sign, interpolation falls back to
/// `(1.0 - frac) * lo + frac * hi`.
///
/// Returns `None` if `sorted` is empty or `p` is non-finite (`NaN` or `±Inf`).
///
/// Reference: Hyndman, R. J., & Fan, Y. (1996). Sample quantiles in statistical packages.
/// *The American Statistician*, 50(4), 361-365. `doi:10.1080/00031305.1996.10473566`.
#[must_use]
pub fn quantile_type7_sorted(sorted: &[f64], p: f64) -> Option<f64> {
    let n = sorted.len();
    if n == 0 || !p.is_finite() {
        return None;
    }
    if n == 1 {
        return sorted.first().copied();
    }
    let pc = p.clamp(0.0, 1.0);
    let idx = pc * ((n - 1) as f64);
    let lo = (idx.floor() as usize).min(n - 1);
    let hi = (idx.ceil() as usize).min(n - 1);
    let lo_v = *sorted.get(lo)?;
    if lo == hi {
        return Some(lo_v);
    }
    let hi_v = *sorted.get(hi)?;
    let frac = idx - (lo as f64);
    let diff = hi_v - lo_v;
    if diff.is_finite() { Some(lo_v + frac * diff) } else { Some((1.0 - frac) * lo_v + frac * hi_v) }
}

#[cfg(test)]
mod tests {
    use super::quantile_type7_sorted;

    #[test]
    fn empty_and_non_finite_probability_return_none() {
        assert_eq!(quantile_type7_sorted(&[], 0.5), None);
        assert_eq!(quantile_type7_sorted(&[1.0, 2.0, 3.0], f64::NAN), None);
        assert_eq!(quantile_type7_sorted(&[1.0, 2.0, 3.0], f64::INFINITY), None);
        assert_eq!(quantile_type7_sorted(&[1.0, 2.0, 3.0], f64::NEG_INFINITY), None);
    }

    #[test]
    fn single_element_returns_that_element() {
        for p in [-1.0, 0.0, 0.25, 0.5, 1.0, 2.0] {
            assert_eq!(quantile_type7_sorted(&[42.5], p), Some(42.5));
        }
    }

    #[test]
    fn type7_exact_and_interpolated_values_match_hyndman_fan_def7() {
        let data = [10.0, 20.0, 30.0, 40.0];
        assert_eq!(quantile_type7_sorted(&data, -0.5), Some(10.0));
        assert_eq!(quantile_type7_sorted(&data, 0.0), Some(10.0));
        assert_eq!(quantile_type7_sorted(&data, 0.25), Some(17.5));
        assert_eq!(quantile_type7_sorted(&data, 0.5), Some(25.0));
        assert_eq!(quantile_type7_sorted(&data, 0.75), Some(32.5));
        assert_eq!(quantile_type7_sorted(&data, 1.0), Some(40.0));
        assert_eq!(quantile_type7_sorted(&data, 1.5), Some(40.0));

        let odd = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(quantile_type7_sorted(&odd, 0.25), Some(2.0));
        assert_eq!(quantile_type7_sorted(&odd, 0.5), Some(3.0));
        assert_eq!(quantile_type7_sorted(&odd, 0.75), Some(4.0));
    }

    #[test]
    fn exact_rank_and_extreme_value_spans_avoid_overflow_and_nan() {
        let extreme_pair = [f64::MIN, f64::MAX];
        assert_eq!(quantile_type7_sorted(&extreme_pair, 0.0), Some(f64::MIN));
        assert_eq!(quantile_type7_sorted(&extreme_pair, 1.0), Some(f64::MAX));
        let mid = quantile_type7_sorted(&extreme_pair, 0.5).unwrap();
        assert!(mid.is_finite(), "midpoint of [f64::MIN, f64::MAX] must stay finite, got {mid}");
        assert_eq!(mid, 0.0);

        let symmetric = [-1e308, 1e308];
        assert_eq!(quantile_type7_sorted(&symmetric, 0.0), Some(-1e308));
        assert_eq!(quantile_type7_sorted(&symmetric, 0.25), Some(-0.5e308));
        assert_eq!(quantile_type7_sorted(&symmetric, 0.5), Some(0.0));
        assert_eq!(quantile_type7_sorted(&symmetric, 0.75), Some(0.5e308));
        assert_eq!(quantile_type7_sorted(&symmetric, 1.0), Some(1e308));

        let triple = [-1e308, 0.0, 1e308];
        assert_eq!(quantile_type7_sorted(&triple, 0.0), Some(-1e308));
        assert_eq!(quantile_type7_sorted(&triple, 0.5), Some(0.0));
        assert_eq!(quantile_type7_sorted(&triple, 1.0), Some(1e308));
    }

    #[test]
    fn tied_adjacent_values_are_exact() {
        let tied = [0.3, 0.3, 0.3];
        assert_eq!(quantile_type7_sorted(&tied, 0.1), Some(0.3));
        assert_eq!(quantile_type7_sorted(&tied, 0.37), Some(0.3));
        assert_eq!(quantile_type7_sorted(&tied, 0.9), Some(0.3));
    }
}
