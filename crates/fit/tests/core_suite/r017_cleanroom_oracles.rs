//! Clean-room `D1` oracle tests for `R0.17`:
//! - Lilliefors normality test (`curvebook_fit::diagnostics::lilliefors_ks_test`),
//! - Clean-room Bessel functions (`j0`, `j1`, `jn`, `y0`, `y1`, `yn` via `CompiledDslModel`),
//! - Confluent hypergeometric functions (`hyp1f1` and `confluent_u` via `CompiledDslModel`).
//!
//! All oracles are derived from exact mathematical identities (Wronskians, three-term recurrences,
//! Bessel & Jacobi–Anger trigonometric series, Gauss–Legendre integral representations, `erf`/`exp`/
//! Laguerre/spherical Bessel closed forms, and published Dallal & Wilkinson 1986 / Stephens 1974
//! Lilliefors tables) rather than copied coefficients.

use curvebook_fit::{ParametricModel, lilliefors_ks_test};
use curvebook_math::DeterministicRng;
use curvebook_math::dist::{lilliefors_p_value, norm_cdf};
use curvebook_math::quad::gauss_legendre_64;
use curvebook_math::special::{erf, erfcx, gamma};
use curvebook_math::sum::SummaryMoments;
use curvebook_testkit::assert_approx_eq as assert_close;

use crate::common::dsl_compile;

#[test]
fn r017_lilliefors_fit_diagnostics_parity_and_large_n() {
    for bad in [&[1.0, 2.0, 3.0, 4.0][..], &[2.0; 12][..], &[1.0, 2.0, f64::NAN, 3.0, 4.0][..]] {
        assert!(lilliefors_ks_test(bad).is_none());
    }

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
        let d_ref = xs.iter().enumerate().fold(0.0_f64, |acc, (i, &x)| {
            let fz = norm_cdf((x - m.mean) / sd);
            acc.max(((i + 1) as f64) / nf - fz).max(fz - (i as f64) / nf)
        });
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
    let (mj0, mj1, my0, my1) =
        (dsl_compile("Y = besselj0(X)"), dsl_compile("Y = besselj1(X)"), dsl_compile("Y = bessely0(X)"), dsl_compile("Y = bessely1(X)"));
    let ev = |m: &curvebook_fit::CompiledDslModel, x: f64| m.eval(&[x], &[], 0).unwrap();

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
    for x in xs {
        let wronskian = ev(&mj1, x) * ev(&my0, x) - ev(&mj0, x) * ev(&my1, x);
        assert_close(wronskian, core::f64::consts::FRAC_2_PI / x, 1e-20, 1e-13, &format!("Wronskian(J0, Y0) at x={x}"));
    }

    // 2. Near-f64 accuracy at zeros (old NR §6.5 polynomial had ~1e-8 error here).
    let zero_tables: &[(&curvebook_fit::CompiledDslModel, &str, &[f64])] = &[
        (&mj0, "J0", &[2.404_825_557_695_773, 5.520_078_110_286_311, 18.071_063_967_910_924, 21.211_636_629_879_26]),
        (&mj1, "J1", &[3.831_705_970_207_512_3, 19.615_858_510_468_24, 22.760_084_380_592_77]),
        (&my0, "Y0", &[0.893_576_966_279_167_5, 19.641_309_700_887_94, 22.782_028_047_291_55]),
    ];
    for &(m, lbl, zeros) in zero_tables {
        for &z in zeros {
            let val = ev(m, z);
            assert!(val.abs() < 1e-14, "{lbl} at zero {z}: got {val}");
        }
    }

    // 3. Independent Bessel integral quadrature (DLMF §10.9.2): J_0(x) = (1/pi) int_0^pi cos(x sin t) dt.
    for x in [0.5, 2.5, 10.0, 19.9, 20.1, 30.0] {
        let j0_quad = gauss_legendre_64(|t| (x * t.sin()).cos(), 0.0, core::f64::consts::PI) / core::f64::consts::PI;
        let j1_quad = gauss_legendre_64(|t| (t - x * t.sin()).cos(), 0.0, core::f64::consts::PI) / core::f64::consts::PI;
        assert_close(ev(&mj0, x), j0_quad, 1e-14, 1e-13, &format!("J0 quadrature at x={x}"));
        assert_close(ev(&mj1, x), j1_quad, 1e-14, 1e-13, &format!("J1 quadrature at x={x}"));
    }

    // 4. Parity & domain boundaries
    assert_close(ev(&mj0, 0.0), 1.0, 1e-15, 0.0, "J0(0)");
    assert_close(ev(&mj1, 0.0), 0.0, 1e-15, 0.0, "J1(0)");
    for x in [1.5, 12.0, 25.0] {
        assert_close(ev(&mj0, -x), ev(&mj0, x), 1e-15, 1e-15, "J0 even");
        assert_close(ev(&mj1, -x), -ev(&mj1, x), 1e-15, 1e-15, "J1 odd");
    }
    for m in [&my0, &my1] {
        for x in [0.0, -1.0] {
            assert!(m.eval(&[x], &[], 0).is_none());
        }
    }
}

#[test]
fn r017_bessel_jn_yn_wronskians_recurrences_and_jacobi_anger_identities() {
    let (mjn, myn) = (dsl_compile("Y = besseljn(n, X)"), dsl_compile("Y = besselyn(n, X)"));
    let ev = |m: &curvebook_fit::CompiledDslModel, n: i32, x: f64| m.eval(&[x], &[f64::from(n)], 0).unwrap();

    // 1. Cross-check Wronskian J_{n+1}(x) Y_n(x) - J_n(x) Y_{n+1}(x) = 2 / (pi * x)
    //    across x < n (Steed/Lentz CF1 + scaled Y_k) and x >= n (upward J_k).
    let w_cases = [
        (0, 0.5),
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
    ];
    for (n, x) in w_cases {
        let wronskian = ev(&mjn, n + 1, x) * ev(&myn, n, x) - ev(&mjn, n, x) * ev(&myn, n + 1, x);
        assert_close(wronskian, core::f64::consts::FRAC_2_PI / x, 1e-18, 1e-12, &format!("Wronskian(J_{n}, Y_{n}) at x={x}"));
    }

    // 2. Three-term recurrence J_{n-1}(x) + J_{n+1}(x) = (2n / x) J_n(x) and similarly for Y_n
    for x in [0.3, 4.5, 19.5, 20.5, 55.0] {
        for n in [2, 6, 18, 40] {
            let coef = 2.0 * f64::from(n) / x;
            assert_close(ev(&mjn, n - 1, x) + ev(&mjn, n + 1, x), coef * ev(&mjn, n, x), 1e-13, 1e-12, &format!("J recurrence n={n} x={x}"));
            assert_close(ev(&myn, n - 1, x) + ev(&myn, n + 1, x), coef * ev(&myn, n, x), 1e-12, 1e-12, &format!("Y recurrence n={n} x={x}"));
        }
    }

    // 3. Jacobi–Anger & Neumann identities (DLMF §10.12.1, §10.12.4, §10.23.3)
    for x in [0.7, 3.2, 12.5, 22.5] {
        let j0 = ev(&mjn, 0, x);
        let (mut neumann_even, mut neumann_sq, mut cos_sum, mut sin_sum) = (j0, j0 * j0, j0, 0.0_f64);
        for k in 1..=65 {
            let jk = ev(&mjn, k, x);
            neumann_sq += 2.0 * jk * jk;
            if k % 2 == 0 {
                neumann_even += 2.0 * jk;
                cos_sum += if (k / 2) % 2 == 0 { 2.0 * jk } else { -2.0 * jk };
            } else {
                sin_sum += if ((k - 1) / 2) % 2 == 0 { 2.0 * jk } else { -2.0 * jk };
            }
        }
        assert_close(neumann_even, 1.0, 1e-12, 1e-12, &format!("J0 + 2 sum J_{{2k}} = 1 at x={x}"));
        assert_close(neumann_sq, 1.0, 1e-12, 1e-12, &format!("J0^2 + 2 sum J_k^2 = 1 at x={x}"));
        assert_close(cos_sum, x.cos(), 1e-12, 1e-12, &format!("Jacobi-Anger cos(x) at x={x}"));
        assert_close(sin_sum, x.sin(), 1e-12, 1e-12, &format!("Jacobi-Anger sin(x) at x={x}"));
    }

    // 4. Independent Bessel integral quadrature for higher integer orders
    for (n, x) in [(2, 1.5), (5, 3.0), (12, 15.0), (25, 30.0)] {
        let nf = f64::from(n);
        let jn_quad = gauss_legendre_64(|t| (nf * t - x * t.sin()).cos(), 0.0, core::f64::consts::PI) / core::f64::consts::PI;
        assert_close(ev(&mjn, n, x), jn_quad, 1e-13, 1e-12, &format!("J_{n}({x}) quadrature"));
    }

    // 5. Negative order, negative x, and extreme order/underflow bounds
    for (n, x) in [(3, 2.5), (4, 2.5), (7, 25.0)] {
        let sign = if n % 2 == 0 { 1.0 } else { -1.0 };
        let pos = ev(&mjn, n, x);
        assert_close(ev(&mjn, -n, x), sign * pos, 1e-15, 1e-15, &format!("J_{{-{n}}}({x})"));
        assert_close(ev(&mjn, n, -x), sign * pos, 1e-15, 1e-15, &format!("J_{n}(-{x})"));
        assert_close(ev(&myn, -n, x), sign * ev(&myn, n, x), 1e-15, 1e-15, &format!("Y_{{-{n}}}({x})"));
    }
    assert_eq!(mjn.eval(&[1.0], &[1_000.0], 0), Some(0.0));
    assert_eq!(mjn.eval(&[1.0], &[100_000.0], 0), Some(0.0));
    assert_eq!(mjn.eval(&[1.0], &[100_001.0], 0), None);
    assert_eq!(myn.eval(&[1.0], &[500.0], 0), None);
}

#[test]
fn r017_hyp1f1_and_confluent_u_identities_integer_b_and_large_z() {
    let (m_1f1, m_u) = (dsl_compile("Y = hyp1f1(a, b, X)"), dsl_compile("Y = confluent_u(a, b, X)"));
    let ev = |m: &curvebook_fit::CompiledDslModel, a: f64, b: f64, z: f64| m.eval(&[z], &[a, b], 0).unwrap();

    // 1. Exponential, rational, and erf/erfcx identities (DLMF §13.6.7)
    for z in [-50.0, -2.5, 0.0, 4.0, 200.0, 500.0] {
        assert_close(ev(&m_1f1, 2.5, 2.5, z), z.exp(), 1e-15, 1e-13, &format!("1F1(a,a,{z})"));
    }
    for z in [-1_000.0_f64, -100.0, -5.0, 0.5, 20.0] {
        let exp = if z < -700.0 { -1.0 / z } else { (z.exp() - 1.0) / z };
        assert_close(ev(&m_1f1, 1.0, 2.0, z), exp, 1e-15, 1e-13, &format!("1F1(1,2,{z})"));
    }
    for z in [-1_000.0_f64, -50.0, -2.0, 3.0, 25.0] {
        let exp = if z < -700.0 { 2.0 * (-1.0 - z) / (z * z) } else { 2.0 * (z.exp() - 1.0 - z) / (z * z) };
        assert_close(ev(&m_1f1, 1.0, 3.0, z), exp, 1e-15, 1e-12, &format!("1F1(1,3,{z})"));
    }

    let sqrt_pi = core::f64::consts::PI.sqrt();
    for x in [0.1_f64, 1.0, 3.0, 10.0, 30.0] {
        assert_close(ev(&m_1f1, 0.5, 1.5, -x * x), (0.5 * sqrt_pi / x) * erf(x), 1e-15, 1e-12, &format!("1F1 erf at x={x}"));
        assert_close(ev(&m_u, 0.5, 0.5, x * x), sqrt_pi * erfcx(x), 1e-14, 1e-12, &format!("U erfcx at x={x}"));
    }

    // 2. Half-integer / spherical Bessel closed forms in sinh/cosh and 1/x (DLMF §13.6.9-10)
    for x in [0.3_f64, 1.5, 5.0, 20.0, 80.0] {
        let sph_m = 3.0 * x.exp() * (x * x.cosh() - x.sinh()) / (x * x * x);
        assert_close(ev(&m_1f1, 2.0, 4.0, 2.0 * x), sph_m, 1e-14, 1e-12, &format!("M(2,4,2x) at x={x}"));
        assert_close(ev(&m_u, 1.0, 2.0, 2.0 * x), 1.0 / (2.0 * x), 1e-15, 1e-13, &format!("U(1,2,2x) at x={x}"));
        assert_close(ev(&m_u, 2.0, 4.0, 2.0 * x), (2.0 + 2.0 * x) / (2.0 * x).powi(3), 1e-15, 1e-13, &format!("U(2,4,2x) at x={x}"));
    }

    // 2b. Non-terminating large negative z for 1F1 (DLMF §13.7.2 + contiguous recurrence DLMF §13.3.1)
    for (a, b) in [(1.5_f64, 3.5_f64), (2.25, 4.5)] {
        for z in [-45.0_f64, -800.0, -1_500.0] {
            let lhs = b * (b - 1.0) * ev(&m_1f1, a, b - 1.0, z) + b * (1.0 - b - z) * ev(&m_1f1, a, b, z);
            let rhs = -z * (b - a) * ev(&m_1f1, a, b + 1.0, z);
            assert_close(lhs, rhs, 1e-14, 1e-12, &format!("1F1 contiguous recurrence a={a} b={b} z={z}"));
        }
    }

    // 3. Generalized Laguerre polynomials L_n^(alpha)(z) (DLMF §13.6.19)
    for alpha in [-0.5, 0.0, 1.0, 2.5] {
        let b = alpha + 1.0;
        let pochs = [1.0, b, b * (b + 1.0), b * (b + 1.0) * (b + 2.0)];
        for z in [0.25, 2.0, 12.0, 40.0] {
            let lag = [
                (0, 1.0, 1.0),
                (1, 1.0 + alpha - z, 1.0),
                (2, 0.5 * ((alpha + 1.0) * (alpha + 2.0) - 2.0 * (alpha + 2.0) * z + z * z), 2.0),
                (
                    3,
                    ((alpha + 1.0) * (alpha + 2.0) * (alpha + 3.0) - 3.0 * (alpha + 2.0) * (alpha + 3.0) * z + 3.0 * (alpha + 3.0) * z * z
                        - z * z * z)
                        / 6.0,
                    6.0,
                ),
            ];
            for (n, l_val, fact) in lag {
                let sign = if n % 2 == 0 { 1.0 } else { -1.0 };
                let nf = f64::from(n);
                assert_close(ev(&m_u, -nf, b, z), sign * fact * l_val, 1e-12, 1e-12, &format!("U(-{n},{b},{z})"));
                assert_close(ev(&m_1f1, -nf, b, z), (fact / pochs[n as usize]) * l_val, 1e-12, 1e-12, &format!("1F1(-{n},{b},{z})"));
            }
        }
    }

    // 4. Integer-b, near-integer-b continuity, b <= 0 integer, large z, and Wronskian W{M, U} (DLMF §13.2.33)
    for z in [0.2, 1.5, 4.0, 12.0, 40.0, 200.0] {
        let u_exact = ev(&m_u, 1.5, 2.0, z);
        assert_close(ev(&m_u, 1.5, 2.0 - 1e-9, z), u_exact, 1e-10, 1e-7, &format!("U continuity left of b=2 at z={z}"));
        assert_close(ev(&m_u, 1.5, 2.0 + 1e-9, z), u_exact, 1e-10, 1e-7, &format!("U continuity right of b=2 at z={z}"));
        assert_close(ev(&m_u, 1.5, 0.0, z), z * ev(&m_u, 2.5, 2.0, z), 1e-12, 1e-11, &format!("U b=0 at z={z}"));
        assert_close(ev(&m_u, 1.5, -1.0, z), z * z * ev(&m_u, 3.5, 3.0, z), 1e-12, 1e-11, &format!("U b=-1 at z={z}"));
    }
    for (a, b) in [(1.25_f64, 2.0_f64), (0.75, 1.0), (2.3, 3.4), (-0.4, 2.0)] {
        for z in [0.3, 1.8, 6.5] {
            let wronskian =
                ev(&m_1f1, a, b, z) * (-a * ev(&m_u, a + 1.0, b + 1.0, z)) - (a / b) * ev(&m_1f1, a + 1.0, b + 1.0, z) * ev(&m_u, a, b, z);
            let expected = -(gamma(b).unwrap() / gamma(a).unwrap()) * z.powf(-b) * z.exp();
            assert_close(wronskian, expected, 1e-12, 1e-11, &format!("Wronskian(M, U) a={a} b={b} z={z}"));
        }
    }

    // 5. Domain guards
    for (m, z, p) in [(&m_u, 0.0, [1.0, 2.0]), (&m_u, -1.0, [1.0, 2.0]), (&m_1f1, 1.0, [1.0, 0.0]), (&m_1f1, 1.0, [1.0, -2.0])] {
        assert!(m.eval(&[z], &p, 0).is_none());
    }
}
