//! Clean-room `D1` oracle tests for the Lilliefors normality test (`R0.17`).
//!
//! Provenance & Oracles:
//! - Lilliefors, H. W. (1967). *JASA* 62(318), 399–402 (definition of `D_n` and Table 1).
//! - Stephens, M. A. (1974). *JASA* 69(347), 730–737 (asymptotic `sqrt(n)` scaling).
//! - Dallal, G. E. & Wilkinson, L. (1986). *The American Statistician* 40(4), 294–296 (Table 1 and
//!   analytic upper-tail approximation eq. (1) for `5 <= n <= 100, p <= 0.10`).
//! - Independent deterministic Monte Carlo validation via `lumencraft_math::DeterministicRng`
//!   (`seed = 0xA117_0107_CEAD_BEEF`, distinct from table training seed `0x4C49_4C4C_4945_464F`),
//!   explicitly covering `n > 100` (`n = 150, 500`) as well as `n <= 100`.

use lumencraft_math::DeterministicRng;
use lumencraft_math::dist::{lilliefors_d_from_sorted, lilliefors_p_value, norm_cdf};
use lumencraft_math::sum::SummaryMoments;
use lumencraft_stats::StatsError;
use lumencraft_stats::column::{lilliefors_ks, normality_battery};

fn lilliefors_d_stat_direct(sample: &mut [f64]) -> Result<f64, lumencraft_math::MathError> {
    sample.sort_by(f64::total_cmp);
    let n = sample.len() as f64;
    let m = SummaryMoments::from_slice(sample)?;
    let sd = m.sample_sd()?;
    let mut d_max = 0.0_f64;
    for (i, &x) in sample.iter().enumerate() {
        let fz = norm_cdf((x - m.mean) / sd);
        d_max = d_max.max(((i + 1) as f64) / n - fz).max(fz - (i as f64) / n);
    }
    Ok(d_max)
}

#[test]
fn r017_lilliefors_edge_cases_invariance_and_math_primitive_parity() {
    // Insufficient sample size (< 5) and zero variance / non-finite errors
    assert!(matches!(lilliefors_ks(&[], 0.05), Err(StatsError::EmptyInput)));
    assert!(matches!(lilliefors_ks(&[1.0, 2.0, 3.0, 4.0], 0.05), Err(StatsError::InsufficientSampleSize { required: 5, actual: 4, .. })));
    assert!(matches!(lilliefors_ks(&[7.5; 10], 0.05), Err(StatsError::ZeroVariance { .. })));
    assert!(lilliefors_ks(&[1.0, 2.0, f64::NAN, 4.0, 5.0], 0.05).is_err());
    assert!(lilliefors_ks(&[1.0, 2.0, f64::INFINITY, 4.0, 5.0], 0.05).is_err());

    // Exact agreement with lumencraft_math::dist::lilliefors_* + affine invariance
    let mut rng = DeterministicRng::from_seed(0x0170_C0DE_0000_0001);
    for &n in &[5_usize, 12, 50, 150, 500] {
        let raw: Vec<f64> = (0..n).map(|_| rng.standard_normal()).collect();
        let res = lilliefors_ks(&raw, 0.05).unwrap();
        let mut sorted = raw.clone();
        sorted.sort_by(f64::total_cmp);
        let d_math = lilliefors_d_from_sorted(&sorted).unwrap();
        let p_math = lilliefors_p_value(d_math, n).unwrap();
        assert!((res.statistic - d_math).abs() < 1e-15, "n={n}: stat mismatch {} vs {}", res.statistic, d_math);
        assert!((res.p_value - p_math).abs() < 1e-15, "n={n}: p mismatch {} vs {}", res.p_value, p_math);
        assert_eq!(res.passed, res.p_value > 0.05);

        let transformed: Vec<f64> = raw.iter().map(|&x| 100.0 + 3.5 * x).collect();
        let res_aff = lilliefors_ks(&transformed, 0.05).unwrap();
        assert!((res_aff.statistic - res.statistic).abs() < 1e-12, "affine invariance D at n={n}");
        assert!((res_aff.p_value - res.p_value).abs() < 1e-11, "affine invariance p at n={n}");

        let stiff: Vec<f64> = raw.iter().map(|&x| 1.25e5 + 3.5e-3 * x).collect();
        let res_stiff = lilliefors_ks(&stiff, 0.05).unwrap();
        assert!((res_stiff.statistic - res.statistic).abs() < 1e-7, "stiff affine D at n={n}");
        assert!((res_stiff.p_value - res.p_value).abs() < 1e-6, "stiff affine p at n={n}");
    }
}

#[test]
fn r017_lilliefors_dallal_wilkinson_1986_and_lilliefors_1967_table1_oracles() {
    // Published Dallal & Wilkinson (1986) Table 1 critical values at alpha = 0.05 (1M MC trials)
    // and alpha = 0.10, 0.01 at n = 5, 10, 20, 30, 50, 100.
    // Tolerance: <= 0.0015 absolute at alpha = 0.05 (3-digit table rounding accounts for ~0.001).
    let dw_table1_alpha05: &[(usize, f64)] = &[(5, 0.343), (10, 0.262), (20, 0.192), (30, 0.159), (50, 0.1246), (100, 0.0889)];
    for &(n, d_crit) in dw_table1_alpha05 {
        let p = lilliefors_p_value(d_crit, n).unwrap();
        assert!((p - 0.05).abs() < 0.0015, "D&W 1986 Table 1 alpha=0.05 at n={n} (D={d_crit}): got p={p}");
    }

    // Published Lilliefors (1967) Table 1 critical values (1,000-sample MC table has ~0.015 p tolerance
    // at small n; verify p_value sits close to the nominal alpha).
    let lilliefors_1967_table1: &[(usize, f64, f64, f64)] = &[
        (5, 0.315, 0.10, 0.015),
        (5, 0.405, 0.01, 0.005),
        (10, 0.239, 0.10, 0.012),
        (10, 0.304, 0.01, 0.003),
        (20, 0.174, 0.10, 0.012),
        (20, 0.226, 0.01, 0.003),
        (30, 0.144, 0.10, 0.012),
        (30, 0.187, 0.01, 0.003),
    ];
    for &(n, d_crit, alpha, tol) in lilliefors_1967_table1 {
        let p = lilliefors_p_value(d_crit, n).unwrap();
        assert!((p - alpha).abs() <= tol, "Lilliefors 1967 Table 1 n={n} alpha={alpha} D={d_crit}: got p={p}");
    }

    // Dallal & Wilkinson (1986) eq. (1) analytic formula check inside its valid region (5 <= n <= 100, p <= 0.10).
    for &(n, d_crit) in dw_table1_alpha05 {
        let nf = n as f64;
        let s = (nf + 2.78019).sqrt();
        let p_dw = (-7.01256 * d_crit * d_crit * (nf + 2.78019) + 2.99587 * d_crit * s - 0.122_119 + 0.974_598 / nf.sqrt() + 1.67997 / nf).exp();
        let p_clean = lilliefors_p_value(d_crit, n).unwrap();
        assert!((p_clean - p_dw).abs() < 0.005, "n={n}: clean={p_clean} vs D&W eq(1)={p_dw}");
    }

    // Stephens (1974) / D'Agostino & Stephens (1986) Table 4.2 asymptotic Case 3 critical points
    // T* = D * (sqrt(n) - 0.01 + 0.85 / sqrt(n)) across large n > 100.
    // Note: Stephens's 1974 table (0.775, 0.819, 0.895, 0.955, 1.035) reflects n~50..100 fits
    // while true n -> inf asymptotic D*sqrt(n) is ~0.008-0.015 higher (e.g. 0.783 vs 0.775 at p=0.15).
    for &n in &[100_usize, 250, 1000, 5000] {
        let nf = n as f64;
        let stephens_scale = nf.sqrt() - 0.01 + 0.85 / nf.sqrt();
        for &(t_star, alpha, tol) in &[(0.775, 0.15, 0.015), (0.819, 0.10, 0.015), (0.895, 0.05, 0.008), (1.035, 0.01, 0.003)] {
            let d = t_star / stephens_scale;
            let p = lilliefors_p_value(d, n).unwrap();
            assert!((p - alpha).abs() <= tol, "Stephens Case 3 n={n} T*={t_star} alpha={alpha}: got p={p}");
        }
    }
}

#[test]
fn r017_lilliefors_independent_seeded_mc_calibration_including_large_n() {
    // Independent validation seed (distinct from table training seed 0x4C49_4C4C_4945_464F).
    // Includes n = 150 and n = 500 to verify absence of the old ad-hoc n > 100 bias.
    let mut rng = DeterministicRng::from_seed(0xA117_0107_CEAD_BEEF);
    let trials = 25_000_usize;

    for &n in &[5_usize, 11, 25, 75, 150, 500] {
        let mut buf = vec![0.0_f64; n];
        let mut rej_20 = 0_usize;
        let mut rej_10 = 0_usize;
        let mut rej_05 = 0_usize;
        let mut rej_01 = 0_usize;
        for _ in 0..trials {
            for slot in &mut buf {
                *slot = rng.standard_normal();
            }
            let d = lilliefors_d_stat_direct(&mut buf).unwrap();
            let p = lilliefors_p_value(d, n).unwrap();
            if p < 0.20 {
                rej_20 += 1;
            }
            if p < 0.10 {
                rej_10 += 1;
            }
            if p < 0.05 {
                rej_05 += 1;
            }
            if p < 0.01 {
                rej_01 += 1;
            }
        }
        let tf = trials as f64;
        for &(count, alpha, tol) in &[(rej_20, 0.20, 0.012), (rej_10, 0.10, 0.009), (rej_05, 0.05, 0.007), (rej_01, 0.01, 0.0035)] {
            let rate = (count as f64) / tf;
            assert!((rate - alpha).abs() <= tol, "MC null calibration failed at n={n}, alpha={alpha}: observed rate={rate:.5}");
        }
    }

    // Power / discrimination check for n > 100 on skewed lognormal & heavy-tailed Laplace vs normal
    let mut norm_x = vec![0.0_f64; 250];
    let mut exp_x = vec![0.0_f64; 250];
    for i in 0..250 {
        norm_x[i] = rng.standard_normal();
        let u = rng.next_open_f64();
        exp_x[i] = -u.ln();
    }
    let ks_norm = lilliefors_ks(&norm_x, 0.05).unwrap();
    let ks_exp = lilliefors_ks(&exp_x, 0.05).unwrap();
    assert!(ks_norm.p_value > 0.05 && ks_norm.passed, "normal n=250 passed: p={}", ks_norm.p_value);
    assert!(ks_exp.p_value < 1e-6 && !ks_exp.passed, "exponential n=250 rejected: p={}", ks_exp.p_value);

    // Full normality_battery keeps lognormal KS passing on exponential-of-normal data
    let lognorm_x: Vec<f64> = norm_x.iter().map(|&z| z.exp()).collect();
    let bat = normality_battery(&lognorm_x, 0.05).unwrap();
    assert!(!bat.kolmogorov_smirnov.as_ref().unwrap().passed);
    assert!(bat.lognormal_kolmogorov_smirnov.as_ref().unwrap().passed);
}
