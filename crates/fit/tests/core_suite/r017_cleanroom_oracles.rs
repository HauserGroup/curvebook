//! Clean-room `D1` oracle tests for `R0.17`:
//! - Lilliefors normality test (`curvebook_fit::diagnostics::lilliefors_ks_test`),
//! - Clean-room Bessel functions (`j0`, `j1`, `jn`, `y0`, `y1`, `yn` via `CompiledDslModel`),
//! - Confluent hypergeometric functions (`hyp1f1` and `confluent_u` via `CompiledDslModel`).
//!
//! All oracles are derived from exact mathematical identities (Wronskians, three-term recurrences,
//! Bessel & Jacobi–Anger trigonometric series, Gauss–Legendre integral representations, `erf`/`exp`/
//! Laguerre/spherical Bessel closed forms, and published Dallal & Wilkinson 1986 / Stephens 1974
//! Lilliefors tables) rather than copied coefficients.

use curvebook_fit::{CompiledDslModel, ParametricModel, lilliefors_ks_test};
use curvebook_math::DeterministicRng;
use curvebook_math::dist::{lilliefors_p_value, norm_cdf};
use curvebook_math::quad::gauss_legendre_64;
use curvebook_math::special::{erf, erfcx, gamma};
use curvebook_math::sum::SummaryMoments;
use curvebook_testkit::assert_approx_eq as assert_close;

#[test]
fn r017_lilliefors_fit_diagnostics_parity_and_large_n() {
    assert!(lilliefors_ks_test(&[1.0, 2.0, 3.0, 4.0]).is_none());
    assert!(lilliefors_ks_test(&[2.0; 12]).is_none());
    assert!(lilliefors_ks_test(&[1.0, 2.0, f64::NAN, 3.0, 4.0]).is_none());

    let mut rng = DeterministicRng::from_seed(0xF170_1717_0000_0042);
    for &n in &[5_usize, 15, 50, 100, 250, 1000] {
        let mut xs: Vec<f64> = (0..n).map(|_| rng.standard_normal()).collect();
        // Inject one non-finite element that diagnostics::lilliefors_ks_test filters out.
        let mut with_nan = xs.clone();
        with_nan.push(f64::NAN);
        let res = lilliefors_ks_test(&with_nan).unwrap();

        xs.sort_by(f64::total_cmp);
        let m = SummaryMoments::from_slice(&xs).unwrap();
        let sd = m.sample_sd().unwrap();
        let nf = n as f64;
        let mut d_ref = 0.0_f64;
        for (i, &x) in xs.iter().enumerate() {
            let fz = norm_cdf((x - m.mean) / sd);
            d_ref = d_ref.max(((i + 1) as f64) / nf - fz).max(fz - (i as f64) / nf);
        }
        let p_ref = lilliefors_p_value(d_ref, n).unwrap();
        assert_close(res.statistic, d_ref, 1e-15, 1e-15, &format!("fit D_n n={n}"));
        assert_close(res.p_value, p_ref, 1e-15, 1e-15, &format!("fit p n={n}"));
        assert_eq!(res.passed, res.p_value >= 0.05);
    }

    // D&W 1986 Table 1 critical check + n=300 non-normal rejection
    let exp_sample: Vec<f64> = (0..300).map(|_| -rng.next_open_f64().ln()).collect();
    let res_exp = lilliefors_ks_test(&exp_sample).unwrap();
    assert!(!res_exp.passed && res_exp.p_value < 1e-6, "n=300 exponential should fail: p={}", res_exp.p_value);
}

#[test]
fn r017_bessel_j0_j1_y0_y1_wronskians_zeros_and_quadrature() {
    let mj0 = CompiledDslModel::compile("j0", "Y = besselj0(X)").unwrap();
    let mj1 = CompiledDslModel::compile("j1", "Y = besselj1(X)").unwrap();
    let my0 = CompiledDslModel::compile("y0", "Y = bessely0(X)").unwrap();
    let my1 = CompiledDslModel::compile("y1", "Y = bessely1(X)").unwrap();

    // 1. Exact Wronskian J1(x) Y0(x) - J0(x) Y1(x) = 2 / (pi * x) across series (<= 20) and Hankel (> 20).
    let xs = [
        1e-6,
        1e-3,
        0.1,
        0.893_576_966_279_167_5,
        2.404_825_557_695_773,
        7.999_999,
        8.000_001,
        15.0,
        19.999_999,
        20.0,
        20.000_001,
        25.0,
        100.0,
        10_000.0,
        1e6,
    ];
    for &x in &xs {
        let j0 = mj0.eval(&[x], &[], 0).unwrap();
        let j1 = mj1.eval(&[x], &[], 0).unwrap();
        let y0 = my0.eval(&[x], &[], 0).unwrap();
        let y1 = my1.eval(&[x], &[], 0).unwrap();
        let wronskian = j1 * y0 - j0 * y1;
        let expected = core::f64::consts::FRAC_2_PI / x;
        assert_close(wronskian, expected, 1e-20, 1e-13, &format!("Wronskian(J0, Y0) at x={x}"));
    }

    // 2. Near-f64 accuracy at zeros (old NR §6.5 polynomial had ~1e-8 error here).
    let j0_zeros = [2.404_825_557_695_773, 5.520_078_110_286_311, 18.071_063_967_910_924, 21.211_636_629_879_26];
    for &z in &j0_zeros {
        let val = mj0.eval(&[z], &[], 0).unwrap();
        assert!(val.abs() < 1e-14, "J0 at zero {z}: got {val}");
    }
    let j1_zeros = [3.831_705_970_207_512_3, 19.615_858_510_468_24, 22.760_084_380_592_77];
    for &z in &j1_zeros {
        let val = mj1.eval(&[z], &[], 0).unwrap();
        assert!(val.abs() < 1e-14, "J1 at zero {z}: got {val}");
    }
    let y0_zeros = [0.893_576_966_279_167_5, 19.641_309_700_887_94, 22.782_028_047_291_55];
    for &z in &y0_zeros {
        let val = my0.eval(&[z], &[], 0).unwrap();
        assert!(val.abs() < 1e-14, "Y0 at zero {z}: got {val}");
    }

    // 3. Independent Bessel integral quadrature (DLMF §10.9.2): J_0(x) = (1/pi) int_0^pi cos(x sin t) dt.
    for &x in &[0.5, 2.5, 10.0, 19.9, 20.1, 30.0] {
        let j0_quad = gauss_legendre_64(|t| (x * t.sin()).cos(), 0.0, core::f64::consts::PI) / core::f64::consts::PI;
        let j1_quad = gauss_legendre_64(|t| (t - x * t.sin()).cos(), 0.0, core::f64::consts::PI) / core::f64::consts::PI;
        assert_close(mj0.eval(&[x], &[], 0).unwrap(), j0_quad, 1e-14, 1e-13, &format!("J0 quadrature at x={x}"));
        assert_close(mj1.eval(&[x], &[], 0).unwrap(), j1_quad, 1e-14, 1e-13, &format!("J1 quadrature at x={x}"));
    }

    // 4. Parity & domain boundaries
    assert_close(mj0.eval(&[0.0], &[], 0).unwrap(), 1.0, 1e-15, 0.0, "J0(0)");
    assert_close(mj1.eval(&[0.0], &[], 0).unwrap(), 0.0, 1e-15, 0.0, "J1(0)");
    for &x in &[1.5, 12.0, 25.0] {
        assert_close(mj0.eval(&[-x], &[], 0).unwrap(), mj0.eval(&[x], &[], 0).unwrap(), 1e-15, 1e-15, "J0 even");
        assert_close(mj1.eval(&[-x], &[], 0).unwrap(), -mj1.eval(&[x], &[], 0).unwrap(), 1e-15, 1e-15, "J1 odd");
    }
    assert!(my0.eval(&[0.0], &[], 0).is_none());
    assert!(my0.eval(&[-1.0], &[], 0).is_none());
    assert!(my1.eval(&[0.0], &[], 0).is_none());
    assert!(my1.eval(&[-1.0], &[], 0).is_none());
}

#[test]
fn r017_bessel_jn_yn_wronskians_recurrences_and_jacobi_anger_identities() {
    let mjn = CompiledDslModel::compile("jn", "Y = besseljn(n, X)").unwrap();
    let myn = CompiledDslModel::compile("yn", "Y = besselyn(n, X)").unwrap();

    // 1. Cross-check Wronskian J_{n+1}(x) Y_n(x) - J_n(x) Y_{n+1}(x) = 2 / (pi * x)
    //    across x < n (Steed/Lentz CF1 + scaled Y_k) and x >= n (upward J_k).
    for &(n, x) in &[
        (0_i32, 0.5_f64),
        (1, 0.5),
        (2, 0.5),
        (5, 2.0),
        (5, 5.0),
        (5, 10.0),
        (15, 10.0),
        (15, 19.9),
        (15, 20.1),
        (30, 15.0),
        (30, 45.0),
        (80, 60.0),
        (80, 120.0),
        (150, 140.0),
        (150, 220.0),
    ] {
        let jn = mjn.eval(&[x], &[f64::from(n)], 0).unwrap();
        let jnp1 = mjn.eval(&[x], &[f64::from(n + 1)], 0).unwrap();
        let yn = myn.eval(&[x], &[f64::from(n)], 0).unwrap();
        let ynp1 = myn.eval(&[x], &[f64::from(n + 1)], 0).unwrap();
        let wronskian = jnp1 * yn - jn * ynp1;
        let expected = core::f64::consts::FRAC_2_PI / x;
        assert_close(wronskian, expected, 1e-18, 1e-12, &format!("Wronskian(J_{n}, Y_{n}) at x={x}"));
    }

    // 2. Three-term recurrence J_{n-1}(x) + J_{n+1}(x) = (2n / x) J_n(x) and similarly for Y_n
    for &x in &[0.3, 4.5, 19.5, 20.5, 55.0] {
        for &n in &[2_i32, 6, 18, 40] {
            let jm1 = mjn.eval(&[x], &[f64::from(n - 1)], 0).unwrap();
            let jn = mjn.eval(&[x], &[f64::from(n)], 0).unwrap();
            let jp1 = mjn.eval(&[x], &[f64::from(n + 1)], 0).unwrap();
            assert_close(jm1 + jp1, (2.0 * f64::from(n) / x) * jn, 1e-13, 1e-12, &format!("J recurrence n={n} x={x}"));

            let ym1 = myn.eval(&[x], &[f64::from(n - 1)], 0).unwrap();
            let yn = myn.eval(&[x], &[f64::from(n)], 0).unwrap();
            let yp1 = myn.eval(&[x], &[f64::from(n + 1)], 0).unwrap();
            assert_close(ym1 + yp1, (2.0 * f64::from(n) / x) * yn, 1e-12, 1e-12, &format!("Y recurrence n={n} x={x}"));
        }
    }

    // 3. Jacobi–Anger & Neumann identities (DLMF §10.12.1, §10.12.4, §10.23.3):
    //    - J_0(x) + 2 sum_{k>=1} J_{2k}(x) = 1
    //    - J_0(x)^2 + 2 sum_{k>=1} J_k(x)^2 = 1
    //    - cos(x) = J_0(x) + 2 sum_{k>=1} (-1)^k J_{2k}(x)
    //    - sin(x) = 2 sum_{k>=0} (-1)^k J_{2k+1}(x)
    for &x in &[0.7, 3.2, 12.5, 22.5] {
        let j0 = mjn.eval(&[x], &[0.0], 0).unwrap();
        let mut neumann_even = j0;
        let mut neumann_sq = j0 * j0;
        let mut cos_sum = j0;
        let mut sin_sum = 0.0_f64;
        for k in 1..=65_i32 {
            let jk = mjn.eval(&[x], &[f64::from(k)], 0).unwrap();
            neumann_sq += 2.0 * jk * jk;
            if k % 2 == 0 {
                neumann_even += 2.0 * jk;
                let sign = if (k / 2) % 2 == 0 { 1.0 } else { -1.0 };
                cos_sum += 2.0 * sign * jk;
            } else {
                let sign = if ((k - 1) / 2) % 2 == 0 { 1.0 } else { -1.0 };
                sin_sum += 2.0 * sign * jk;
            }
        }
        assert_close(neumann_even, 1.0, 1e-12, 1e-12, &format!("J0 + 2 sum J_{{2k}} = 1 at x={x}"));
        assert_close(neumann_sq, 1.0, 1e-12, 1e-12, &format!("J0^2 + 2 sum J_k^2 = 1 at x={x}"));
        assert_close(cos_sum, x.cos(), 1e-12, 1e-12, &format!("Jacobi-Anger cos(x) at x={x}"));
        assert_close(sin_sum, x.sin(), 1e-12, 1e-12, &format!("Jacobi-Anger sin(x) at x={x}"));
    }

    // 4. Independent Bessel integral quadrature for higher integer orders
    for &(n, x) in &[(2_i32, 1.5_f64), (5, 3.0), (12, 15.0), (25, 30.0)] {
        let nf = f64::from(n);
        let jn_quad = gauss_legendre_64(|t| (nf * t - x * t.sin()).cos(), 0.0, core::f64::consts::PI) / core::f64::consts::PI;
        let jn_val = mjn.eval(&[x], &[nf], 0).unwrap();
        assert_close(jn_val, jn_quad, 1e-13, 1e-12, &format!("J_{n}({x}) quadrature"));
    }

    // 5. Negative order, negative x, and extreme order/underflow bounds
    for &(n, x) in &[(3_i32, 2.5_f64), (4, 2.5), (7, 25.0)] {
        let pos = mjn.eval(&[x], &[f64::from(n)], 0).unwrap();
        let neg_n = mjn.eval(&[x], &[f64::from(-n)], 0).unwrap();
        let neg_x = mjn.eval(&[-x], &[f64::from(n)], 0).unwrap();
        let expected_sign = if n % 2 == 0 { 1.0 } else { -1.0 };
        assert_close(neg_n, expected_sign * pos, 1e-15, 1e-15, &format!("J_{{-{n}}}({x})"));
        assert_close(neg_x, expected_sign * pos, 1e-15, 1e-15, &format!("J_{n}(-{x})"));

        let yn_pos = myn.eval(&[x], &[f64::from(n)], 0).unwrap();
        let yn_neg = myn.eval(&[x], &[f64::from(-n)], 0).unwrap();
        assert_close(yn_neg, expected_sign * yn_pos, 1e-15, 1e-15, &format!("Y_{{-{n}}}({x})"));
    }
    assert_eq!(mjn.eval(&[1.0], &[1_000.0], 0), Some(0.0));
    assert_eq!(mjn.eval(&[1.0], &[100_000.0], 0), Some(0.0));
    assert_eq!(mjn.eval(&[1.0], &[100_001.0], 0), None);
    assert_eq!(myn.eval(&[1.0], &[500.0], 0), None);
}

#[test]
fn r017_hyp1f1_and_confluent_u_identities_integer_b_and_large_z() {
    let m_1f1 = CompiledDslModel::compile("m1f1", "Y = hyp1f1(a, b, X)").unwrap();
    let m_u = CompiledDslModel::compile("cu", "Y = confluent_u(a, b, X)").unwrap();

    // 1. Exponential, rational, and erf/erfcx identities (DLMF §13.6.7)
    for &z in &[-50.0, -2.5, 0.0, 4.0, 200.0, 500.0] {
        assert_close(m_1f1.eval(&[z], &[2.5, 2.5], 0).unwrap(), z.exp(), 1e-15, 1e-13, &format!("1F1(a,a,{z})"));
    }
    for &z in &[-1_000.0_f64, -100.0, -5.0, 0.5, 20.0] {
        let expected = if z < -700.0 { -1.0 / z } else { (z.exp() - 1.0) / z };
        assert_close(m_1f1.eval(&[z], &[1.0, 2.0], 0).unwrap(), expected, 1e-15, 1e-13, &format!("1F1(1,2,{z})"));
    }
    for &z in &[-1_000.0_f64, -50.0, -2.0, 3.0, 25.0] {
        let expected = if z < -700.0 { 2.0 * (-1.0 - z) / (z * z) } else { 2.0 * (z.exp() - 1.0 - z) / (z * z) };
        assert_close(m_1f1.eval(&[z], &[1.0, 3.0], 0).unwrap(), expected, 1e-15, 1e-12, &format!("1F1(1,3,{z})"));
    }

    let half_sqrt_pi = 0.5 * core::f64::consts::PI.sqrt();
    for &x in &[0.1_f64, 1.0, 3.0, 10.0, 30.0] {
        // 1F1(1/2; 3/2; -x^2) = (sqrt(pi) / (2x)) * erf(x) (x=30 => z=-900 tests DLMF §13.7.2)
        let expected_m = (half_sqrt_pi / x) * erf(x);
        assert_close(m_1f1.eval(&[-x * x], &[0.5, 1.5], 0).unwrap(), expected_m, 1e-15, 1e-12, &format!("1F1 erf at x={x}"));

        // U(1/2, 1/2, x^2) = sqrt(pi) * erfcx(x)
        let expected_u = core::f64::consts::PI.sqrt() * erfcx(x);
        assert_close(m_u.eval(&[x * x], &[0.5, 0.5], 0).unwrap(), expected_u, 1e-14, 1e-12, &format!("U erfcx at x={x}"));
    }

    // 2. Half-integer / spherical Bessel closed forms in sinh/cosh and 1/x (DLMF §13.6.9-10)
    for &x in &[0.3_f64, 1.5, 5.0, 20.0, 80.0] {
        // M(2, 4, 2x) = 3 * exp(x) * (x cosh x - sinh x) / x^3
        let expected_sph_m = 3.0 * x.exp() * (x * x.cosh() - x.sinh()) / (x * x * x);
        assert_close(m_1f1.eval(&[2.0 * x], &[2.0, 4.0], 0).unwrap(), expected_sph_m, 1e-14, 1e-12, &format!("M(2,4,2x) at x={x}"));
        // U(1, 2, 2x) = 1/(2x), U(2, 4, 2x) = (2 + 2x) / (2x)^3 (integer b = 2, 4!)
        assert_close(m_u.eval(&[2.0 * x], &[1.0, 2.0], 0).unwrap(), 1.0 / (2.0 * x), 1e-15, 1e-13, &format!("U(1,2,2x) at x={x}"));
        assert_close(
            m_u.eval(&[2.0 * x], &[2.0, 4.0], 0).unwrap(),
            (2.0 + 2.0 * x) / (2.0 * x).powi(3),
            1e-15,
            1e-13,
            &format!("U(2,4,2x) at x={x}"),
        );
    }

    // 2b. Non-terminating large negative z for 1F1 (DLMF §13.7.2 + contiguous recurrence DLMF §13.3.1):
    //     b(b - 1) M(a, b - 1, z) + b(1 - b - z) M(a, b, z) + z(b - a) M(a, b + 1, z) = 0
    for &(a, b) in &[(1.5_f64, 3.5_f64), (2.25, 4.5)] {
        for &z in &[-45.0_f64, -800.0, -1_500.0] {
            let m_bm1 = m_1f1.eval(&[z], &[a, b - 1.0], 0).unwrap();
            let m_b = m_1f1.eval(&[z], &[a, b], 0).unwrap();
            let m_bp1 = m_1f1.eval(&[z], &[a, b + 1.0], 0).unwrap();
            let lhs = b * (b - 1.0) * m_bm1 + b * (1.0 - b - z) * m_b;
            let rhs = -z * (b - a) * m_bp1;
            assert_close(lhs, rhs, 1e-14, 1e-12, &format!("1F1 contiguous recurrence a={a} b={b} z={z}"));
        }
    }

    // 3. Generalized Laguerre polynomials L_n^(alpha)(z) (DLMF §13.6.19)
    //    U(-n, alpha+1, z) = (-1)^n n! L_n^(alpha)(z), M(-n, alpha+1, z) = n! / (alpha+1)_n * L_n^(alpha)(z)
    for &alpha in &[-0.5, 0.0, 1.0, 2.5] {
        let b = alpha + 1.0;
        for &z in &[0.25, 2.0, 12.0, 40.0] {
            let l0 = 1.0;
            let l1 = 1.0 + alpha - z;
            let l2 = 0.5 * ((alpha + 1.0) * (alpha + 2.0) - 2.0 * (alpha + 2.0) * z + z * z);
            let l3 = ((alpha + 1.0) * (alpha + 2.0) * (alpha + 3.0) - 3.0 * (alpha + 2.0) * (alpha + 3.0) * z + 3.0 * (alpha + 3.0) * z * z
                - z * z * z)
                / 6.0;
            for &(n, l_val, fact) in &[(0.0, l0, 1.0), (1.0, l1, 1.0), (2.0, l2, 2.0), (3.0, l3, 6.0)] {
                let sign = if (n as i32) % 2 == 0 { 1.0 } else { -1.0 };
                assert_close(m_u.eval(&[z], &[-n, b], 0).unwrap(), sign * fact * l_val, 1e-12, 1e-12, &format!("U(-{n},{b},{z})"));
                let poch = if n == 0.0 {
                    1.0
                } else if n == 1.0 {
                    b
                } else if n == 2.0 {
                    b * (b + 1.0)
                } else {
                    b * (b + 1.0) * (b + 2.0)
                };
                assert_close(m_1f1.eval(&[z], &[-n, b], 0).unwrap(), (fact / poch) * l_val, 1e-12, 1e-12, &format!("1F1(-{n},{b},{z})"));
            }
        }
    }

    // 4. Integer-b, near-integer-b continuity, b <= 0 integer, large z, and Wronskian W{M, U} (DLMF §13.2.33)
    for &z in &[0.2, 1.5, 4.0, 12.0, 40.0, 200.0] {
        // Continuity across b = 2.0 (replaces the old 1e-7 perturbation jump)
        let u_lo = m_u.eval(&[z], &[1.5, 2.0 - 1e-9], 0).unwrap();
        let u_exact = m_u.eval(&[z], &[1.5, 2.0], 0).unwrap();
        let u_hi = m_u.eval(&[z], &[1.5, 2.0 + 1e-9], 0).unwrap();
        assert_close(u_lo, u_exact, 1e-10, 1e-7, &format!("U continuity left of b=2 at z={z}"));
        assert_close(u_hi, u_exact, 1e-10, 1e-7, &format!("U continuity right of b=2 at z={z}"));

        // Non-positive integer b (b = 0, b = -1) via DLMF §13.2.40: U(a, 1-m, z) = z^m U(a+m, 1+m, z)
        assert_close(m_u.eval(&[z], &[1.5, 0.0], 0).unwrap(), z * m_u.eval(&[z], &[2.5, 2.0], 0).unwrap(), 1e-12, 1e-11, &format!("U b=0 at z={z}"));
        assert_close(
            m_u.eval(&[z], &[1.5, -1.0], 0).unwrap(),
            z * z * m_u.eval(&[z], &[3.5, 3.0], 0).unwrap(),
            1e-12,
            1e-11,
            &format!("U b=-1 at z={z}"),
        );
    }

    // Wronskian W{M(a,b,z), U(a,b,z)} = M(a,b,z)*(-a*U(a+1,b+1,z)) - (a/b)*M(a+1,b+1,z)*U(a,b,z)
    //                                  = -(Gamma(b) / Gamma(a)) * z^(-b) * exp(z)
    for &(a, b) in &[(1.25_f64, 2.0_f64), (0.75, 1.0), (2.3, 3.4), (-0.4, 2.0)] {
        for &z in &[0.3, 1.8, 6.5] {
            let m0 = m_1f1.eval(&[z], &[a, b], 0).unwrap();
            let mp = (a / b) * m_1f1.eval(&[z], &[a + 1.0, b + 1.0], 0).unwrap();
            let u0 = m_u.eval(&[z], &[a, b], 0).unwrap();
            let up = -a * m_u.eval(&[z], &[a + 1.0, b + 1.0], 0).unwrap();
            let wronskian = m0 * up - mp * u0;
            let expected = -(gamma(b).unwrap() / gamma(a).unwrap()) * z.powf(-b) * z.exp();
            assert_close(wronskian, expected, 1e-12, 1e-11, &format!("Wronskian(M, U) a={a} b={b} z={z}"));
        }
    }

    // 5. Domain guards
    assert!(m_u.eval(&[0.0], &[1.0, 2.0], 0).is_none());
    assert!(m_u.eval(&[-1.0], &[1.0, 2.0], 0).is_none());
    assert!(m_1f1.eval(&[1.0], &[1.0, 0.0], 0).is_none());
    assert!(m_1f1.eval(&[1.0], &[1.0, -2.0], 0).is_none());
}
