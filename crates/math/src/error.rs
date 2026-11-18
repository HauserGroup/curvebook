//! Structured error types for `curvebook-math`.

use std::fmt;

/// Domain, parameter, dimension, or convergence error returned by `curvebook-math` routines.
#[derive(Debug, Clone, PartialEq)]
pub enum MathError {
    /// Input slice or iterator contained zero usable observations.
    EmptyInput,
    /// Vector or matrix dimensions were incompatible for the requested operation.
    DimensionMismatch {
        /// Required length or row/column count for the operation.
        expected: usize,
        /// Actual length or row/column count supplied by the caller.
        actual: usize,
    },
    /// Distribution or structural parameter lay outside its valid mathematical domain.
    InvalidParameter {
        /// Canonical name of the invalid parameter (for example `"df"`, `"alpha"`, or `"confidence_percent"`).
        name: &'static str,
        /// Offending floating-point value (may be `NaN` or `±Inf` when non-finite).
        value: f64,
    },
    /// Evaluation argument lay outside the function's domain (for instance `NaN` or `p` outside `[0, 1]`).
    DomainError {
        /// Canonical name of the evaluation argument (for example `"x"`, `"p"`, or `"bracket"`).
        name: &'static str,
        /// Offending argument value (may be `NaN` or `±Inf` when non-finite).
        value: f64,
    },
    /// Matrix was singular or rank-deficient when a full-rank system was required.
    SingularMatrix {
        /// Numerical rank detected under the factorization's pivot tolerance.
        rank: usize,
        /// Full rank required to solve the system uniquely.
        expected_rank: usize,
    },
    /// Symmetric matrix was not positive definite during Cholesky factorization.
    NotPositiveDefinite {
        /// Zero-based leading principal minor index `i` where positive definiteness failed.
        index: usize,
        /// Schur-complement diagonal value `A[i, i] - sum_{k < i} L[i, k]^2` that fell below the positive pivot threshold.
        pivot: f64,
    },
    /// Iterative numerical solver did not converge within its iteration budget.
    NotConverged {
        /// Identifier of the iterative kernel (for example `"brent_root"` or `"integrate_adaptive"`).
        algorithm: &'static str,
        /// Iteration or subdivision cap exhausted before the convergence criterion was satisfied.
        iterations: usize,
    },
}

impl fmt::Display for MathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyInput => f.write_str("input contained zero elements"),
            Self::DimensionMismatch { expected, actual } => {
                write!(f, "dimension mismatch: expected {expected}, got {actual}")
            }
            Self::InvalidParameter { name, value } => {
                write!(f, "invalid parameter `{name}` = {value}")
            }
            Self::DomainError { name, value } => {
                write!(f, "argument `{name}` = {value} is outside domain")
            }
            Self::SingularMatrix { rank, expected_rank } => {
                write!(f, "singular or rank-deficient matrix: numerical rank {rank} < {expected_rank}")
            }
            Self::NotPositiveDefinite { index, pivot } => {
                write!(f, "matrix is not positive definite at leading minor {index} (pivot = {pivot})")
            }
            Self::NotConverged { algorithm, iterations } => {
                write!(f, "{algorithm} did not converge within {iterations} iterations")
            }
        }
    }
}

impl std::error::Error for MathError {}
