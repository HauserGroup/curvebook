//! Targeted regression tests for foundation audit fixes in `curvebook-math` (#64, #65, #82).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_math::pvalue::{p_value_stars, p_value_stars_at_alpha};
use curvebook_math::{DenseMatrix, DeterministicRng, PValueFormat, qr_col_pivoted, solve_linear_least_squares, svd};

fn assert_svd_invariants(a: &DenseMatrix, expected_rank: usize, label: &str) {
    let s = svd(a).unwrap_or_else(|e| panic!("{label}: svd failed: {e}"));
    let k = a.nrows().min(a.ncols());
    assert_eq!(s.rank(), expected_rank, "{label}: unexpected rank (sigmas={:?})", s.singular_values);

    let utu = s.u.transpose().matmul(&s.u).unwrap();
    let vtv = s.v.transpose().matmul(&s.v).unwrap();
    for i in 0..k {
        for j in 0..k {
            let exp = if i == j { 1.0 } else { 0.0 };
            assert!((utu.get(i, j).unwrap() - exp).abs() < 1e-12, "{label}: U^T U [{i},{j}] = {}", utu.get(i, j).unwrap());
            assert!((vtv.get(i, j).unwrap() - exp).abs() < 1e-12, "{label}: V^T V [{i},{j}] = {}", vtv.get(i, j).unwrap());
        }
    }

    let mut u_sigma = s.u.clone();
    for r in 0..a.nrows() {
        for (j, &sj) in s.singular_values.iter().enumerate().take(k) {
            u_sigma.set(r, j, u_sigma.get(r, j).unwrap() * sj).unwrap();
        }
    }
    let recon = u_sigma.matmul(&s.v.transpose()).unwrap();
    let scale = a.frobenius_norm().max(1.0);
    for r in 0..a.nrows() {
        for c in 0..a.ncols() {
            assert!(
                (recon.get(r, c).unwrap() - a.get(r, c).unwrap()).abs() < 1e-12 * scale,
                "{label}: recon[{r},{c}]={} vs actual={}",
                recon.get(r, c).unwrap(),
                a.get(r, c).unwrap()
            );
        }
    }
}

fn assert_aliased_zeroed(sol_coeffs: &[f64], cov: &DenseMatrix, aliased: &[usize], p: usize) {
    for &al in aliased {
        assert_eq!(sol_coeffs.get(al).copied(), Some(0.0));
        for k in 0..p {
            assert_eq!(cov.get(al, k).unwrap(), 0.0);
            assert_eq!(cov.get(k, al).unwrap(), 0.0);
        }
    }
}

#[test]
fn audit_64_duplicate_and_scaled_columns_match_basic_qr() {
    let xs: Vec<f64> = (0..10).map(|i| 0.1 + 0.5 * (i as f64)).collect();
    let ys: Vec<f64> = xs.iter().enumerate().map(|(i, &x)| 1.0 + 2.0 * x + 0.01 * ((7 * i) as f64).sin()).collect();

    let x_ref = DenseMatrix::from_row_major(10, 2, xs.iter().flat_map(|&x| [1.0, x]).collect()).unwrap();
    let (sol_ref, cov_ref) = solve_linear_least_squares(&x_ref, &ys).unwrap();
    assert_eq!(sol_ref.rank, 2);

    for scale_dup in [1.0_f64, 1000.0, 1e-4] {
        let x_dup = DenseMatrix::from_row_major(10, 3, xs.iter().flat_map(|&x| [1.0, x, scale_dup * x]).collect()).unwrap();
        let (sol, cov) = solve_linear_least_squares(&x_dup, &ys).unwrap();

        let mut x_scaled = DenseMatrix::zeros(10, 3).unwrap();
        let mut scales = [1.0_f64; 3];
        for (j, slot) in scales.iter_mut().enumerate() {
            let nrm = x_dup.col_norm2(j);
            *slot = nrm;
            for r in 0..10 {
                x_scaled.set(r, j, x_dup.get(r, j).unwrap() / nrm).unwrap();
            }
        }
        let basic_scaled = qr_col_pivoted(&x_scaled).unwrap().solve_basic(&ys).unwrap();

        assert_eq!(sol.rank, 2);
        assert_eq!(sol.aliased_columns, vec![2], "scale={scale_dup}: expected col 2 aliased");
        assert_eq!(sol.df_residual, 8);
        assert!((sol.rss - sol_ref.rss).abs() < 1e-12);
        assert_aliased_zeroed(&sol.coefficients, &cov, &[2], 3);
        assert!((sol.coefficients[0] - sol_ref.coefficients[0]).abs() < 1e-12);
        assert!((sol.coefficients[1] - sol_ref.coefficients[1]).abs() < 1e-12);
        for (j, &sj) in scales.iter().enumerate() {
            assert!((sol.coefficients[j] - basic_scaled[j] / sj).abs() < 1e-12);
        }
        if (scale_dup - 1.0).abs() == 0.0 {
            let basic_raw = qr_col_pivoted(&x_dup).unwrap().solve_basic(&ys).unwrap();
            for j in 0..3 {
                assert!((sol.coefficients[j] - basic_raw[j]).abs() < 1e-12);
            }
        }
        for i in 0..2 {
            for j in 0..2 {
                assert!((cov.get(i, j).unwrap() - cov_ref.get(i, j).unwrap()).abs() < 1e-12);
            }
        }
    }
}

#[test]
fn audit_64_effect_coded_six_by_six_matches_unhalved_basic() {
    let a_rows = [[1.0_f64, 1.0, 0.0], [1.0, 1.0, 0.0], [1.0, 0.0, 1.0], [1.0, 0.0, 1.0], [1.0, -1.0, -1.0], [1.0, -1.0, -1.0]];
    let y_eff = [10.0_f64, 11.0, 14.0, 15.5, 20.0, 21.5];
    let x_a = DenseMatrix::from_row_major(6, 3, a_rows.iter().flat_map(|r| r.iter().copied()).collect()).unwrap();
    let (sol_a, cov_a) = solve_linear_least_squares(&x_a, &y_eff).unwrap();

    let x_aa = DenseMatrix::from_row_major(6, 6, a_rows.iter().flat_map(|r| r.iter().chain(r.iter()).copied()).collect()).unwrap();
    let (sol_aa, cov_aa) = solve_linear_least_squares(&x_aa, &y_eff).unwrap();
    assert_eq!(sol_aa.rank, 3);
    assert_eq!(sol_aa.aliased_columns.len(), 3);
    assert_eq!(sol_aa.df_residual, 3);
    assert!((sol_aa.rss - sol_a.rss).abs() < 1e-12);
    assert_aliased_zeroed(&sol_aa.coefficients, &cov_aa, &sol_aa.aliased_columns, 6);
    assert!((sol_aa.coefficients[0] - sol_a.coefficients[0]).abs() < 1e-12);
    assert!((cov_aa.get(0, 0).unwrap() - cov_a.get(0, 0).unwrap()).abs() < 1e-12);
}

#[test]
fn audit_64_zero_and_underdetermined_zeroes_aliased_columns() {
    let zero = DenseMatrix::zeros(4, 3).unwrap();
    let (sol_z, cov_z) = solve_linear_least_squares(&zero, &[1.0, 2.0, 3.0, 4.0]).unwrap();
    assert_eq!(sol_z.rank, 0);
    assert_eq!(sol_z.aliased_columns, vec![0, 1, 2]);
    assert_eq!(sol_z.coefficients, vec![0.0, 0.0, 0.0]);
    assert_eq!(cov_z.frobenius_norm(), 0.0);

    let wide = DenseMatrix::from_row_major(
        3,
        5,
        vec![
            1.0, 0.0, 2.0, 1.0, -1.0, //
            0.0, 1.0, 1.0, 2.0, 3.0, //
            1.0, 1.0, 3.0, 0.0, 2.0,
        ],
    )
    .unwrap();
    let (sol_w, cov_w) = solve_linear_least_squares(&wide, &[5.0, 8.0, 13.0]).unwrap();
    assert_eq!(sol_w.rank, 3);
    assert_eq!(sol_w.aliased_columns.len(), 2);
    assert!(sol_w.rss < 1e-24);
    assert_aliased_zeroed(&sol_w.coefficients, &cov_w, &sol_w.aliased_columns, 5);
}

fn random_permuted_effect_matrix(rng: &mut DeterministicRng, b_sign: f64) -> DenseMatrix {
    let mut eff = DenseMatrix::zeros(6, 6).unwrap();
    for r in 0..6 {
        let lv = r / 2;
        let e1 = match lv {
            0 => 1.0,
            2 => -1.0,
            _ => 0.0,
        };
        let e2 = match lv {
            1 => 1.0,
            2 => -1.0,
            _ => 0.0,
        };
        let row = [1.0, e1, e2, b_sign, b_sign * e1, b_sign * e2];
        for (c, &val) in row.iter().enumerate() {
            eff.set(r, c, val).unwrap();
        }
    }
    let mut cols: Vec<usize> = (0..6).collect();
    rng.shuffle(&mut cols);
    let mut perm_eff = DenseMatrix::zeros(6, 6).unwrap();
    for r in 0..6 {
        for (c_out, &c_in) in cols.iter().enumerate() {
            perm_eff.set(r, c_out, eff.get(r, c_in).unwrap()).unwrap();
        }
    }
    perm_eff
}

fn random_tall_standardized_rank4(rng: &mut DeterministicRng, m: usize) -> DenseMatrix {
    let p = 5_usize;
    let mut raw_cols = vec![vec![0.0_f64; m]; p];
    for col in raw_cols.iter_mut().take(p - 1) {
        for cell in col.iter_mut() {
            *cell = rng.standard_normal();
        }
    }
    let weights = [0.7_f64, -1.3, 0.4, 2.1];
    for r in 0..m {
        let sum: f64 = weights.iter().zip(raw_cols.iter()).map(|(&w, col)| w * col.get(r).copied().unwrap_or(0.0)).sum();
        if let Some(last_col) = raw_cols.last_mut()
            && let Some(cell) = last_col.get_mut(r)
        {
            *cell = sum;
        }
    }
    for col in &mut raw_cols {
        let mean = col.iter().sum::<f64>() / (m as f64);
        for v in col.iter_mut() {
            *v -= mean;
        }
        let norm = col.iter().map(|v| v * v).sum::<f64>().sqrt().max(1.0);
        for v in col.iter_mut() {
            *v /= norm;
        }
    }
    let mut tall = DenseMatrix::zeros(m, p).unwrap();
    for r in 0..m {
        for (c, col) in raw_cols.iter().enumerate() {
            tall.set(r, c, col.get(r).copied().unwrap_or(0.0)).unwrap();
        }
    }
    tall
}

#[test]
fn audit_65_jacobi_svd_converges_on_effect_coded_and_scaled_duplicate_matrices() {
    // 1. Exact 6x6 effect-coded [A, A] matrix from audit #65 (singular values: sqrt(12), sqrt(12), 2, 0, 0, 0).
    let a_rows = [[1.0_f64, 1.0, 0.0], [1.0, 1.0, 0.0], [1.0, 0.0, 1.0], [1.0, 0.0, 1.0], [1.0, -1.0, -1.0], [1.0, -1.0, -1.0]];
    let x_aa = DenseMatrix::from_row_major(6, 6, a_rows.iter().flat_map(|r| r.iter().chain(r.iter()).copied()).collect()).unwrap();
    assert_svd_invariants(&x_aa, 3, "6x6 effect-coded [A, A]");
    let sv_aa = svd(&x_aa).unwrap();
    let sqrt12 = 12.0_f64.sqrt();
    assert!((sv_aa.singular_values[0] - sqrt12).abs() < 1e-13);
    assert!((sv_aa.singular_values[1] - sqrt12).abs() < 1e-13);
    assert!((sv_aa.singular_values[2] - 2.0).abs() < 1e-13);
    for s in &sv_aa.singular_values[3..] {
        assert!(*s < 1e-13, "expected zero trailing singular value, got {s}");
    }

    // 2. 7x3 scaled duplicate matrices (QR-preconditioned path: m = 7 >= 2 * 3).
    let c0 = [1.0_f64, -2.0, 0.5, 3.0, -1.5, 2.0, 0.25];
    let c1 = [0.5_f64, 1.0, -1.0, 0.25, 2.5, -0.5, 1.75];
    for scale in [1.0_f64, -3.5, 1e-6, 1e6] {
        let mut data = Vec::with_capacity(21);
        for r in 0..7 {
            data.push(c0[r]);
            data.push(c1[r]);
            data.push(scale * c0[r]);
        }
        let m7x3 = DenseMatrix::from_row_major(7, 3, data).unwrap();
        assert_svd_invariants(&m7x3, 2, &format!("7x3 scaled duplicate scale={scale}"));
    }
}

#[test]
fn audit_65_jacobi_svd_converges_on_effect_coded_and_tall_sweeps() {
    let mut rng = DeterministicRng::from_seed(0xA0D1_7650);
    for trial in 0..1200 {
        let b_sign = if (trial & 1) == 0 { 1.0 } else { -1.0 };
        let perm_eff = random_permuted_effect_matrix(&mut rng, b_sign);
        assert_svd_invariants(&perm_eff, 3, &format!("effect-coded trial {trial}"));
    }
    for trial in 0..200 {
        let tall = random_tall_standardized_rank4(&mut rng, 20);
        assert_svd_invariants(&tall, 4, &format!("tall standardized trial {trial}"));
    }
}

#[test]
fn audit_82_alpha_aware_significance_stars_and_boundary_consistency() {
    // Audit #82 example: 99% confidence (alpha = 0.01) with P = 0.0369 is "ns".
    assert_eq!(PValueFormat::FourDecimal.stars_at_alpha(0.0369, 0.01), "ns");
    assert_eq!(PValueFormat::FourDecimal.summary_stars_at_alpha(0.0369, 0.01), "ns");
    assert_eq!(p_value_stars_at_alpha(0.0369, 0.01), "ns");

    // Existing fixed-0.05 helpers remain unchanged for downstream chart/UI callers.
    assert_eq!(PValueFormat::FourDecimal.stars(0.0369), "*");
    assert_eq!(p_value_stars(0.0369), "*");

    // Consistent inclusive boundary `p <= alpha`.
    assert_eq!(p_value_stars_at_alpha(0.01, 0.01), "*");
    assert_eq!(p_value_stars_at_alpha(0.010_000_1, 0.01), "ns");
    assert_eq!(p_value_stars_at_alpha(0.002, 0.01), "**");
    assert_eq!(p_value_stars_at_alpha(0.0002, 0.01), "***");
    assert_eq!(p_value_stars_at_alpha(0.00002, 0.01), "****");
    assert_eq!(PValueFormat::Apa.stars_at_alpha(0.00002, 0.01), "***");
    assert_eq!(PValueFormat::Nejm.stars_at_alpha(0.00002, 0.01), "***");

    // Alpha >= 0.05 uses alpha as the `*` gate and keeps standard `0.01/0.001/0.0001` higher tiers.
    assert_eq!(p_value_stars_at_alpha(0.08, 0.10), "*");
    assert_eq!(p_value_stars_at_alpha(0.10, 0.10), "*");
    assert_eq!(p_value_stars_at_alpha(0.11, 0.10), "ns");
    assert_eq!(p_value_stars_at_alpha(0.005, 0.10), "**");
    assert_eq!(p_value_stars_at_alpha(0.0005, 0.10), "***");
    assert_eq!(p_value_stars_at_alpha(0.00005, 0.10), "****");

    // Edge cases & invalid inputs never panic.
    for (p, a) in [(f64::NAN, 0.05), (0.01, f64::NAN), (-0.01, 0.05), (1.01, 0.05), (0.01, 0.0), (0.01, 1.0), (0.01, f64::INFINITY)] {
        assert_eq!(p_value_stars_at_alpha(p, a), "ns");
    }
}
