//! Stage 4C audit regression tests for `R1-S01-08` in `curvebook-math`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_math::MathError;
use curvebook_math::special::gamma;

#[test]
fn r1_s01_08_gamma_rejects_negative_infinity_and_poles_with_domain_error() {
    let err = gamma(f64::NEG_INFINITY).unwrap_err();
    assert!(matches!(err, MathError::DomainError { name: "x", value } if value == f64::NEG_INFINITY), "got {err:?}");

    let nan_err = gamma(f64::NAN).unwrap_err();
    assert!(matches!(nan_err, MathError::DomainError { name: "x", value } if value.is_nan()), "got {nan_err:?}");

    for &pole in &[0.0_f64, -0.0, -1.0, -2.0, -10.0, -1e20] {
        let pole_err = gamma(pole).unwrap_err();
        assert!(matches!(pole_err, MathError::DomainError { name: "x", .. }), "pole {pole}: {pole_err:?}");
    }

    let pos_inf = gamma(f64::INFINITY).unwrap();
    assert!(pos_inf.is_infinite() && pos_inf.is_sign_positive(), "gamma(+inf) = {pos_inf}");

    assert!((gamma(0.5).unwrap() - std::f64::consts::PI.sqrt()).abs() < 1e-15);
    assert!((gamma(-0.5).unwrap() - (-2.0 * std::f64::consts::PI.sqrt())).abs() < 1e-14);
    assert!((gamma(-1.5).unwrap() - (4.0 / 3.0 * std::f64::consts::PI.sqrt())).abs() < 1e-14);
}
