//! Stage 4B audit regression tests for `R1-S01-05` and `R1-S01-06` in `lumencraft-math`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumencraft_math::MathError;
use lumencraft_math::dist::{
    chi2_isf, chi2_ppf, chi2_sf, f_cdf, f_isf, f_ppf, f_sf, student_t_isf, student_t_ppf, student_t_sf, student_t_two_sided_isf,
    student_t_two_sided_p,
};
use lumencraft_math::special::{
    ln_reg_gamma_q, reg_beta_inv, reg_beta_inv_pair, reg_beta_xy, reg_gamma_p, reg_gamma_p_inv, reg_gamma_q, reg_gamma_q_inv,
};

#[test]
fn r1_s01_05_small_shape_inverse_gamma_converges_accurately() {
    for &(a, q) in &[(0.001, 0.05), (0.001, 0.3), (0.001, 0.45), (0.01, 0.05)] {
        let x = reg_gamma_q_inv(a, q).unwrap();
        let q_back = reg_gamma_q(a, x).unwrap();
        let rel_err = (q_back - q).abs() / q;
        assert!(rel_err < 1e-12, "reg_gamma_q_inv({a}, {q}) -> x={x}, Q={q_back}, rel_err={rel_err}");
    }

    let xp = reg_gamma_p_inv(0.001, 0.7).unwrap();
    let p_back = reg_gamma_p(0.001, xp).unwrap();
    assert!((p_back - 0.7).abs() / 0.7 < 1e-12, "reg_gamma_p_inv(0.001, 0.7) -> xp={xp}, P={p_back}");

    let chi2_val = chi2_isf(0.05, 0.002).unwrap();
    let chi2_back = chi2_sf(chi2_val, 0.002).unwrap();
    assert!((chi2_back - 0.05).abs() / 0.05 < 1e-12, "chi2_isf(0.05, 0.002) -> {chi2_val}, sf={chi2_back}");

    let chi2_p = chi2_ppf(0.7, 0.002).unwrap();
    assert!((chi2_p - 2.0 * xp).abs() <= 1e-12 * chi2_p);

    // Deep-upper tail with tiny shape both below x=1 and above x=1.
    for &(a, q) in &[(1e-10, 1e-9), (1e-10, 1e-15)] {
        let x = reg_gamma_q_inv(a, q).unwrap();
        let ln_q_back = ln_reg_gamma_q(a, x).unwrap();
        let target = q.ln();
        assert!((ln_q_back - target).abs() <= 1e-11 * target.abs(), "a={a}, q={q}: x={x}, ln_q_back={ln_q_back} vs {target}");
    }
}

#[test]
fn r1_s01_05_unbracketable_or_underflowing_inverses_return_not_converged() {
    assert!(matches!(reg_gamma_q_inv(0.001, 0.6), Err(MathError::NotConverged { .. })));
    assert!(matches!(reg_gamma_p_inv(0.001, 0.1), Err(MathError::NotConverged { .. })));
    assert!(matches!(reg_gamma_p_inv(1e-6, 0.5), Err(MathError::NotConverged { .. })));
    assert!(matches!(reg_beta_inv(0.001, 1.0, 0.1), Err(MathError::NotConverged { .. })));
    assert!(matches!(reg_beta_inv_pair(1.0, 0.001, 0.7, 0.3), Err(MathError::NotConverged { .. })));

    // Outside Temme window at a >= 4e6: must never return an inaccurate Ok midpoint.
    match reg_gamma_q_inv(1e8, 1e-12) {
        Ok(x) => {
            let target = 1e-12_f64.ln();
            let ln_q = ln_reg_gamma_q(1e8, x).unwrap();
            assert!((ln_q - target).abs() <= 1e-9 * target.abs(), "inaccurate root x={x}, ln_q={ln_q}");
        }
        Err(MathError::NotConverged { .. }) => {}
        Err(err) => panic!("unexpected error variant: {err:?}"),
    }
}

#[test]
fn r1_s01_06_beta_inv_pair_preserves_tiny_complement_precision() {
    let (x, y) = reg_beta_inv_pair(1.0, 0.001, 0.3, 0.7).unwrap();
    let want_y = 0.7_f64.powf(1000.0);
    assert!((y - want_y).abs() / want_y < 1e-12, "y={y} vs want={want_y}");
    assert!((x - (1.0 - want_y)).abs() <= 1e-15, "x={x}");
    assert!((reg_beta_xy(x, y, 1.0, 0.001).unwrap() - 0.3).abs() < 1e-12);

    // Symmetric case where x is tiny while p > q.
    let (x2, y2) = reg_beta_inv_pair(0.001, 1.0, 0.7, 0.3).unwrap();
    assert!((x2 - want_y).abs() / want_y < 1e-12, "x2={x2} vs want={want_y}");
    assert!((y2 - (1.0 - want_y)).abs() <= 1e-15, "y2={y2}");

    // Median with small b=0.002 where y = 0.5^500 (~3.05e-151) is tiny yet representable.
    let (_x3, y3) = reg_beta_inv_pair(1.0, 0.002, 0.5, 0.5).unwrap();
    let want_y3 = 0.5_f64.powf(500.0);
    assert!((y3 - want_y3).abs() / want_y3 < 1e-12, "y3={y3} vs want={want_y3}");

    // Ensure reg_beta_xy handles imbalanced a >> b without (a + b + 2) float cancellation.
    let val = reg_beta_xy(1.0, 1e-25, 1e18, 0.5).unwrap();
    let expected = reg_gamma_q(0.5, 1e-7).unwrap();
    assert!((val - expected).abs() < 1e-10, "val={val} vs expected={expected}");
}

#[test]
fn r1_s01_06_student_t_and_f_extreme_df_quantiles_roundtrip_accurately() {
    for df in 1..=250 {
        let df_f = f64::from(df);
        let t = student_t_two_sided_isf(0.05, df_f).unwrap();
        let p_back = student_t_two_sided_p(t, df_f).unwrap();
        assert!((p_back - 0.05).abs() / 0.05 < 1e-12, "df={df}: t={t}, p_back={p_back}");
    }

    for &df in &[1e9_f64, 5e11_f64, 9.9e11_f64] {
        let t = student_t_two_sided_isf(0.05, df).unwrap();
        let p_back = student_t_two_sided_p(t, df).unwrap();
        assert!((p_back - 0.05).abs() / 0.05 < 1e-12, "df={df}: t={t}, p_back={p_back}");
        assert!((t - 1.959_963_984_540_054).abs() < 1e-7, "df={df}: t={t}");
    }

    for &q in &[1e-12_f64, 1e-6, 0.025] {
        let t = student_t_isf(q, 5e11).unwrap();
        let sf = student_t_sf(t, 5e11).unwrap();
        assert!((sf - q).abs() / q < 1e-11, "student_t_isf({q}, 5e11) -> t={t}, sf={sf}");

        let tp = student_t_ppf(q, 5e11).unwrap();
        assert!((tp + t).abs() <= 1e-12 * t.abs());
    }

    for &(q, df1, df2) in &[(0.05, 1.0, 5e11), (0.05, 5e11, 1.0), (1e-10, 3.0, 5e11)] {
        let f_val = f_isf(q, df1, df2).unwrap();
        let sf_back = f_sf(f_val, df1, df2).unwrap();
        assert!((sf_back - q).abs() / q < 1e-11, "f_isf({q}, {df1}, {df2}) -> {f_val}, sf={sf_back}");
    }

    for &(p, df1, df2) in &[(1e-10, 5e11, 2.0), (0.05, 5e11, 2.0), (1e-10, 2.0, 5e11)] {
        let f_val = f_ppf(p, df1, df2).unwrap();
        let cdf_back = f_cdf(f_val, df1, df2).unwrap();
        assert!((cdf_back - p).abs() / p < 1e-11, "f_ppf({p}, {df1}, {df2}) -> {f_val}, cdf={cdf_back}");
    }
}
