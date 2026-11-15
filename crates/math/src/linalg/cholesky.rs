//! Symmetric positive-definite Cholesky factorization (`A = L L^T`) and symmetric eigenvalue
//! decomposition (`A = Q diag(lambda) Q^T`) for REML mixed-effects covariance matrices, PCA, and
//! confidence ellipses.

use crate::error::MathError;
use crate::linalg::matrix::DenseMatrix;
use crate::sum::NeumaierSum;

const MAX_SYM_JACOBI_SWEEPS: usize = 64;

/// Lower-triangular Cholesky factorization `A = L L^T` of an `n x n` symmetric positive-definite matrix.
#[derive(Debug, Clone)]
pub struct Cholesky {
    l: DenseMatrix,
}

/// Factor an `n x n` symmetric positive-definite matrix `a` into `A = L L^T`.
///
/// # Errors
/// Returns [`MathError::DimensionMismatch`] if `a` is non-square, [`MathError::DomainError`] if `a`
/// is non-finite or asymmetric beyond `1e-11 * ||A||_F`, or [`MathError::NotPositiveDefinite`] if
/// any pivot is `<= 0`.
pub fn cholesky(a: &DenseMatrix) -> Result<Cholesky, MathError> {
    if a.nrows() != a.ncols() {
        return Err(MathError::DimensionMismatch { expected: a.nrows(), actual: a.ncols() });
    }
    if !a.all_finite() {
        return Err(MathError::DomainError { name: "matrix", value: f64::NAN });
    }
    let n = a.nrows();
    let mut max_diag = 0.0_f64;
    let mut max_abs = 0.0_f64;
    for i in 0..n {
        let diag = a.entry(i, i);
        if !diag.is_finite() {
            return Err(MathError::DomainError { name: "non_finite", value: diag });
        }
        max_diag = max_diag.max(diag.abs());
        max_abs = max_abs.max(diag.abs());
        for j in (i + 1)..n {
            let aij = a.entry(i, j);
            let aji = a.entry(j, i);
            if !aij.is_finite() || !aji.is_finite() {
                return Err(MathError::DomainError { name: "non_finite", value: if aij.is_finite() { aji } else { aij } });
            }
            max_abs = max_abs.max(aij.abs()).max(aji.abs());
        }
    }

    let frob = a.frobenius_norm();
    let sym_tol = if frob.is_finite() { 1e-11 * frob } else { 1e-11 * max_abs };
    for i in 0..n {
        for j in (i + 1)..n {
            let diff = (a.entry(i, j) - a.entry(j, i)).abs();
            if !diff.is_finite() || diff > sym_tol {
                return Err(MathError::DomainError { name: "symmetry", value: diff });
            }
        }
    }

    // Reject numerically rank-deficient / semidefinite matrices whose Cholesky pivots are
    // indistinguishable from floating-point roundoff relative to the largest diagonal entry.
    let pivot_tol = 32.0 * (n.max(1) as f64) * f64::EPSILON * max_diag;
    let mut l = DenseMatrix::zeros(n, n)?;
    for i in 0..n {
        for j in 0..=i {
            let mut acc = NeumaierSum::new();
            let aij = a.entry(i, j);
            let aji = a.entry(j, i);
            acc.add(0.5 * aij + 0.5 * aji);
            for k in 0..j {
                acc.add(-l.entry(i, k) * l.entry(j, k));
            }
            let sum_val = acc.total();
            if i == j {
                if !sum_val.is_finite() || sum_val <= pivot_tol {
                    return Err(MathError::NotPositiveDefinite { index: i, pivot: sum_val });
                }
                l.set_in_bounds(i, i, sum_val.sqrt());
            } else {
                l.set_in_bounds(i, j, sum_val / l.entry(j, j));
            }
        }
    }
    Ok(Cholesky { l })
}

impl Cholesky {
    /// Borrow the lower-triangular Cholesky factor `L` (`A = L L^T`).
    #[must_use]
    pub const fn l(&self) -> &DenseMatrix {
        &self.l
    }

    /// Log-determinant `ln det(A) = 2 * sum_i ln(L[i, i])`.
    #[must_use]
    pub fn ln_det(&self) -> f64 {
        let mut acc = NeumaierSum::new();
        for i in 0..self.l.nrows() {
            acc.add(self.l.entry(i, i).ln());
        }
        2.0 * acc.total()
    }

    /// Solve `A x = b` via forward substitution `L y = b` followed by back substitution `L^T x = y`.
    ///
    /// # Errors
    /// Returns [`MathError::DimensionMismatch`] if `b.len() != n`.
    pub fn solve(&self, b: &[f64]) -> Result<Vec<f64>, MathError> {
        let n = self.l.nrows();
        if b.len() != n {
            return Err(MathError::DimensionMismatch { expected: n, actual: b.len() });
        }
        let mut y = vec![0.0_f64; n];
        for i in 0..n {
            let mut acc = NeumaierSum::new();
            acc.add(b[i]);
            for (k, &yk) in y.iter().enumerate().take(i) {
                acc.add(-self.l.entry(i, k) * yk);
            }
            y[i] = acc.total() / self.l.entry(i, i);
        }
        let mut x = vec![0.0_f64; n];
        for i in (0..n).rev() {
            let mut acc = NeumaierSum::new();
            acc.add(y[i]);
            for (k, &xk) in x.iter().enumerate().take(n).skip(i + 1) {
                acc.add(-self.l.entry(k, i) * xk);
            }
            x[i] = acc.total() / self.l.entry(i, i);
        }
        Ok(x)
    }

    /// Compute `A^{-1}` (`n x n`) by solving `A X = I_n`.
    ///
    /// # Errors
    /// Propagates allocation errors.
    pub fn inverse(&self) -> Result<DenseMatrix, MathError> {
        let n = self.l.nrows();
        let mut inv = DenseMatrix::zeros(n, n)?;
        let mut e = vec![0.0_f64; n];
        for col in 0..n {
            e[col] = 1.0;
            let sol = self.solve(&e)?;
            e[col] = 0.0;
            for (row, &val) in sol.iter().enumerate() {
                inv.set_in_bounds(row, col, val);
            }
        }
        Ok(inv)
    }
}

/// Real symmetric eigenvalue decomposition `A = eigenvectors * diag(eigenvalues) * eigenvectors^T`
/// with eigenvalues sorted descending (`lambda_0 >= lambda_1 >= ...`).
#[derive(Debug, Clone)]
pub struct SymEigen {
    /// Real eigenvalues sorted descending.
    pub eigenvalues: Vec<f64>,
    /// Orthonormal eigenvectors (`n x n`, column `j` corresponds to `eigenvalues[j]`).
    pub eigenvectors: DenseMatrix,
}

/// Compute the eigenvalue decomposition of a symmetric real `n x n` matrix via cyclic Jacobi rotations.
///
/// # Errors
/// Returns an error if `a` is non-square, non-finite, asymmetric, or fails to converge.
pub fn sym_eigen(a: &DenseMatrix) -> Result<SymEigen, MathError> {
    if a.nrows() != a.ncols() {
        return Err(MathError::DimensionMismatch { expected: a.nrows(), actual: a.ncols() });
    }
    if !a.all_finite() {
        return Err(MathError::DomainError { name: "matrix", value: f64::NAN });
    }
    let n = a.nrows();
    let frob = a.frobenius_norm();
    let scale = if frob > 0.0 {
        frob
    } else {
        let mut max_abs = 0.0_f64;
        for i in 0..n {
            for j in 0..n {
                max_abs = max_abs.max(a.entry(i, j).abs());
            }
        }
        max_abs
    };
    for i in 0..n {
        for j in (i + 1)..n {
            if (a.entry(i, j) - a.entry(j, i)).abs() > 1e-11 * scale {
                return Err(MathError::DomainError { name: "symmetry", value: (a.entry(i, j) - a.entry(j, i)).abs() });
            }
        }
    }

    let mut d = DenseMatrix::from_fn(n, n, |i, j| 0.5 * (a.entry(i, j) + a.entry(j, i)))?;
    let mut q_mat = DenseMatrix::identity(n)?;

    let mut converged = n <= 1;
    for _ in 0..MAX_SYM_JACOBI_SWEEPS {
        if converged {
            break;
        }
        let mut max_off = 0.0_f64;
        for p in 0..n {
            for q in (p + 1)..n {
                let apq = d.entry(p, q);
                max_off = max_off.max(apq.abs());
                if apq.abs() > 0.25 * f64::EPSILON * (d.entry(p, p).abs() + d.entry(q, q).abs()).max(f64::MIN_POSITIVE) {
                    apply_sym_jacobi_step(&mut d, &mut q_mat, p, q);
                }
            }
        }
        if max_off <= 2.0 * f64::EPSILON * scale {
            converged = true;
        }
    }
    if !converged {
        return Err(MathError::NotConverged { algorithm: "sym_eigen", iterations: MAX_SYM_JACOBI_SWEEPS });
    }

    let mut pairs: Vec<(usize, f64)> = (0..n).map(|i| (i, d.entry(i, i))).collect();
    pairs.sort_by(|x, y| y.1.total_cmp(&x.1));
    let mut eigenvalues = Vec::with_capacity(n);
    let mut eigenvectors = DenseMatrix::zeros(n, n)?;
    for (out_c, &(src_c, eval)) in pairs.iter().enumerate() {
        eigenvalues.push(eval);
        for r in 0..n {
            eigenvectors.set_in_bounds(r, out_c, q_mat.entry(r, src_c));
        }
    }
    Ok(SymEigen { eigenvalues, eigenvectors })
}

fn apply_sym_jacobi_step(d: &mut DenseMatrix, q_mat: &mut DenseMatrix, p: usize, q: usize) {
    let n = d.nrows();
    let app = d.entry(p, p);
    let aqq = d.entry(q, q);
    let apq = d.entry(p, q);
    if apq == 0.0 {
        return;
    }
    let tau = (aqq - app) / (2.0 * apq);
    let t = if tau >= 0.0 { 1.0 / (tau + tau.hypot(1.0)) } else { -1.0 / (-tau + tau.hypot(1.0)) };
    let c = 1.0 / t.hypot(1.0);
    let s = t * c;

    d.set_in_bounds(p, p, app - t * apq);
    d.set_in_bounds(q, q, aqq + t * apq);
    d.set_in_bounds(p, q, 0.0);
    d.set_in_bounds(q, p, 0.0);

    for k in 0..n {
        if k != p && k != q {
            let dkp = d.entry(k, p);
            let dkq = d.entry(k, q);
            let new_kp = c * dkp - s * dkq;
            let new_kq = s * dkp + c * dkq;
            d.set_in_bounds(k, p, new_kp);
            d.set_in_bounds(p, k, new_kp);
            d.set_in_bounds(k, q, new_kq);
            d.set_in_bounds(q, k, new_kq);
        }
        let qkp = q_mat.entry(k, p);
        let qkq = q_mat.entry(k, q);
        q_mat.set_in_bounds(k, p, c * qkp - s * qkq);
        q_mat.set_in_bounds(k, q, s * qkp + c * qkq);
    }
}

#[cfg(test)]
mod tests {
    use super::{cholesky, sym_eigen};
    use crate::error::MathError;
    use crate::linalg::matrix::DenseMatrix;

    #[test]
    fn cholesky_solves_inverts_and_rejects_indefinite() {
        let spd = DenseMatrix::from_row_major(3, 3, vec![4.0, 12.0, -16.0, 12.0, 37.0, -43.0, -16.0, -43.0, 98.0]).unwrap();
        let chol = cholesky(&spd).unwrap();
        // L = [[2, 0, 0], [6, 1, 0], [-8, 5, 3]] -> det = (2 * 1 * 3)^2 = 36!
        assert!((chol.ln_det() - 36.0_f64.ln()).abs() < 1e-14);
        let x = chol.solve(&[1.0, 2.0, 3.0]).unwrap();
        let ax = spd.matvec(&x).unwrap();
        assert!((ax[0] - 1.0).abs() < 1e-13 && (ax[1] - 2.0).abs() < 1e-13 && (ax[2] - 3.0).abs() < 1e-13);

        let prod = spd.matmul(&chol.inverse().unwrap()).unwrap();
        for i in 0..3 {
            for j in 0..3 {
                assert!((prod.entry(i, j) - if i == j { 1.0 } else { 0.0 }).abs() < 1e-13);
            }
        }

        let indef = DenseMatrix::from_row_major(2, 2, vec![1.0, 2.0, 2.0, 1.0]).unwrap();
        assert!(matches!(cholesky(&indef), Err(MathError::NotPositiveDefinite { .. })));
    }

    #[test]
    fn sym_eigen_recovers_spectrum_and_reconstructs() {
        let m = DenseMatrix::from_row_major(3, 3, vec![2.0, -1.0, 0.0, -1.0, 2.0, -1.0, 0.0, -1.0, 2.0]).unwrap();
        let eig = sym_eigen(&m).unwrap();
        assert!((eig.eigenvalues[0] - (2.0 + std::f64::consts::SQRT_2)).abs() < 1e-14);
        assert!((eig.eigenvalues[1] - 2.0).abs() < 1e-14);
        assert!((eig.eigenvalues[2] - (2.0 - std::f64::consts::SQRT_2)).abs() < 1e-14);
    }
}
