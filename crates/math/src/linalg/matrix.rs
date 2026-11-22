//! Safe, bounds-checked dense matrix representation (`DenseMatrix`) in row-major layout with
//! compensated inner-product operations.

use crate::error::MathError;
use crate::sum::{NeumaierSum, dot};

/// Maximum total element count (`nrows * ncols`) permitted in a [`DenseMatrix`].
pub const MAX_MATRIX_ELEMENTS: usize = 100_000_000;

/// Dense 2D `f64` matrix (`nrows x ncols`) stored contiguously in row-major order.
#[derive(Debug, Clone, PartialEq)]
pub struct DenseMatrix {
    nrows: usize,
    ncols: usize,
    data: Vec<f64>,
}

impl DenseMatrix {
    /// Allocate an `nrows x ncols` zero matrix.
    ///
    /// # Errors
    /// Returns [`MathError::InvalidParameter`] if `nrows * ncols` overflows `usize` or exceeds [`MAX_MATRIX_ELEMENTS`].
    pub fn zeros(nrows: usize, ncols: usize) -> Result<Self, MathError> {
        let Some(len) = nrows.checked_mul(ncols) else {
            return Err(MathError::InvalidParameter { name: "matrix_elements", value: f64::INFINITY });
        };
        if len > MAX_MATRIX_ELEMENTS {
            return Err(MathError::InvalidParameter { name: "matrix_elements", value: len as f64 });
        }
        Ok(Self { nrows, ncols, data: vec![0.0; len] })
    }

    /// Construct the `n x n` identity matrix `I_n`.
    ///
    /// # Errors
    /// Returns an error if `n * n` exceeds allocation bounds.
    pub fn identity(n: usize) -> Result<Self, MathError> {
        let mut m = Self::zeros(n, n)?;
        for i in 0..n {
            m.set_in_bounds(i, i, 1.0);
        }
        Ok(m)
    }

    /// Construct an `nrows x ncols` matrix from row-major `data`.
    ///
    /// # Errors
    /// Returns [`MathError::InvalidParameter`] if `nrows * ncols` overflows or exceeds
    /// [`MAX_MATRIX_ELEMENTS`], or [`MathError::DimensionMismatch`] if `data.len() != nrows * ncols`.
    pub fn from_row_major(nrows: usize, ncols: usize, data: Vec<f64>) -> Result<Self, MathError> {
        let Some(expected) = nrows.checked_mul(ncols) else {
            return Err(MathError::InvalidParameter { name: "matrix_elements", value: f64::INFINITY });
        };
        if expected > MAX_MATRIX_ELEMENTS {
            return Err(MathError::InvalidParameter { name: "matrix_elements", value: expected as f64 });
        }
        if data.len() != expected {
            return Err(MathError::DimensionMismatch { expected, actual: data.len() });
        }
        Ok(Self { nrows, ncols, data })
    }

    /// Construct an `nrows x ncols` matrix from column-major `cols` (`nrows * ncols` elements).
    ///
    /// # Errors
    /// Returns [`MathError::DimensionMismatch`] if `cols.len() != nrows * ncols`.
    pub fn from_col_major(nrows: usize, ncols: usize, cols: &[f64]) -> Result<Self, MathError> {
        let mut out = Self::zeros(nrows, ncols)?;
        if cols.len() != out.data.len() {
            return Err(MathError::DimensionMismatch { expected: out.data.len(), actual: cols.len() });
        }
        for c in 0..ncols {
            for r in 0..nrows {
                if let Some(&val) = cols.get(c * nrows + r) {
                    out.set_in_bounds(r, c, val);
                }
            }
        }
        Ok(out)
    }

    /// Construct an `nrows x ncols` matrix populated by `f(row, col)`.
    ///
    /// # Errors
    /// Returns an error if `nrows * ncols` exceeds allocation bounds.
    pub fn from_fn<F: FnMut(usize, usize) -> f64>(nrows: usize, ncols: usize, mut f: F) -> Result<Self, MathError> {
        let mut out = Self::zeros(nrows, ncols)?;
        for r in 0..nrows {
            for c in 0..ncols {
                out.set_in_bounds(r, c, f(r, c));
            }
        }
        Ok(out)
    }

    /// Number of rows `m`.
    #[must_use]
    #[inline]
    pub const fn nrows(&self) -> usize {
        self.nrows
    }

    /// Number of columns `n`.
    #[must_use]
    #[inline]
    pub const fn ncols(&self) -> usize {
        self.ncols
    }

    /// Underlying row-major slice.
    #[must_use]
    #[inline]
    pub fn as_slice(&self) -> &[f64] {
        &self.data
    }

    /// Bounds-checked element read `A[r, c]`.
    #[must_use]
    #[inline]
    pub fn get(&self, r: usize, c: usize) -> Option<f64> {
        if r < self.nrows && c < self.ncols { self.data.get(r * self.ncols + c).copied() } else { None }
    }

    /// Bounds-checked mutable element reference `&mut A[r, c]`.
    #[must_use]
    #[inline]
    pub fn get_mut(&mut self, r: usize, c: usize) -> Option<&mut f64> {
        if r < self.nrows && c < self.ncols { self.data.get_mut(r * self.ncols + c) } else { None }
    }

    /// Bounds-checked element write `A[r, c] = val`.
    ///
    /// # Errors
    /// Returns [`MathError::DimensionMismatch`] if `(r, c)` is out of bounds.
    pub fn set(&mut self, r: usize, c: usize, val: f64) -> Result<(), MathError> {
        let ncols = self.ncols;
        let nrows = self.nrows;
        let Some(slot) = self.get_mut(r, c) else {
            return Err(MathError::DimensionMismatch { expected: nrows * ncols, actual: r.saturating_mul(ncols).saturating_add(c) });
        };
        *slot = val;
        Ok(())
    }

    /// Internal fast-path read returning `0.0` only if out of bounds.
    #[inline]
    pub(crate) fn entry(&self, r: usize, c: usize) -> f64 {
        self.get(r, c).unwrap_or(0.0)
    }

    /// Internal fast-path write when `(r, c)` is verified in bounds.
    #[inline]
    pub(crate) fn set_in_bounds(&mut self, r: usize, c: usize, val: f64) {
        if let Some(slot) = self.get_mut(r, c) {
            *slot = val;
        }
    }

    /// Borrow row `r` as a contiguous slice.
    #[must_use]
    #[inline]
    pub fn row(&self, r: usize) -> Option<&[f64]> {
        if r < self.nrows {
            let start = r * self.ncols;
            self.data.get(start..start + self.ncols)
        } else {
            None
        }
    }

    /// Copy column `c` into a `Vec<f64>`.
    #[must_use]
    pub fn column_vec(&self, c: usize) -> Option<Vec<f64>> {
        if c >= self.ncols {
            return None;
        }
        let mut col = Vec::with_capacity(self.nrows);
        for r in 0..self.nrows {
            col.push(self.entry(r, c));
        }
        Some(col)
    }

    /// Check whether every element of `self` is finite (`!is_nan() && !is_infinite()`).
    #[must_use]
    pub fn all_finite(&self) -> bool {
        self.data.iter().all(|v| v.is_finite())
    }

    /// Matrix transpose `A^T` (`ncols x nrows`).
    #[must_use]
    pub fn transpose(&self) -> Self {
        let mut out = Self { nrows: self.ncols, ncols: self.nrows, data: vec![0.0; self.data.len()] };
        for r in 0..self.nrows {
            for c in 0..self.ncols {
                out.set_in_bounds(c, r, self.entry(r, c));
            }
        }
        out
    }

    /// Matrix-vector product `y = A * x` using compensated FMA dot products per row.
    ///
    /// # Errors
    /// Returns [`MathError::DimensionMismatch`] if `x.len() != self.ncols`.
    pub fn matvec(&self, x: &[f64]) -> Result<Vec<f64>, MathError> {
        if x.len() != self.ncols {
            return Err(MathError::DimensionMismatch { expected: self.ncols, actual: x.len() });
        }
        let mut out = Vec::with_capacity(self.nrows);
        for r in 0..self.nrows {
            let row_slice = self.row(r).unwrap_or(&[]);
            out.push(dot(row_slice, x)?);
        }
        Ok(out)
    }

    /// Transposed matrix-vector product `y = A^T * x` using compensated FMA accumulation.
    ///
    /// # Errors
    /// Returns [`MathError::DimensionMismatch`] if `x.len() != self.nrows`.
    pub fn t_matvec(&self, x: &[f64]) -> Result<Vec<f64>, MathError> {
        if x.len() != self.nrows {
            return Err(MathError::DimensionMismatch { expected: self.nrows, actual: x.len() });
        }
        let mut out = Vec::with_capacity(self.ncols);
        for c in 0..self.ncols {
            let mut prod_acc = NeumaierSum::new();
            let mut err_acc = NeumaierSum::new();
            for (r, &xr) in x.iter().enumerate() {
                let a = self.entry(r, c);
                let p = a * xr;
                if p.is_finite() {
                    prod_acc.add(p);
                    err_acc.add(a.mul_add(xr, -p));
                } else {
                    prod_acc.add(p);
                }
            }
            out.push(prod_acc.total() + err_acc.total());
        }
        Ok(out)
    }

    /// Matrix-matrix product `C = self * rhs` (`(m x k) * (k x n) -> (m x n)`).
    ///
    /// # Errors
    /// Returns [`MathError::DimensionMismatch`] if `self.ncols != rhs.nrows`.
    pub fn matmul(&self, rhs: &Self) -> Result<Self, MathError> {
        if self.ncols != rhs.nrows {
            return Err(MathError::DimensionMismatch { expected: self.ncols, actual: rhs.nrows });
        }
        let rhs_t = rhs.transpose();
        let mut out = Self::zeros(self.nrows, rhs.ncols)?;
        for r in 0..self.nrows {
            let row_a = self.row(r).unwrap_or(&[]);
            for c in 0..rhs.ncols {
                let col_b = rhs_t.row(c).unwrap_or(&[]);
                out.set_in_bounds(r, c, dot(row_a, col_b)?);
            }
        }
        Ok(out)
    }

    /// Overflow-safe and underflow-safe Euclidean 2-norm of column `col` over rows `row_start..nrows`.
    #[must_use]
    pub fn col_norm2_from(&self, col: usize, row_start: usize) -> f64 {
        if col >= self.ncols || row_start >= self.nrows {
            return 0.0;
        }
        let mut scale = 0.0_f64;
        for r in row_start..self.nrows {
            scale = scale.max(self.entry(r, col).abs());
        }
        if scale == 0.0 || !scale.is_finite() {
            return scale;
        }
        let subnormal_scale = scale < f64::MIN_POSITIVE;
        let inv = if subnormal_scale { 0.0 } else { 1.0 / scale };
        let mut acc = NeumaierSum::new();
        for r in row_start..self.nrows {
            let v = self.entry(r, col);
            let u = if subnormal_scale { v / scale } else { v * inv };
            let sq = u * u;
            acc.add(sq);
            acc.add(u.mul_add(u, -sq));
        }
        scale * acc.total().max(0.0).sqrt()
    }

    /// Euclidean 2-norm of full column `col`.
    #[must_use]
    pub fn col_norm2(&self, col: usize) -> f64 {
        self.col_norm2_from(col, 0)
    }

    /// Frobenius norm `sqrt(sum_{r, c} A[r, c]^2)`.
    #[must_use]
    pub fn frobenius_norm(&self) -> f64 {
        let mut scale = 0.0_f64;
        for &v in &self.data {
            scale = scale.max(v.abs());
        }
        if scale == 0.0 || !scale.is_finite() {
            return scale;
        }
        let subnormal_scale = scale < f64::MIN_POSITIVE;
        let inv = if subnormal_scale { 0.0 } else { 1.0 / scale };
        let mut acc = NeumaierSum::new();
        for &v in &self.data {
            let u = if subnormal_scale { v / scale } else { v * inv };
            acc.add(u * u);
        }
        scale * acc.total().max(0.0).sqrt()
    }

    /// Maximum element magnitude `max_{r, c} |A[r, c]|` (`0.0` if empty).
    #[must_use]
    pub(crate) fn max_abs(&self) -> f64 {
        self.data.iter().fold(0.0_f64, |acc, &v| acc.max(v.abs()))
    }
}

/// Power-of-two exponent `exp` for exact matrix scaling (`A * 2^{-exp}`) when entries sit outside
/// `[1e-100, 1e100]`, returning `0` inside the safe middle exponent band.
#[inline]
pub(crate) fn pow2_exp_for_max_abs(max_abs: f64) -> i32 {
    if max_abs == 0.0 || !max_abs.is_finite() || (max_abs > 1e-100 && max_abs < 1e100) {
        return 0;
    }
    (max_abs.log2().floor() as i32).clamp(-1020, 1020)
}

/// Multiply finite `v` by `2^{exp}` without intermediate `2^{exp}` overflow/underflow for `|exp| <= 2048`.
#[inline]
pub(crate) fn scale_pow2(mut v: f64, mut exp: i32) -> f64 {
    if v == 0.0 || !v.is_finite() || exp == 0 {
        return v;
    }
    while exp > 1000 {
        v *= 2.0_f64.powi(1000);
        exp -= 1000;
    }
    while exp < -1000 {
        v *= 2.0_f64.powi(-1000);
        exp += 1000;
    }
    v * 2.0_f64.powi(exp)
}

#[cfg(test)]
mod tests {
    use super::{DenseMatrix, MAX_MATRIX_ELEMENTS};
    use crate::error::MathError;

    #[test]
    fn from_row_major_and_zeros_enforce_max_matrix_elements() {
        let side = 10_001;
        assert!(side * side > MAX_MATRIX_ELEMENTS);
        assert!(matches!(DenseMatrix::zeros(side, side), Err(MathError::InvalidParameter { name: "matrix_elements", .. })));
        assert!(matches!(DenseMatrix::from_row_major(side, side, Vec::new()), Err(MathError::InvalidParameter { name: "matrix_elements", .. })));
    }
}
