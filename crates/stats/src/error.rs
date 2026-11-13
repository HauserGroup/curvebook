//! Structured error types for `lumencraft-stats`.

use std::fmt;

use lumencraft_doc::{DocError, SummaryPointError, TableKind};
use lumencraft_math::MathError;

/// Errors returned by statistical computation engines and `ResultSheet` builders.
#[derive(Debug, Clone, PartialEq)]
pub enum StatsError {
    EmptyInput,
    InsufficientSampleSize { context: &'static str, required: usize, actual: usize },
    InvalidParameter { name: &'static str, value: f64 },
    DimensionMismatch { expected: usize, actual: usize },
    RequiresRawReplicates,
    IncompatibleTable { analysis: &'static str, actual: TableKind },
    IncompatibleAnalysisSpec { expected: &'static str },
    ZeroVariance { context: &'static str },
    MissingColumn { index: usize },
    NotConverged { algorithm: &'static str },
    SummaryPoint(SummaryPointError),
    Math(MathError),
    Doc(DocError),
}

impl fmt::Display for StatsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyInput => f.write_str("input dataset contains no valid observations"),
            Self::InsufficientSampleSize { context, required, actual } => {
                write!(f, "{context}: requires at least {required} observations, got {actual}")
            }
            Self::InvalidParameter { name, value } => {
                write!(f, "invalid parameter `{name}` = {value}")
            }
            Self::DimensionMismatch { expected, actual } => {
                write!(f, "dimension mismatch: expected {expected}, got {actual}")
            }
            Self::RequiresRawReplicates => f.write_str("analysis requires raw replicate observations rather than pre-summarized points"),
            Self::IncompatibleTable { analysis, actual } => {
                write!(f, "{analysis} is not compatible with {actual:?} tables")
            }
            Self::IncompatibleAnalysisSpec { expected } => {
                write!(f, "analysis specification does not match expected family `{expected}`")
            }
            Self::ZeroVariance { context } => {
                write!(f, "{context}: sample variance is zero")
            }
            Self::MissingColumn { index } => {
                write!(f, "requested column index {index} does not exist")
            }
            Self::NotConverged { algorithm } => {
                write!(f, "iterative algorithm `{algorithm}` failed to converge")
            }
            Self::SummaryPoint(err) => write!(f, "pre-summarized cell error: {err}"),
            Self::Math(err) => write!(f, "numerical error: {err}"),
            Self::Doc(err) => write!(f, "document error: {err}"),
        }
    }
}

impl std::error::Error for StatsError {}

impl From<MathError> for StatsError {
    fn from(value: MathError) -> Self {
        Self::Math(value)
    }
}

impl From<DocError> for StatsError {
    fn from(value: DocError) -> Self {
        Self::Doc(value)
    }
}

impl From<SummaryPointError> for StatsError {
    fn from(value: SummaryPointError) -> Self {
        Self::SummaryPoint(value)
    }
}
