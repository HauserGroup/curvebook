//! Dense linear algebra primitives for design matrices, nonlinear regression Jacobians, and REML
//! covariance structures:
//! - [`DenseMatrix`] bounds-checked dense matrix with compensated `Dot2` inner products,
//! - [`qr()`] and rank-revealing [`qr_col_pivoted`] Householder factorizations (`A P = Q R`),
//! - [`solve_linear_least_squares`] column-equilibrated rank-safe QR/SVD least-squares solver,
//! - [`parameter_dependencies`] multicollinearity diagnostics,
//! - [`svd()`] thin Singular Value Decomposition (`A = U diag(sigma) V^T`) and pseudoinverses,
//! - [`cholesky()`] symmetric positive-definite factorization (`A = L L^T`),
//! - [`sym_eigen`] real symmetric eigenvalue decomposition (`A = Q Lambda Q^T`).

pub mod cholesky;
pub mod matrix;
pub mod qr;
pub mod svd;

pub use cholesky::{Cholesky, SymEigen, cholesky, sym_eigen};
pub use matrix::DenseMatrix;
pub use qr::{LeastSquaresSolution, PivotedQr, parameter_dependencies, qr, qr_col_pivoted, solve_linear_least_squares};
pub use svd::{Svd, svd};
