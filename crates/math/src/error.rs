//! Structured error types for `lumencraft-math`.

use std::fmt;

/// Domain, parameter, dimension, or convergence error returned by `lumencraft-math` routines.
#[derive(Debug, Clone, PartialEq)]
pub enum MathError {
    /// Input slice or iterator contained zero usable observations.
    EmptyInput,
    /// Vector or matrix dimensions were incompatible for the requested operation.
    DimensionMismatch { expected: usize, actual: usize },
    /// Distribution or structural parameter lay outside its valid mathematical domain.
    InvalidParameter { name: &'static str, value: f64 },
    /// Evaluation argument lay outside the function's domain (for instance `NaN` or `p` outside `[0, 1]`).
    DomainError { name: &'static str, value: f64 },
    /// Matrix was singular or rank-deficient when a full-rank system was required.
    SingularMatrix { rank: usize, expected_rank: usize },
    /// Symmetric matrix was not positive definite during Cholesky factorization.
    NotPositiveDefinite { index: usize, pivot: f64 },
    /// Iterative numerical solver did not converge within its iteration budget.
    NotConverged { algorithm: &'static str, iterations: usize },
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
