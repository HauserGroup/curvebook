//! Unpivoted and column-pivoted rank-revealing Householder QR factorizations (`A = Q R` and
//! `A P = Q R`), parameter covariance `(R^T R)^{-1}`, multicollinearity dependency metrics, and
//! equilibrated rank-safe linear least-squares solvers.
//!
//! Per `docs/implementation/statistics-implementation.md` §1.3, design matrices `X in R^{n x p}` use
//! column-pivoted Householder QR (`qr_col_pivoted`) with explicit rank-deficiency detection
//! `|R_{kk}| <= eps * max(n, p) * |R_{00}|`, falling back cleanly to SVD pseudoinverse or reporting
//! aliased columns rather than dividing by zero.
//!
//! References:
//! - Golub, G. H., & Van Loan, C. F. (2013). *Matrix Computations* (4th ed., §5.2 & §5.4.2).
//! - Bjorck, A. (1996). *Numerical Methods for Least Squares Problems*. SIAM.

use crate::error::MathError;
use crate::linalg::matrix::DenseMatrix;
use crate::linalg::svd::svd;
use crate::sum::{NeumaierSum, sum_iter};

/// Compact Householder QR factorization `A P = Q R` (where `perm == 0..n` when unpivoted).
#[derive(Debug, Clone)]
pub struct PivotedQr {
    nrows: usize,
    ncols: usize,
    /// Compact storage: upper triangle is `R`; strict lower triangle in column `k` stores the tail
    /// `v[k+1..nrows]` of unit-leading (`v[k] = 1`) Householder vector `v_k`.
    factors: DenseMatrix,
    /// Scalar Householder weights `tau_k` (`I - tau_k v_k v_k^T`) for `k = 0..min(nrows, ncols)`.
    taus: Vec<f64>,
    /// Column permutation `P`: column `j` of `A P` corresponds to original column `perm[j]` of `A`.
    perm: Vec<usize>,
    /// Detected numerical rank under default threshold `eps * max(m, n) * max_k |R_{kk}|`.
    rank: usize,
    /// Whether column pivoting was enabled during factorization.
    pivoted: bool,
    /// Default diagonal tolerance `eps * max(m, n) * max_k |R_{kk}|`.
    default_tol: f64,
}

/// Structured output of a least-squares linear regression or Jacobian step solve `min_x ||A x - b||_2`.
#[derive(Debug, Clone, PartialEq)]
pub struct LeastSquaresSolution {
    /// Estimated parameter vector `x` (`ncols` elements).
    pub coefficients: Vec<f64>,
    /// Numerical rank of `A`.
    pub rank: usize,
    /// Original column indices (`0..ncols`) detected as collinear/aliased (`empty` when full rank).
    pub aliased_columns: Vec<usize>,
    /// Residual sum of squares `||b - A x||_2^2` computed via compensated summation.
    pub rss: f64,
    /// Degrees of freedom `nrows.saturating_sub(rank)`.
    pub df_residual: usize,
}

/// Compute unpivoted Householder QR factorization `A = Q R` (useful for damped Levenberg-Marquardt
/// augmented systems where column order is fixed).
///
/// # Errors
/// Returns [`MathError::DomainError`] if `a` contains `NaN` or infinite entries.
pub fn qr(a: &DenseMatrix) -> Result<PivotedQr, MathError> {
    factor_householder(a, false)
}

/// Compute rank-revealing column-pivoted Householder QR factorization `A P = Q R` with exact
/// compensated norm recomputation and explicit rank-deficiency threshold
/// `|R_{kk}| <= eps * max(m, n) * |R_{00}|`.
///
/// # Errors
/// Returns [`MathError::DomainError`] if `a` contains `NaN` or infinite entries.
pub fn qr_col_pivoted(a: &DenseMatrix) -> Result<PivotedQr, MathError> {
    factor_householder(a, true)
}

fn factor_householder(a: &DenseMatrix, pivot_columns: bool) -> Result<PivotedQr, MathError> {
    if !a.all_finite() {
        return Err(MathError::DomainError { name: "matrix", value: f64::NAN });
    }
    let m = a.nrows();
    let n = a.ncols();
    let k_max = m.min(n);
    let mut work = a.clone();
    let mut perm: Vec<usize> = (0..n).collect();
    let mut taus = vec![0.0_f64; k_max];

    for (k, tau_slot) in taus.iter_mut().enumerate().take(k_max) {
        if pivot_columns {
            let best_col = select_max_norm_column(&work, k, n);
            if best_col != k {
                swap_matrix_columns(&mut work, k, best_col);
                perm.swap(k, best_col);
            }
        }
        let tau = compute_householder_column(&mut work, k, m);
        *tau_slot = tau;
        if tau != 0.0 && k + 1 < n {
            apply_householder_to_trailing(&mut work, k, m, n, tau);
        }
    }

    let diag_scale = if pivot_columns {
        if k_max > 0 { work.entry(0, 0).abs() } else { 0.0 }
    } else {
        (0..k_max).fold(0.0_f64, |acc, k| acc.max(work.entry(k, k).abs()))
    };
    let default_tol = f64::EPSILON * (m.max(n) as f64) * diag_scale;
    let rank = count_rank(&work, k_max, default_tol, pivot_columns);

    Ok(PivotedQr { nrows: m, ncols: n, factors: work, taus, perm, rank, pivoted: pivot_columns, default_tol })
}

fn select_max_norm_column(work: &DenseMatrix, k: usize, n: usize) -> usize {
    let mut best_col = k;
    let mut best_norm = work.col_norm2_from(k, k);
    for c in (k + 1)..n {
        let cand = work.col_norm2_from(c, k);
        if cand > best_norm {
            best_norm = cand;
            best_col = c;
        }
    }
    best_col
}

fn swap_matrix_columns(m: &mut DenseMatrix, c1: usize, c2: usize) {
    let rows = m.nrows();
    for r in 0..rows {
        let v1 = m.entry(r, c1);
        let v2 = m.entry(r, c2);
        m.set_in_bounds(r, c1, v2);
        m.set_in_bounds(r, c2, v1);
    }
}

/// Form Householder reflector `H_k = I - tau v v^T` (`v[k] = 1`) zeroing `work[k+1..m, k]`,
/// storing `-sign(alpha) * ||x||_2` in `work[k, k]` and `v[k+1..m]` in `work[k+1..m, k]`.
fn compute_householder_column(work: &mut DenseMatrix, k: usize, m: usize) -> f64 {
    if k + 1 >= m {
        return 0.0;
    }
    let norm_x = work.col_norm2_from(k, k);
    if norm_x == 0.0 {
        return 0.0;
    }
    let alpha = work.entry(k, k);
    // Choose beta with opposite sign to alpha to avoid subtraction cancellation in `alpha - beta`.
    let beta = if alpha >= 0.0 { -norm_x } else { norm_x };
    let v0 = alpha - beta;
    if v0 == 0.0 {
        return 0.0;
    }
    let inv_v0 = 1.0 / v0;
    for r in (k + 1)..m {
        let scaled = work.entry(r, k) * inv_v0;
        work.set_in_bounds(r, k, scaled);
    }
    work.set_in_bounds(k, k, beta);
    (beta - alpha) / beta
}

fn apply_householder_to_trailing(work: &mut DenseMatrix, k: usize, m: usize, n: usize, tau: f64) {
    for c in (k + 1)..n {
        let mut prod_acc = NeumaierSum::new();
        let mut err_acc = NeumaierSum::new();
        prod_acc.add(work.entry(k, c));
        for r in (k + 1)..m {
            let vr = work.entry(r, k);
            let xrc = work.entry(r, c);
            let p = vr * xrc;
            prod_acc.add(p);
            err_acc.add(vr.mul_add(xrc, -p));
        }
        let dot_val = tau * (prod_acc.total() + err_acc.total());
        work.set_in_bounds(k, c, work.entry(k, c) - dot_val);
        for r in (k + 1)..m {
            let vr = work.entry(r, k);
            work.set_in_bounds(r, c, work.entry(r, c) - dot_val * vr);
        }
    }
}

fn count_rank(factors: &DenseMatrix, k_max: usize, tol: f64, pivoted: bool) -> usize {
    if k_max == 0 {
        return 0;
    }
    if pivoted {
        if factors.entry(0, 0).abs() == 0.0 {
            return 0;
        }
        let mut r = 0;
        for k in 0..k_max {
            if factors.entry(k, k).abs() > tol {
                r += 1;
            } else {
                break;
            }
        }
        r
    } else {
        let mut r = 0;
        for k in 0..k_max {
            if factors.entry(k, k).abs() > tol {
                r += 1;
            }
        }
        r
    }
}

impl PivotedQr {
    /// Row count `m` of the factored matrix.
    #[must_use]
    pub const fn nrows(&self) -> usize {
        self.nrows
    }

    /// Column count `n` of the factored matrix.
    #[must_use]
    pub const fn ncols(&self) -> usize {
        self.ncols
    }

    /// Detected numerical rank under `|R_{kk}| > eps * max(m, n) * max_j |R_{jj}|`.
    #[must_use]
    pub const fn rank(&self) -> usize {
        self.rank
    }

    /// Re-evaluate numerical rank with a caller-supplied diagonal tolerance `tol >= 0`.
    #[must_use]
    pub fn rank_with_tol(&self, tol: f64) -> usize {
        count_rank(&self.factors, self.nrows.min(self.ncols), tol, self.pivoted)
    }

    /// Whether `rank == min(nrows, ncols)`.
    #[must_use]
    pub fn is_full_rank(&self) -> bool {
        self.rank == self.nrows.min(self.ncols)
    }

    /// Column permutation `P` (`A[:, perm[j]]` is column `j` of `A P`).
    #[must_use]
    pub fn permutation(&self) -> &[usize] {
        &self.perm
    }

    /// Original column indices detected as linearly dependent / aliased (sorted).
    #[must_use]
    pub fn aliased_columns(&self) -> Vec<usize> {
        if self.pivoted {
            let mut cols: Vec<usize> = self.perm.get(self.rank..).unwrap_or(&[]).to_vec();
            cols.sort_unstable();
            cols
        } else {
            let k_max = self.nrows.min(self.ncols);
            let mut cols = Vec::new();
            for j in 0..self.ncols {
                if j >= k_max || self.factors.entry(j, j).abs() <= self.default_tol {
                    cols.push(j);
                }
            }
            cols
        }
    }

    /// Extract the thin upper-trapezoidal factor `R` (`min(m, n) x n`).
    ///
    /// # Errors
    /// Propagates allocation errors if dimensions are out of bounds.
    pub fn r(&self) -> Result<DenseMatrix, MathError> {
        let k = self.nrows.min(self.ncols);
        let mut r_mat = DenseMatrix::zeros(k, self.ncols)?;
        for i in 0..k {
            for j in i..self.ncols {
                r_mat.set_in_bounds(i, j, self.factors.entry(i, j));
            }
        }
        Ok(r_mat)
    }

    /// Materialize the thin orthonormal factor `Q_1` (`m x min(m, n)`, `Q_1^T Q_1 = I`).
    ///
    /// # Errors
    /// Propagates allocation errors.
    pub fn thin_q(&self) -> Result<DenseMatrix, MathError> {
        let k_max = self.nrows.min(self.ncols);
        let mut q = DenseMatrix::zeros(self.nrows, k_max)?;
        for i in 0..k_max {
            q.set_in_bounds(i, i, 1.0);
        }
        self.apply_q_left_in_place(&mut q)?;
        Ok(q)
    }

    /// Materialize the full orthogonal factor `Q` (`m x m`, `Q^T Q = Q Q^T = I_m`).
    ///
    /// # Errors
    /// Propagates allocation errors.
    pub fn full_q(&self) -> Result<DenseMatrix, MathError> {
        let mut q = DenseMatrix::identity(self.nrows)?;
        self.apply_q_left_in_place(&mut q)?;
        Ok(q)
    }

    fn apply_q_left_in_place(&self, mat: &mut DenseMatrix) -> Result<(), MathError> {
        if mat.nrows() != self.nrows {
            return Err(MathError::DimensionMismatch { expected: self.nrows, actual: mat.nrows() });
        }
        let cols = mat.ncols();
        for (k, &tau) in self.taus.iter().enumerate().rev() {
            if tau == 0.0 {
                continue;
            }
            for c in 0..cols {
                let mut acc = NeumaierSum::new();
                acc.add(mat.entry(k, c));
                for r in (k + 1)..self.nrows {
                    acc.add(self.factors.entry(r, k) * mat.entry(r, c));
                }
                let w = tau * acc.total();
                mat.set_in_bounds(k, c, mat.entry(k, c) - w);
                for r in (k + 1)..self.nrows {
                    mat.set_in_bounds(r, c, mat.entry(r, c) - w * self.factors.entry(r, k));
                }
            }
        }
        Ok(())
    }

    /// Compute `Q^T * b` (`m` elements) implicitly from the stored Householder vectors without
    /// allocating `m x m` `Q`.
    ///
    /// # Errors
    /// Returns [`MathError::DimensionMismatch`] if `b.len() != self.nrows`.
    pub fn apply_qt(&self, b: &[f64]) -> Result<Vec<f64>, MathError> {
        if b.len() != self.nrows {
            return Err(MathError::DimensionMismatch { expected: self.nrows, actual: b.len() });
        }
        let mut y = b.to_vec();
        for (k, &tau) in self.taus.iter().enumerate() {
            if tau == 0.0 {
                continue;
            }
            let mut prod_acc = NeumaierSum::new();
            let mut err_acc = NeumaierSum::new();
            prod_acc.add(y.get(k).copied().unwrap_or(0.0));
            for r in (k + 1)..self.nrows {
                let vr = self.factors.entry(r, k);
                let yr = y.get(r).copied().unwrap_or(0.0);
                let p = vr * yr;
                prod_acc.add(p);
                err_acc.add(vr.mul_add(yr, -p));
            }
            let w = tau * (prod_acc.total() + err_acc.total());
            if let Some(yk) = y.get_mut(k) {
                *yk -= w;
            }
            for r in (k + 1)..self.nrows {
                let vr = self.factors.entry(r, k);
                if let Some(yr) = y.get_mut(r) {
                    *yr -= w * vr;
                }
            }
        }
        Ok(y)
    }

    /// Compute `Q * v` (`m` elements) implicitly from stored Householder reflectors.
    fn apply_q_vec(&self, v: &[f64]) -> Result<Vec<f64>, MathError> {
        if v.len() != self.nrows {
            return Err(MathError::DimensionMismatch { expected: self.nrows, actual: v.len() });
        }
        let mut out = v.to_vec();
        for (k, &tau) in self.taus.iter().enumerate().rev() {
            if tau == 0.0 {
                continue;
            }
            let mut prod_acc = NeumaierSum::new();
            let mut err_acc = NeumaierSum::new();
            prod_acc.add(out.get(k).copied().unwrap_or(0.0));
            for r in (k + 1)..self.nrows {
                let vr = self.factors.entry(r, k);
                let or = out.get(r).copied().unwrap_or(0.0);
                let p = vr * or;
                prod_acc.add(p);
                err_acc.add(vr.mul_add(or, -p));
            }
            let w = tau * (prod_acc.total() + err_acc.total());
            if let Some(ok) = out.get_mut(k) {
                *ok -= w;
            }
            for r in (k + 1)..self.nrows {
                let vr = self.factors.entry(r, k);
                if let Some(or) = out.get_mut(r) {
                    *or -= w * vr;
                }
            }
        }
        Ok(out)
    }

    /// Solve `R^T u = rhs` (`ncols` elements, full-rank `nrows >= ncols`).
    fn solve_rt_full(&self, rhs: &[f64]) -> Vec<f64> {
        let p = self.ncols;
        let mut u = vec![0.0_f64; p];
        for i in 0..p {
            let mut acc = NeumaierSum::new();
            acc.add(rhs.get(i).copied().unwrap_or(0.0));
            for (k, &uk) in u.iter().enumerate().take(i) {
                acc.add(-self.factors.entry(k, i) * uk);
            }
            if let Some(slot) = u.get_mut(i) {
                *slot = acc.total() / self.factors.entry(i, i);
            }
        }
        u
    }

    /// Solve `R w = rhs` (`ncols` elements, full-rank `nrows >= ncols`).
    fn solve_r_full(&self, rhs: &[f64]) -> Vec<f64> {
        let p = self.ncols;
        let mut w = vec![0.0_f64; p];
        for i in (0..p).rev() {
            let mut acc = NeumaierSum::new();
            acc.add(rhs.get(i).copied().unwrap_or(0.0));
            for (j, &wj) in w.iter().enumerate().take(p).skip(i + 1) {
                acc.add(-self.factors.entry(i, j) * wj);
            }
            if let Some(slot) = w.get_mut(i) {
                *slot = acc.total() / self.factors.entry(i, i);
            }
        }
        w
    }

    /// Solve `min_x ||A x - b||_2` using the basic QR solution on non-aliased columns
    /// (setting aliased columns to `0.0`).
    ///
    /// # Errors
    /// Returns [`MathError::DimensionMismatch`] if `b.len() != self.nrows`.
    pub fn solve_basic(&self, b: &[f64]) -> Result<Vec<f64>, MathError> {
        let qtb = self.apply_qt(b)?;
        let active_cols = if self.pivoted { self.rank } else { self.nrows.min(self.ncols) };
        let mut z = vec![0.0_f64; self.ncols];
        for i in (0..active_cols).rev() {
            let diag = self.factors.entry(i, i);
            if !self.pivoted && diag.abs() <= self.default_tol {
                continue;
            }
            let mut acc = NeumaierSum::new();
            acc.add(qtb.get(i).copied().unwrap_or(0.0));
            for (j, &zj) in z.iter().enumerate().take(active_cols).skip(i + 1) {
                acc.add(-self.factors.entry(i, j) * zj);
            }
            if let Some(slot) = z.get_mut(i) {
                *slot = acc.total() / diag;
            }
        }
        let mut x = vec![0.0_f64; self.ncols];
        for (piv_idx, &orig_col) in self.perm.iter().enumerate() {
            let z_val = z.get(piv_idx).copied().unwrap_or(0.0);
            if let Some(slot) = x.get_mut(orig_col) {
                *slot = z_val;
            }
        }
        Ok(x)
    }

    /// Compute the unscaled covariance inverse `(A^T A)^{-1} = P R^{-1} R^{-T} P^T` (`ncols x ncols`)
    /// directly via triangular back-substitution on `R` without forming `A^T A`.
    ///
    /// # Errors
    /// Returns [`MathError::SingularMatrix`] if `self.rank < self.ncols` or `nrows < ncols`.
    pub fn xtx_inverse(&self) -> Result<DenseMatrix, MathError> {
        if self.nrows < self.ncols || self.rank < self.ncols {
            return Err(MathError::SingularMatrix { rank: self.rank, expected_rank: self.ncols });
        }
        let p = self.ncols;
        let mut r_inv = DenseMatrix::zeros(p, p)?;
        for i in (0..p).rev() {
            let diag = self.factors.entry(i, i);
            r_inv.set_in_bounds(i, i, 1.0 / diag);
            for j in (i + 1)..p {
                let mut acc = NeumaierSum::new();
                for k in (i + 1)..=j {
                    acc.add(-self.factors.entry(i, k) * r_inv.entry(k, j));
                }
                r_inv.set_in_bounds(i, j, acc.total() / diag);
            }
        }
        // (R^T R)^{-1} = R^{-1} * R^{-T}, then unpermute rows and columns by `perm`.
        let rtr_inv_piv = r_inv.matmul(&r_inv.transpose())?;
        let mut unpermuted = DenseMatrix::zeros(p, p)?;
        for (i, &oi) in self.perm.iter().enumerate().take(p) {
            for (j, &oj) in self.perm.iter().enumerate().take(p) {
                unpermuted.set_in_bounds(oi, oj, rtr_inv_piv.entry(i, j));
            }
        }
        Ok(unpermuted)
    }
}

/// Compute nonlinear/linear regression parameter dependencies
/// `Dependency_k = 1 - 1 / ((A^T A)_{kk} * [(A^T A)^{-1}]_{kk})` in `[0, 1)`
/// per `docs/implementation/curve-fitting-implementation.md` §2.1.
///
/// # Errors
/// Returns [`MathError::SingularMatrix`] if `a` does not have full column rank.
pub fn parameter_dependencies(a: &DenseMatrix) -> Result<Vec<f64>, MathError> {
    let pqr = qr_col_pivoted(a)?;
    let xtx_inv = pqr.xtx_inverse()?;
    let mut deps = Vec::with_capacity(a.ncols());
    for k in 0..a.ncols() {
        let col_norm = a.col_norm2(k);
        let ata_kk = col_norm * col_norm;
        let inv_kk = xtx_inv.entry(k, k);
        let prod = ata_kk * inv_kk;
        let dep = if prod <= 1.0 { 0.0 } else { (1.0 - 1.0 / prod).clamp(0.0, 1.0) };
        deps.push(dep);
    }
    Ok(deps)
}

/// High-accuracy column-equilibrated linear least-squares solver `min_x ||X beta - y||_2`
/// (`statistics-implementation.md` §1.3):
/// 1. Equilibrates non-zero columns by Euclidean norm `D_j = ||X_{:, j}||_2` so multiscale / polynomial
///    Vandermonde design matrices (`Filip`, `Wampler1`-`Wampler5`) do not suffer artificial conditioning loss,
/// 2. Factors `X D^{-1}` via column-pivoted Householder QR (`qr_col_pivoted`) with one compensated
///    Bjorck residual-refinement pass when full rank,
/// 3. Falls back cleanly to minimum-norm SVD pseudoinverse when rank-deficient, reporting exact
///    `rank` and `aliased_columns`.
///
/// Also returns the unscaled covariance factor `(X^T X)^+` (`p x p`) so parameter standard errors
/// `SE_j = sqrt(sigma^2 * cov_unscaled[j, j])` can be read directly without squaring `X^T X`.
///
/// # Errors
/// Returns an error on dimension mismatch or non-finite inputs.
pub fn solve_linear_least_squares(x: &DenseMatrix, y: &[f64]) -> Result<(LeastSquaresSolution, DenseMatrix), MathError> {
    if x.nrows() != y.len() {
        return Err(MathError::DimensionMismatch { expected: x.nrows(), actual: y.len() });
    }
    if !x.all_finite() {
        return Err(MathError::DomainError { name: "X", value: f64::NAN });
    }
    for &yi in y {
        if !yi.is_finite() {
            return Err(MathError::DomainError { name: "y", value: yi });
        }
    }

    let n = x.nrows();
    let p = x.ncols();
    if p == 0 {
        let rss = sum_iter(y.iter().map(|&v| v * v));
        return Ok((
            LeastSquaresSolution { coefficients: Vec::new(), rank: 0, aliased_columns: Vec::new(), rss, df_residual: n },
            DenseMatrix::zeros(0, 0)?,
        ));
    }

    // Column equilibration X_scaled[:, j] = X[:, j] / scale[j].
    let mut scales = vec![1.0_f64; p];
    let mut x_scaled = DenseMatrix::zeros(n, p)?;
    for (j, scale_slot) in scales.iter_mut().enumerate().take(p) {
        let norm_j = x.col_norm2(j);
        if norm_j > 0.0 {
            *scale_slot = norm_j;
            let inv = 1.0 / norm_j;
            for i in 0..n {
                x_scaled.set_in_bounds(i, j, x.entry(i, j) * inv);
            }
        }
    }

    let pqr = qr_col_pivoted(&x_scaled)?;
    let rank = pqr.rank();
    let aliased = pqr.aliased_columns();

    let (beta, cov_unscaled, rss) = if rank == p && n >= p {
        let (beta_refined, rss_refined) = bjorck_refined_qr_solve(x, y, &scales, &pqr)?;
        let cov_s = pqr.xtx_inverse()?;
        let cov_u = unscale_covariance(&cov_s, &scales)?;
        (beta_refined, cov_u, rss_refined)
    } else {
        // Rank-deficient design matrix: fall back cleanly to SVD pseudoinverse (per §1.3).
        let svd_s = svd(&x_scaled)?;
        let z = svd_s.solve_least_squares(y, None)?;
        let cov_s = svd_s.ata_pseudoinverse(None)?;
        let beta_svd: Vec<f64> = z.iter().zip(scales.iter()).map(|(&zj, &sj)| if sj > 0.0 { zj / sj } else { 0.0 }).collect();
        let cov_u = unscale_covariance(&cov_s, &scales)?;
        let rss_svd = compensated_rss(x, y, &beta_svd);
        (beta_svd, cov_u, rss_svd)
    };

    let df_residual = n.saturating_sub(rank);

    Ok((LeastSquaresSolution { coefficients: beta, rank, aliased_columns: aliased, rss, df_residual }, cov_unscaled))
}

fn unscale_covariance(cov_scaled: &DenseMatrix, scales: &[f64]) -> Result<DenseMatrix, MathError> {
    let p = scales.len();
    let mut cov_unscaled = DenseMatrix::zeros(p, p)?;
    for (i, &si) in scales.iter().enumerate() {
        for (j, &sj) in scales.iter().enumerate() {
            let denom = si * sj;
            let v = if denom > 0.0 { cov_scaled.entry(i, j) / denom } else { 0.0 };
            cov_unscaled.set_in_bounds(i, j, v);
        }
    }
    Ok(cov_unscaled)
}

/// Compensated `Dot2` evaluation of `r_i = y_i - sum_j X_{i,j} beta_j` without intermediate rounding.
fn compensated_residual_vec(x: &DenseMatrix, y: &[f64], beta: &[f64]) -> Vec<f64> {
    let n = x.nrows();
    let p = x.ncols();
    let mut out = Vec::with_capacity(n);
    for (i, &yi) in y.iter().enumerate().take(n) {
        let mut main = NeumaierSum::new();
        let mut err = NeumaierSum::new();
        main.add(yi);
        for (j, &bj) in beta.iter().enumerate().take(p) {
            let xij = x.entry(i, j);
            let prod = -xij * bj;
            main.add(prod);
            err.add((-xij).mul_add(bj, -prod));
        }
        out.push(main.total() + err.total());
    }
    out
}

fn compensated_rss(x: &DenseMatrix, y: &[f64], beta: &[f64]) -> f64 {
    let res = compensated_residual_vec(x, y, beta);
    let mut main = NeumaierSum::new();
    let mut err = NeumaierSum::new();
    for r in res {
        let sq = r * r;
        main.add(sq);
        err.add(r.mul_add(r, -sq));
    }
    (main.total() + err.total()).max(0.0)
}

/// Bjorck (1967) mixed-precision augmented KKT iterative refinement using `Dot2` FMA inner products
/// against the unscaled matrix `X` so equilibration rounding and moderate-to-high condition numbers
/// retain near-full `f64` accuracy even when residuals are large (`Wampler1`-`Wampler5`).
fn bjorck_refined_qr_solve(x: &DenseMatrix, y: &[f64], scales: &[f64], pqr: &PivotedQr) -> Result<(Vec<f64>, f64), MathError> {
    let n = x.nrows();
    let p = x.ncols();
    let qty = pqr.apply_qt(y)?;
    let z_piv0 = pqr.solve_r_full(qty.get(..p).unwrap_or(&[]));
    let mut z = vec![0.0_f64; p];
    for (piv_idx, &orig_col) in pqr.perm.iter().enumerate() {
        if let (Some(slot), Some(&val)) = (z.get_mut(orig_col), z_piv0.get(piv_idx)) {
            *slot = val;
        }
    }
    let mut q_rhs0 = vec![0.0_f64; n];
    if let (Some(dst), Some(src)) = (q_rhs0.get_mut(p..n), qty.get(p..n)) {
        dst.copy_from_slice(src);
    }
    let mut r_vec = pqr.apply_q_vec(&q_rhs0)?;

    for _ in 0..3 {
        let beta_curr: Vec<f64> = z.iter().zip(scales.iter()).map(|(&zj, &sj)| zj / sj).collect();
        if !beta_curr.iter().all(|v| v.is_finite()) {
            break;
        }
        // Primal defect: f = y - r - X * beta_curr (evaluated in Dot2 precision).
        let mut f_def = Vec::with_capacity(n);
        for i in 0..n {
            let mut main = NeumaierSum::new();
            let mut err = NeumaierSum::new();
            main.add(y.get(i).copied().unwrap_or(0.0));
            main.add(-r_vec.get(i).copied().unwrap_or(0.0));
            for (j, &bj) in beta_curr.iter().enumerate() {
                let xij = x.entry(i, j);
                let prod = -xij * bj;
                main.add(prod);
                err.add((-xij).mul_add(bj, -prod));
            }
            f_def.push(main.total() + err.total());
        }

        // Dual defect: g_j = -(X^T r)_j / scales[j] (evaluated in Dot2 precision).
        let mut g_def = vec![0.0_f64; p];
        for (j, (g_slot, &sj)) in g_def.iter_mut().zip(scales.iter()).enumerate() {
            let mut main = NeumaierSum::new();
            let mut err = NeumaierSum::new();
            for (i, &ri) in r_vec.iter().enumerate() {
                let xij = x.entry(i, j);
                let prod = -xij * ri;
                main.add(prod);
                err.add((-xij).mul_add(ri, -prod));
            }
            *g_slot = (main.total() + err.total()) / sj;
        }
        if !f_def.iter().all(|v| v.is_finite()) || !g_def.iter().all(|v| v.is_finite()) {
            break;
        }

        let g_piv: Vec<f64> = pqr.perm.iter().map(|&orig| g_def.get(orig).copied().unwrap_or(0.0)).collect();
        let u1 = pqr.solve_rt_full(&g_piv);
        let c = pqr.apply_qt(&f_def)?;
        let mut u_full = vec![0.0_f64; n];
        if let Some(dst) = u_full.get_mut(..p) {
            dst.copy_from_slice(&u1);
        }
        if let (Some(dst), Some(src)) = (u_full.get_mut(p..n), c.get(p..n)) {
            dst.copy_from_slice(src);
        }
        let dr = pqr.apply_q_vec(&u_full)?;

        let rhs_dz: Vec<f64> = c.get(..p).unwrap_or(&[]).iter().zip(u1.iter()).map(|(&ci, &ui)| ci - ui).collect();
        let dz_piv = pqr.solve_r_full(&rhs_dz);

        // Guard against non-finite refinement steps so `z` and `r_vec` are never corrupted.
        if !dr.iter().all(|v| v.is_finite()) || !dz_piv.iter().all(|v| v.is_finite()) {
            break;
        }
        let steps_keep_finite = pqr.perm.iter().enumerate().all(|(piv_idx, &orig_col)| {
            let z_cur = z.get(orig_col).copied().unwrap_or(0.0);
            let step = dz_piv.get(piv_idx).copied().unwrap_or(0.0);
            (z_cur + step).is_finite()
        }) && r_vec.iter().zip(dr.iter()).all(|(&ri, &dri)| (ri + dri).is_finite());
        if !steps_keep_finite {
            break;
        }

        let mut max_rel = 0.0_f64;
        for (piv_idx, &orig_col) in pqr.perm.iter().enumerate() {
            let step = dz_piv.get(piv_idx).copied().unwrap_or(0.0);
            if let Some(slot) = z.get_mut(orig_col) {
                *slot += step;
                let denom = slot.abs().max(1e-30);
                max_rel = max_rel.max(step.abs() / denom);
            }
        }
        for (ri, &dri) in r_vec.iter_mut().zip(dr.iter()) {
            *ri += dri;
        }
        if max_rel <= f64::EPSILON {
            break;
        }
    }

    let beta_final: Vec<f64> = z.iter().zip(scales.iter()).map(|(&zj, &sj)| zj / sj).collect();
    let rss = compensated_rss(x, y, &beta_final);
    Ok((beta_final, rss))
}

#[cfg(test)]
mod tests {
    use super::{parameter_dependencies, qr, qr_col_pivoted, solve_linear_least_squares};
    use crate::linalg::matrix::DenseMatrix;

    #[test]
    fn qr_and_pivoted_qr_reconstruct_a_and_detect_collinearity() {
        let a = DenseMatrix::from_row_major(
            4,
            3,
            vec![
                1.0, 2.0, 3.0, //
                4.0, 5.0, 6.0, //
                7.0, 8.0, 10.0, //
                2.0, 1.0, 0.0,
            ],
        )
        .unwrap();
        let uqr = qr(&a).unwrap();
        let q = uqr.thin_q().unwrap();
        let r = uqr.r().unwrap();
        let recon = q.matmul(&r).unwrap();
        for i in 0..4 {
            for j in 0..3 {
                assert!((recon.entry(i, j) - a.entry(i, j)).abs() < 1e-14);
            }
        }

        // Exact collinear column: col 2 == 2 * col 0 + col 1.
        let sing = DenseMatrix::from_row_major(
            5,
            3,
            vec![
                1.0, 1.0, 3.0, //
                2.0, 0.0, 4.0, //
                3.0, -1.0, 5.0, //
                4.0, 2.0, 10.0, //
                5.0, 3.0, 13.0,
            ],
        )
        .unwrap();
        let pqr = qr_col_pivoted(&sing).unwrap();
        assert_eq!(pqr.rank(), 2);
        assert!(!pqr.is_full_rank());
        assert_eq!(pqr.aliased_columns().len(), 1);

        // Fallback least-squares must succeed without dividing by zero or panicking!
        let y = [4.0, 4.0, 4.0, 12.0, 16.0];
        let (sol, _cov) = solve_linear_least_squares(&sing, &y).unwrap();
        assert_eq!(sol.rank, 2);
        assert!(sol.rss < 1e-24);

        let deps = parameter_dependencies(&a).unwrap();
        assert_eq!(deps.len(), 3);
        assert!(deps.iter().all(|&d| (0.0..1.0).contains(&d)));
    }

    #[test]
    fn unpivoted_qr_handles_leading_zero_column_rank_and_refinement_stays_finite() {
        let leading_zero = DenseMatrix::from_row_major(2, 2, vec![0.0, 1.0, 0.0, 2.0]).unwrap();
        let uqr = qr(&leading_zero).unwrap();
        assert_eq!(uqr.rank(), 1);
        assert!(!uqr.is_full_rank());
        assert_eq!(uqr.aliased_columns(), vec![0]);

        let q = uqr.full_q().unwrap();
        let r = uqr.r().unwrap();
        let recon = q.matmul(&r).unwrap();
        for i in 0..2 {
            for j in 0..2 {
                assert!((recon.entry(i, j) - leading_zero.entry(i, j)).abs() < 1e-14);
            }
        }
        let basic = uqr.solve_basic(&[3.0, 6.0]).unwrap();
        assert!(basic.iter().all(|v| v.is_finite()));
        assert_eq!(basic[0], 0.0);

        let pqr = qr_col_pivoted(&leading_zero).unwrap();
        assert_eq!(pqr.rank(), 1);
        assert_eq!(pqr.aliased_columns(), vec![0]);
    }
}
