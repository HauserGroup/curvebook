//! Singular Value Decomposition (`A = U diag(sigma) V^T`) via Householder QR preconditioning
//! (when `m > n`) followed by Drmac-Veselic / Hestenes one-sided Jacobi orthogonalization with
//! compensated `Dot2` inner products.
//!
//! Preserves high relative accuracy on tiny singular values of ill-conditioned Vandermonde and
//! collinear regression design matrices (such as NIST StRD `Filip`, `Longley`, and `Wampler5`),
//! handles rectangular (`m >= n` and `m < n`) and zero/rank-deficient matrices cleanly, and never
//! panics.
//!
//! References:
//! - Drmac, Z., & Veselic, K. (2008). New fast and accurate Jacobi SVD algorithm I & II.
//!   *SIAM Journal on Matrix Analysis and Applications*, 29(4), 1322-1362.
//! - Golub, G. H., & Van Loan, C. F. (2013). *Matrix Computations* (4th ed., §8.6).

use crate::error::MathError;
use crate::linalg::matrix::{DenseMatrix, pow2_exp_for_max_abs, scale_pow2};
use crate::linalg::qr::qr_col_pivoted;
use crate::sum::{NeumaierSum, dot};

const MAX_JACOBI_SWEEPS: usize = 64;

/// Thin Singular Value Decomposition `A = U * diag(singular_values) * V^T` where `k = min(nrows, ncols)`.
///
/// - `u` has shape `nrows x k` with orthonormal columns (`U^T U = I_k`),
/// - `singular_values` has length `k` sorted descending (`sigma_0 >= sigma_1 >= ... >= 0`),
/// - `v` has shape `ncols x k` with orthonormal columns (`V^T V = I_k`).
#[derive(Debug, Clone)]
pub struct Svd {
    nrows: usize,
    ncols: usize,
    /// Left singular vectors `U` (`nrows x min(nrows, ncols)`).
    pub u: DenseMatrix,
    /// Non-negative singular values sorted descending (`min(nrows, ncols)`).
    pub singular_values: Vec<f64>,
    /// Right singular vectors `V` (`ncols x min(nrows, ncols)`).
    pub v: DenseMatrix,
}

/// Compute the thin Singular Value Decomposition `A = U diag(sigma) V^T` of any `m x n` finite matrix.
///
/// # Errors
/// Returns [`MathError::DomainError`] if `a` contains `NaN` or infinite entries, or
/// [`MathError::NotConverged`] if Jacobi sweeps exceed `MAX_JACOBI_SWEEPS`.
pub fn svd(a: &DenseMatrix) -> Result<Svd, MathError> {
    if !a.all_finite() {
        return Err(MathError::DomainError { name: "matrix", value: f64::NAN });
    }
    let m = a.nrows();
    let n = a.ncols();
    if m == 0 || n == 0 {
        return Ok(Svd { nrows: m, ncols: n, u: DenseMatrix::zeros(m, 0)?, singular_values: Vec::new(), v: DenseMatrix::zeros(n, 0)? });
    }
    if m < n {
        let at = a.transpose();
        let svd_t = svd_tall_or_square(&at)?;
        return Ok(Svd { nrows: m, ncols: n, u: svd_t.v, singular_values: svd_t.singular_values, v: svd_t.u });
    }
    svd_tall_or_square(a)
}

fn svd_tall_or_square(a: &DenseMatrix) -> Result<Svd, MathError> {
    let scale_exp = pow2_exp_for_max_abs(a.max_abs());
    if scale_exp == 0 {
        return svd_tall_or_square_scaled(a);
    }
    let a_scaled = DenseMatrix::from_fn(a.nrows(), a.ncols(), |r, c| scale_pow2(a.entry(r, c), -scale_exp))?;
    let mut res = svd_tall_or_square_scaled(&a_scaled)?;
    for s in &mut res.singular_values {
        *s = scale_pow2(*s, scale_exp);
        if !s.is_finite() {
            return Err(MathError::DomainError { name: "singular_value", value: *s });
        }
    }
    Ok(res)
}

fn svd_tall_or_square_scaled(a: &DenseMatrix) -> Result<Svd, MathError> {
    let m = a.nrows();
    let n = a.ncols();
    // When m >= 2 * n, precondition by pivoted Householder QR `A P = Q_1 R` (R is n x n),
    // compute SVD of `R = U_r Sigma V_r^T`, and form `U = Q_1 U_r`, `V = P V_r` in O(m n^2)!
    if m >= 2 * n && n > 0 {
        let pqr = qr_col_pivoted(a)?;
        let q1 = pqr.thin_q()?;
        let r = pqr.r()?;
        let perm = pqr.permutation();
        let inner = one_sided_jacobi_svd(&r)?;
        let u_full = q1.matmul(&inner.u)?;
        let mut v_full = DenseMatrix::zeros(n, n)?;
        for (piv_row, &orig_col) in perm.iter().enumerate().take(n) {
            for col in 0..n {
                v_full.set_in_bounds(orig_col, col, inner.v.entry(piv_row, col));
            }
        }
        return Ok(Svd { nrows: m, ncols: n, u: u_full, singular_values: inner.singular_values, v: v_full });
    }
    one_sided_jacobi_svd(a)
}

fn one_sided_jacobi_svd(a: &DenseMatrix) -> Result<Svd, MathError> {
    let m = a.nrows();
    let n = a.ncols();
    let mut w = a.clone();
    let mut v = DenseMatrix::identity(n)?;
    let frob_a = a.frobenius_norm();
    let col_noise_tol = (f64::EPSILON * f64::EPSILON) * frob_a;

    let mut converged = n <= 1 || frob_a == 0.0;
    for _ in 0..MAX_JACOBI_SWEEPS {
        if converged {
            break;
        }
        let mut max_cosine = 0.0_f64;
        for p in 0..n {
            for q in (p + 1)..n {
                let cos_pq = orthogonalize_column_pair(&mut w, &mut v, p, q, col_noise_tol);
                max_cosine = max_cosine.max(cos_pq);
            }
        }
        if max_cosine <= 4.0 * f64::EPSILON {
            converged = true;
        }
    }
    if !converged {
        return Err(MathError::NotConverged { algorithm: "svd_jacobi", iterations: MAX_JACOBI_SWEEPS });
    }

    let mut col_norms: Vec<(usize, f64)> = (0..n).map(|j| (j, w.col_norm2(j))).collect();
    col_norms.sort_by(|x, y| y.1.total_cmp(&x.1));

    let mut u = DenseMatrix::zeros(m, n)?;
    let mut v_sorted = DenseMatrix::zeros(n, n)?;
    let mut sigmas = Vec::with_capacity(n);

    for (out_col, &(src_col, sigma)) in col_norms.iter().enumerate() {
        sigmas.push(sigma);
        for r in 0..n {
            v_sorted.set_in_bounds(r, out_col, v.entry(r, src_col));
        }
        if sigma > 0.0 {
            let subnormal = sigma < f64::MIN_POSITIVE;
            let inv = if subnormal { 0.0 } else { 1.0 / sigma };
            for r in 0..m {
                let val = w.entry(r, src_col);
                u.set_in_bounds(r, out_col, if subnormal { val / sigma } else { val * inv });
            }
        }
    }

    // Complete any zero or noise-floor columns of U so U^T U == I_n holds unconditionally!
    complete_orthonormal_columns(&mut u);

    Ok(Svd { nrows: m, ncols: n, u, singular_values: sigmas, v: v_sorted })
}

/// Apply one symmetric Jacobi rotation `(c, s)` to columns `(p, q)` of `w` and `v`, returning
/// the pre-rotation normalized inner product `|gamma| / sqrt(alpha * beta)`.
fn orthogonalize_column_pair(w: &mut DenseMatrix, v: &mut DenseMatrix, p: usize, q: usize, col_noise_tol: f64) -> f64 {
    let m = w.nrows();
    let n = v.nrows();

    let norm_p = w.col_norm2(p);
    let norm_q = w.col_norm2(q);
    if norm_p <= col_noise_tol || norm_q <= col_noise_tol {
        return 0.0;
    }
    let inv_p = 1.0 / norm_p;
    let inv_q = 1.0 / norm_q;

    // Normalized inner product cos_theta = (w_p / ||w_p||) . (w_q / ||w_q||) avoids overflow/underflow!
    let mut prod_acc = NeumaierSum::new();
    let mut err_acc = NeumaierSum::new();
    for r in 0..m {
        let up = w.entry(r, p) * inv_p;
        let uq = w.entry(r, q) * inv_q;
        let prod = up * uq;
        prod_acc.add(prod);
        err_acc.add(up.mul_add(uq, -prod));
    }
    let cos_pq = prod_acc.total() + err_acc.total();
    let abs_cos = cos_pq.abs();
    if abs_cos <= f64::EPSILON {
        return abs_cos;
    }

    // Exact 2x2 Gram matrix scaled by max(norm_p, norm_q)^2:
    let scale = norm_p.max(norm_q);
    let ap = (norm_p / scale) * (norm_p / scale);
    let bq = (norm_q / scale) * (norm_q / scale);
    let g = (norm_p / scale) * (norm_q / scale) * cos_pq;
    if g == 0.0 || !g.is_finite() {
        return 0.0;
    }

    let zeta = (bq - ap) / (2.0 * g);
    let t = if zeta >= 0.0 { 1.0 / (zeta + zeta.hypot(1.0)) } else { -1.0 / (-zeta + zeta.hypot(1.0)) };
    let c = 1.0 / t.hypot(1.0);
    let s = c * t;

    for r in 0..m {
        let wp = w.entry(r, p);
        let wq = w.entry(r, q);
        w.set_in_bounds(r, p, c * wp - s * wq);
        w.set_in_bounds(r, q, s * wp + c * wq);
    }
    for r in 0..n {
        let vp = v.entry(r, p);
        let vq = v.entry(r, q);
        v.set_in_bounds(r, p, c * vp - s * vq);
        v.set_in_bounds(r, q, s * vp + c * vq);
    }
    abs_cos
}

fn max_abs_dot_prefix(u: &DenseMatrix, col: usize) -> f64 {
    let m = u.nrows();
    let mut max_dot = 0.0_f64;
    for prev in 0..col {
        let mut acc = NeumaierSum::new();
        for r in 0..m {
            acc.add(u.entry(r, prev) * u.entry(r, col));
        }
        max_dot = max_dot.max(acc.total().abs());
    }
    max_dot
}

fn complete_orthonormal_columns(u: &mut DenseMatrix) {
    let m = u.nrows();
    let n = u.ncols();

    for col in 0..n {
        let unit_norm_ok = (u.col_norm2(col) - 1.0).abs() < 1e-10;
        let orth_ok = max_abs_dot_prefix(u, col) < 1e-11;
        if unit_norm_ok && orth_ok {
            continue;
        }
        // Pick standard basis vector e_cand least aligned with existing columns `0..col`.
        let mut best_basis = 0;
        let mut best_rem_norm = -1.0_f64;
        for cand in 0..m {
            let mut vec_c = vec![0.0_f64; m];
            if let Some(slot) = vec_c.get_mut(cand) {
                *slot = 1.0;
            }
            orthogonalize_vec_against_prefix(u, col, &mut vec_c);
            let nrm = vec_norm2(&vec_c);
            if nrm > best_rem_norm {
                best_rem_norm = nrm;
                best_basis = cand;
                if nrm > 0.5 {
                    break;
                }
            }
        }
        let mut vec_c = vec![0.0_f64; m];
        if let Some(slot) = vec_c.get_mut(best_basis) {
            *slot = 1.0;
        }
        orthogonalize_vec_against_prefix(u, col, &mut vec_c);
        orthogonalize_vec_against_prefix(u, col, &mut vec_c);
        let nrm = vec_norm2(&vec_c);
        if nrm > 0.0 {
            let inv = 1.0 / nrm;
            for (r, &val) in vec_c.iter().enumerate() {
                u.set_in_bounds(r, col, val * inv);
            }
        }
    }
}

fn orthogonalize_vec_against_prefix(u: &DenseMatrix, prefix_cols: usize, vec_c: &mut [f64]) {
    let m = u.nrows();
    for prev in 0..prefix_cols {
        let mut acc = NeumaierSum::new();
        for (r, &vc) in vec_c.iter().enumerate().take(m) {
            acc.add(u.entry(r, prev) * vc);
        }
        let proj = acc.total();
        for (r, vc) in vec_c.iter_mut().enumerate().take(m) {
            *vc -= proj * u.entry(r, prev);
        }
    }
}

fn vec_norm2(v: &[f64]) -> f64 {
    let mut scale = 0.0_f64;
    for &x in v {
        scale = scale.max(x.abs());
    }
    if scale == 0.0 {
        return 0.0;
    }
    let inv = 1.0 / scale;
    let mut acc = NeumaierSum::new();
    for &x in v {
        let u = x * inv;
        acc.add(u * u);
    }
    scale * acc.total().max(0.0).sqrt()
}

impl Svd {
    /// Default singular-value threshold `eps * max(nrows, ncols) * sigma_0`.
    #[must_use]
    pub fn default_tol(&self) -> f64 {
        let s0 = self.singular_values.first().copied().unwrap_or(0.0);
        f64::EPSILON * (self.nrows.max(self.ncols) as f64) * s0
    }

    /// Numerical rank under [`Self::default_tol`].
    #[must_use]
    pub fn rank(&self) -> usize {
        self.rank_with_tol(self.default_tol())
    }

    /// Numerical rank counting singular values strictly exceeding `tol`.
    #[must_use]
    pub fn rank_with_tol(&self, tol: f64) -> usize {
        self.singular_values.iter().filter(|&&s| s > tol).count()
    }

    /// 2-norm condition number `sigma_max / sigma_min` (`+inf` if rank-deficient or empty).
    #[must_use]
    pub fn condition_number(&self) -> f64 {
        let Some(&s_max) = self.singular_values.first() else {
            return f64::INFINITY;
        };
        let Some(&s_min) = self.singular_values.last() else {
            return f64::INFINITY;
        };
        if s_min <= 0.0 { f64::INFINITY } else { s_max / s_min }
    }

    /// Moore-Penrose pseudoinverse `A^+ = V * Sigma^+ * U^T` (`ncols x nrows`).
    ///
    /// # Errors
    /// Propagates allocation errors.
    pub fn pseudoinverse(&self, tol: Option<f64>) -> Result<DenseMatrix, MathError> {
        let thresh = tol.unwrap_or_else(|| self.default_tol());
        let k = self.singular_values.len();
        let mut v_scaled = DenseMatrix::zeros(self.ncols, k)?;
        for (j, &s) in self.singular_values.iter().enumerate() {
            if s > thresh {
                let inv_s = 1.0 / s;
                for r in 0..self.ncols {
                    v_scaled.set_in_bounds(r, j, self.v.entry(r, j) * inv_s);
                }
            }
        }
        v_scaled.matmul(&self.u.transpose())
    }

    /// Minimum-norm least-squares solution `x^+ = V * Sigma^+ * (U^T b)`.
    ///
    /// # Errors
    /// Returns [`MathError::DimensionMismatch`] if `b.len() != self.nrows`.
    pub fn solve_least_squares(&self, b: &[f64], tol: Option<f64>) -> Result<Vec<f64>, MathError> {
        if b.len() != self.nrows {
            return Err(MathError::DimensionMismatch { expected: self.nrows, actual: b.len() });
        }
        let thresh = tol.unwrap_or_else(|| self.default_tol());
        let ut_b = self.u.t_matvec(b)?;
        let mut coeff = vec![0.0_f64; ut_b.len()];
        for (slot, (&s, &ub)) in coeff.iter_mut().zip(self.singular_values.iter().zip(ut_b.iter())) {
            if s > thresh {
                *slot = ub / s;
            }
        }
        let mut x = Vec::with_capacity(self.ncols);
        for r in 0..self.ncols {
            let row_v = self.v.row(r).unwrap_or(&[]);
            x.push(dot(row_v, &coeff)?);
        }
        Ok(x)
    }

    /// Unscaled covariance pseudoinverse `(A^T A)^+ = V * (Sigma^+)^2 * V^T` (`ncols x ncols`).
    ///
    /// # Errors
    /// Propagates allocation errors.
    pub fn ata_pseudoinverse(&self, tol: Option<f64>) -> Result<DenseMatrix, MathError> {
        let thresh = tol.unwrap_or_else(|| self.default_tol());
        let k = self.singular_values.len();
        let mut v_scaled = DenseMatrix::zeros(self.ncols, k)?;
        for (j, &s) in self.singular_values.iter().enumerate() {
            if s > thresh {
                let inv_s = 1.0 / s;
                for r in 0..self.ncols {
                    v_scaled.set_in_bounds(r, j, self.v.entry(r, j) * inv_s);
                }
            }
        }
        v_scaled.matmul(&v_scaled.transpose())
    }
}

#[cfg(test)]
mod tests {
    use super::svd;
    use crate::linalg::matrix::DenseMatrix;

    fn assert_svd_invariants(a: &DenseMatrix) {
        let s = svd(a).unwrap();
        let k = a.nrows().min(a.ncols());
        // Check U^T U = I_k and V^T V = I_k:
        let utu = s.u.transpose().matmul(&s.u).unwrap();
        let vtv = s.v.transpose().matmul(&s.v).unwrap();
        for i in 0..k {
            for j in 0..k {
                let exp = if i == j { 1.0 } else { 0.0 };
                assert!((utu.entry(i, j) - exp).abs() < 1e-13, "U^T U [{i},{j}] = {}", utu.entry(i, j));
                assert!((vtv.entry(i, j) - exp).abs() < 1e-13, "V^T V [{i},{j}] = {}", vtv.entry(i, j));
            }
        }
        // Check U * diag(sigma) * V^T == A:
        let mut u_sigma = s.u.clone();
        for r in 0..a.nrows() {
            for j in 0..k {
                u_sigma.set_in_bounds(r, j, u_sigma.entry(r, j) * s.singular_values[j]);
            }
        }
        let recon = u_sigma.matmul(&s.v.transpose()).unwrap();
        let scale = a.frobenius_norm().max(1.0);
        for r in 0..a.nrows() {
            for c in 0..a.ncols() {
                assert!((recon.entry(r, c) - a.entry(r, c)).abs() < 1e-13 * scale);
            }
        }
    }

    #[test]
    fn svd_reconstructs_tall_wide_square_zero_and_rank_deficient_matrices() {
        let tall = DenseMatrix::from_fn(12, 3, |r, c| ((r + 1) as f64).powi(c as i32) + 0.25 * ((r * c) as f64).sin()).unwrap();
        assert_svd_invariants(&tall);

        let wide = tall.transpose();
        assert_svd_invariants(&wide);

        let zero = DenseMatrix::zeros(4, 3).unwrap();
        assert_svd_invariants(&zero);
        assert_eq!(svd(&zero).unwrap().rank(), 0);

        let rank1 = DenseMatrix::from_fn(5, 4, |r, c| ((r + 1) * (c + 2)) as f64).unwrap();
        assert_svd_invariants(&rank1);
        assert_eq!(svd(&rank1).unwrap().rank(), 1);
    }
}
