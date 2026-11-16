//! Stage 4B regression tests for `R1-S01-04` (`crates/math/src/linalg/cholesky.rs`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_math::{DenseMatrix, MathError, cholesky};

#[test]
fn r1_s01_04_cholesky_rejects_collinear_gram_and_singular_matrices_across_scales() {
    // Rank-2 design matrix (col 2 == col 0 + col 1) yields exact rank-deficient X^T X.
    let rows = [[1.0_f64, 1.0, 2.0], [2.0, 3.0, 5.0], [4.0, 1.0, 5.0], [3.0, 2.0, 5.0]];
    for scale in [1e-12_f64, 1e-6, 1.0, 1e6, 1e12] {
        let mut xtx = vec![0.0_f64; 9];
        for r in &rows {
            for i in 0..3 {
                for j in 0..3 {
                    xtx[i * 3 + j] += scale * r[i] * r[j];
                }
            }
        }
        let mat = DenseMatrix::from_row_major(3, 3, xtx).unwrap();
        let err = cholesky(&mat).unwrap_err();
        assert!(
            matches!(err, MathError::NotPositiveDefinite { .. }),
            "collinear X^T X at scale {scale} should fail with NotPositiveDefinite, got {err:?}"
        );
    }

    // Zero diagonal / exact semidefinite must be rejected regardless of scale.
    let zero_diag = DenseMatrix::from_row_major(2, 2, vec![1.0, 0.0, 0.0, 0.0]).unwrap();
    assert!(matches!(cholesky(&zero_diag), Err(MathError::NotPositiveDefinite { index: 1, .. })));

    // Tiny machine-roundoff negative/zero-ish pivot relative to max_diag (`1.0`).
    let near_singular = DenseMatrix::from_row_major(2, 2, vec![1.0, 1.0, 1.0, 1.0 + 1e-15]).unwrap();
    assert!(matches!(cholesky(&near_singular), Err(MathError::NotPositiveDefinite { index: 1, .. })));
}

#[test]
fn r1_s01_04_cholesky_scale_invariant_spd_symmetry_and_non_finite() {
    // Truly SPD tiny matrices (below legacy 1e-14 absolute floor) must factorize accurately.
    for scale in [1e-16_f64, 1e-12, 1e160] {
        let a = DenseMatrix::from_row_major(2, 2, vec![4.0 * scale, scale, scale, 4.0 * scale]).unwrap();
        let chol = cholesky(&a).unwrap_or_else(|e| panic!("well-conditioned SPD at scale {scale} failed: {e}"));
        let b = [5.0 * scale, 5.0 * scale];
        let x = chol.solve(&b).unwrap();
        assert!((x[0] - 1.0).abs() < 1e-12 && (x[1] - 1.0).abs() < 1e-12, "solve mismatch at scale {scale}: {x:?}");
    }

    // Asymmetry above 1e-11 * ||A||_F on small scale (1e-12) must fail symmetry check.
    let asym_small = DenseMatrix::from_row_major(2, 2, vec![1e-12, 0.0, 1e-21, 1e-12]).unwrap();
    assert!(matches!(cholesky(&asym_small), Err(MathError::DomainError { name: "symmetry", .. })));

    // Mild numerical asymmetry well within 1e-11 * ||A||_F succeeds cleanly and symmetrizes.
    let tiny_asym = DenseMatrix::from_row_major(2, 2, vec![1e-12, 0.25e-12, 0.25e-12 + 1e-25, 1e-12]).unwrap();
    assert!(cholesky(&tiny_asym).is_ok());

    // Non-finite entries anywhere in A must return DomainError cleanly.
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let bad_diag = DenseMatrix::from_row_major(2, 2, vec![bad, 0.0, 0.0, 1.0]).unwrap();
        assert!(matches!(cholesky(&bad_diag), Err(MathError::DomainError { .. })));

        let bad_off = DenseMatrix::from_row_major(2, 2, vec![1.0, bad, bad, 1.0]).unwrap();
        assert!(matches!(cholesky(&bad_off), Err(MathError::DomainError { .. })));
    }
}
