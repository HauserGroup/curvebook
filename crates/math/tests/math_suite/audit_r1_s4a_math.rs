//! Stage 4A regression tests for S01 fixes in `curvebook-math` (`R1-S01-02`, `R1-S01-03`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_math::dist::{ncchi2_cdf, ncchi2_sf, ncf_cdf, ncf_sf, norm_cdf};
use curvebook_math::{DenseMatrix, noncentral_chi2_ci, noncentral_f_ci, sym_eigen};
use curvebook_testkit::assert_close as assert_near;

#[test]
fn r1_s01_02_noncentral_f_and_chi2_above_million() {
    // Previously `lambda > 1_000_000` (`k0 > 500_000`) failed/under-converged due to the hardcoded cap.
    let c_1p5e6 = ncf_cdf(600_000.0, 1.0, 10.0, 1_500_000.0).unwrap();
    let c_1p01e6 = ncf_cdf(600_000.0, 1.0, 10.0, 1_010_000.0).unwrap();
    assert_near(c_1p5e6, 0.005_346_01, 1e-4, "ncf_cdf(6e5, 1, 10, 1.5e6)");
    assert_near(c_1p01e6, 0.078_137_22, 1e-3, "ncf_cdf(6e5, 1, 10, 1.01e6)");
    assert_near(c_1p5e6 + ncf_sf(600_000.0, 1.0, 10.0, 1_500_000.0).unwrap(), 1.0, 1e-12, "ncf cdf+sf complement");

    // Noncentral F CI whose upper bound crosses lambda > 1_000_000.
    let (f_lo, f_hi) = noncentral_f_ci(600_000.0, 1.0, 10.0, 95.0).unwrap();
    assert!(f_lo > 0.0 && f_lo < f_hi && f_hi > 1_000_000.0, "expected huge F CI crossing 1e6, got ({f_lo}, {f_hi})");
    assert!((f_hi - 1_229_002.0).abs() / 1_229_002.0 < 1e-3, "unexpected F CI upper bound: {f_hi}");
    assert_near(ncf_sf(600_000.0, 1.0, 10.0, f_lo).unwrap(), 0.025, 1e-6, "ncf CI lower roundtrip");
    assert_near(ncf_cdf(600_000.0, 1.0, 10.0, f_hi).unwrap(), 0.025, 1e-6, "ncf CI upper roundtrip");

    // Smooth handoff across `lambda_half = 1_000_000` (`lambda = 2_000_000`) quadrature switch.
    let ncf_left = ncf_cdf(1_999_999.0, 4.0, 20.0, 1_999_999.9).unwrap();
    let ncf_right = ncf_cdf(1_999_999.0, 4.0, 20.0, 2_000_000.1).unwrap();
    assert!(
        ncf_left > ncf_right && (ncf_left - ncf_right).abs() < 1e-6,
        "ncf seam continuity failed across lambda=2e6: left={ncf_left}, right={ncf_right}"
    );

    let chi_left = ncchi2_cdf(2_000_000.0, 10.0, 1_999_999.9).unwrap();
    let chi_right = ncchi2_cdf(2_000_000.0, 10.0, 2_000_000.1).unwrap();
    assert!(
        chi_left > chi_right && (chi_left - chi_right).abs() < 1e-4,
        "ncchi2 seam continuity failed across lambda=2e6: left={chi_left}, right={chi_right}"
    );
    // Normal check at lambda = 2e6+10: mean = df + lam = 2_000_020, var = 2*df + 4*lam = 8_000_060.
    let chi_2m10 = ncchi2_cdf(2_000_000.0, 10.0, 2_000_010.0).unwrap();
    assert_near(chi_2m10, norm_cdf(-20.0 / (8_000_060.0_f64).sqrt()), 1e-3, "ncchi2(2e6, 10, 2e6+10)");
    assert_near(chi_2m10 + ncchi2_sf(2_000_000.0, 10.0, 2_000_010.0).unwrap(), 1.0, 1e-12, "ncchi2 cdf+sf complement");

    // Very large noncentralities (1e8, 1e12) and CI roundtrip above 2e6.
    // Note: E[num/df1] = (df1 + ncp)/df1; for df1=2, ncp=2.5e6 -> 1.25e6 (below 2e6, above 1e6).
    assert!(ncchi2_cdf(5_000_000.0, 5.0, 4_995_000.0).unwrap() > 0.5);
    assert!(ncchi2_cdf(1e8, 5.0, 1e8 + 1e5).unwrap() < 0.01);
    assert!(ncf_cdf(1_000_000.0, 2.0, 12.0, 2.5e6).unwrap() < 0.5);
    assert!(ncf_cdf(2_000_000.0, 2.0, 12.0, 2.5e6).unwrap() > 0.5);
    assert!(ncf_sf(1_000_000.0, 2.0, 12.0, 2.5e6).unwrap() > 0.5);

    let (c_lo, c_hi) = noncentral_chi2_ci(3_000_000.0, 10.0, 95.0).unwrap();
    assert!(c_lo > 0.0 && c_lo < 3_000_000.0 && 3_000_000.0 < c_hi, "expected chi2 CI enclosing 3e6, got ({c_lo}, {c_hi})");
    assert_near(ncchi2_sf(3_000_000.0, 10.0, c_lo).unwrap(), 0.025, 1e-6, "ncchi2 CI lower roundtrip");
    assert_near(ncchi2_cdf(3_000_000.0, 10.0, c_hi).unwrap(), 0.025, 1e-6, "ncchi2 CI upper roundtrip");
}

#[test]
fn r1_s01_03_sym_eigen_small_scale_matrices() {
    let base_data = vec![4.0, 1.5, -0.5, 1.5, 3.0, 0.75, -0.5, 0.75, 2.0];
    let base = DenseMatrix::from_row_major(3, 3, base_data.clone()).unwrap();
    let eig_unit = sym_eigen(&base).unwrap();

    for scale in [1e-17_f64, 1e-20, 1e-100, -1e-30] {
        let scaled_data: Vec<f64> = base_data.iter().map(|v| v * scale).collect();
        let scaled = DenseMatrix::from_row_major(3, 3, scaled_data).unwrap();
        let eig = sym_eigen(&scaled).unwrap();

        // Check V^T V = I.
        let n = scaled.nrows();
        for i in 0..n {
            for j in 0..n {
                let mut dot = 0.0_f64;
                for r in 0..n {
                    dot += eig.eigenvectors.get(r, i).unwrap() * eig.eigenvectors.get(r, j).unwrap();
                }
                let target = if i == j { 1.0 } else { 0.0 };
                assert!((dot - target).abs() < 1e-12, "scale={scale}: V^T V ({i},{j}) = {dot}");
            }
        }

        // Check reconstruction V D V^T = A relative to ||A||_F.
        let frob_a: f64 = scaled.as_slice().iter().map(|v| v * v).sum::<f64>().sqrt();
        let mut err_sq = 0.0_f64;
        for r in 0..n {
            for c in 0..n {
                let mut rec = 0.0_f64;
                for k in 0..n {
                    rec += eig.eigenvectors.get(r, k).unwrap() * eig.eigenvalues[k] * eig.eigenvectors.get(c, k).unwrap();
                }
                let d = rec - scaled.get(r, c).unwrap();
                err_sq += d * d;
            }
        }
        let rel_err = err_sq.sqrt() / frob_a;
        assert!(rel_err < 1e-12, "scale={scale}: relative reconstruction error {rel_err}");

        // Check descending eigenvalues match scaled reference eigenvalues.
        let mut expected_eigs: Vec<f64> = eig_unit.eigenvalues.iter().map(|v| v * scale).collect();
        expected_eigs.sort_by(|a, b| b.total_cmp(a));
        for (actual, exp) in eig.eigenvalues.iter().zip(&expected_eigs) {
            let rel = (actual - exp).abs() / exp.abs();
            assert!(rel < 1e-11, "scale={scale}: eigenvalue mismatch actual={actual} vs exp={exp}");
        }
    }

    // Exact zero symmetric matrix should succeed cleanly with zero eigenvalues.
    let zero = DenseMatrix::zeros(3, 3).unwrap();
    let eig_zero = sym_eigen(&zero).unwrap();
    assert_eq!(eig_zero.eigenvalues, vec![0.0, 0.0, 0.0]);
}
