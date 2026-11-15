//! Stage 3A regression tests for S01 fixes in `lumencraft-math`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumencraft_math::dist::{nct_cdf, nct_sf, norm_cdf};
use lumencraft_math::noncentral_t_ci;

fn assert_near(actual: f64, expected: f64, tol: f64, label: &str) {
    assert!(actual.is_finite() && (actual - expected).abs() <= tol, "{label}: expected {expected} +/- {tol}, got {actual}");
}

#[test]
fn r1_s01_01_nct_large_delta_quadrature_and_ci() {
    // R reference integrals:
    // integrate(function(u) pnorm(512.6 * u - 500) * ..., 0, Inf) = 0.4840760
    // integrate(function(u) pnorm(450.0 * u - 448) * ..., 0, Inf) = 0.4483127
    let c1 = nct_cdf(512.6, 10.0, 500.0).unwrap();
    let c2 = nct_cdf(450.0, 10.0, 448.0).unwrap();
    assert_near(c1, 0.484_076_0, 1e-5, "nct_cdf(512.6, 10, 500)");
    assert_near(c2, 0.448_312_7, 1e-5, "nct_cdf(450.0, 10, 448)");

    // Symmetry check for negative t and delta.
    let s1 = nct_cdf(-512.6, 10.0, -500.0).unwrap();
    assert_near(c1 + s1, 1.0, 1e-12, "nct_cdf(-t, df, -delta) = 1 - nct_cdf(t, df, delta)");

    // Smooth continuity across the lambda_half = 1e5 seam (delta = sqrt(200_000) ~= 447.2135955).
    let left_seam = nct_cdf(460.0, 10.0, 447.21).unwrap();
    let right_seam = nct_cdf(460.0, 10.0, 447.22).unwrap();
    assert!(
        (left_seam - right_seam).abs() < 1e-4 && left_seam > right_seam,
        "continuity across lambda_half=1e5 seam failed: left={left_seam}, right={right_seam}"
    );

    // Huge df > 1e6 + large delta avoids ln_gamma / 1 - eu^2 cancellation.
    // Note: at df = 1e8, delta = 500, Var(t*U) = t^2 / (2*df) = 0.001255 contributes measurably!
    let z_1e8 = (501.0_f64 - 500.0) / (1.0_f64 + 501.0 * 501.0 / (2.0 * 1e8)).sqrt();
    assert_near(nct_cdf(500.0, 1e8, 500.0).unwrap(), 0.5, 1e-6, "nct_cdf huge df median");
    assert_near(nct_cdf(501.0, 1e8, 500.0).unwrap(), norm_cdf(z_1e8), 1e-6, "nct_cdf huge df=1e8 +1sd");
    assert_near(nct_cdf(501.0, 1e12, 500.0).unwrap(), norm_cdf(1.0), 1e-7, "nct_cdf huge df=1e12 +1sd");

    // Noncentral t CI roundtrip when upper delta crosses lambda_half > 1e5.
    let (lo, hi) = noncentral_t_ci(300.0, 10.0, 95.0).unwrap();
    assert!(lo < 300.0 && 300.0 < hi, "CI ({lo}, {hi}) must contain observed t = 300.0");
    assert_near(nct_sf(300.0, 10.0, lo).unwrap(), 0.025, 1e-6, "CI lower roundtrip");
    assert_near(nct_cdf(300.0, 10.0, hi).unwrap(), 0.025, 1e-6, "CI upper roundtrip");

    // Even larger t where BOTH CI bounds lie in lambda_half > 1e5.
    let (lo2, hi2) = noncentral_t_ci(1_000.0, 10.0, 95.0).unwrap();
    assert!(lo2 < 1_000.0 && 1_000.0 < hi2, "CI ({lo2}, {hi2}) must contain observed t = 1000.0");
    assert_near(nct_sf(1_000.0, 10.0, lo2).unwrap(), 0.025, 1e-6, "large CI lower roundtrip");
    assert_near(nct_cdf(1_000.0, 10.0, hi2).unwrap(), 0.025, 1e-6, "large CI upper roundtrip");
}
