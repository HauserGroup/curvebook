//! Structured error types for `curvebook-stats`.

use std::fmt;

use curvebook_doc::{DocError, SummaryPointError, TableKind};
use curvebook_math::MathError;

/// Errors returned by statistical computation engines and `ResultSheet` builders.
#[derive(Debug, Clone, PartialEq)]
pub enum StatsError {
    /// Analysis received an empty dataset or zero complete finite cases after filtering exclusions/missing cells.
    EmptyInput,
    /// Too few active observations remained to satisfy minimum model/test sample size (`n < required`).
    InsufficientSampleSize {
        /// Descriptive name of the statistical procedure or sub-step enforcing the minimum.
        context: &'static str,
        /// Minimum required number of observations or levels.
        required: usize,
        /// Actual number of valid observations supplied.
        actual: usize,
    },
    /// Invalid numerical parameter, option combination, or domain violation (e.g. non-finite or out-of-range configuration).
    InvalidParameter {
        /// Identifier of the offending parameter or configuration field.
        name: &'static str,
        /// Numerical value observed (`0.0` or count when indexing integer structural bounds).
        value: f64,
    },
    /// Input arrays, design matrices, or paired series differ in length where exact alignment is required.
    DimensionMismatch {
        /// Expected vector/row/column length.
        expected: usize,
        /// Actual supplied length.
        actual: usize,
    },
    /// Selected procedure (e.g. rank test, normality battery, ROUT, PCA) requires individual replicate cells rather than pre-summarized moments.
    RequiresRawReplicates,
    /// Input `DataSheet` archetype does not support the requested analysis.
    IncompatibleTable {
        /// Name of the requested statistical analysis.
        analysis: &'static str,
        /// Table archetype actually present on the input `DataSheet`.
        actual: TableKind,
    },
    /// `AnalysisSpec` variant passed to an internal dispatcher did not match the expected analysis kind.
    IncompatibleAnalysisSpec {
        /// Name of the expected `AnalysisKind` variant.
        expected: &'static str,
    },
    /// Sample variance or difference variance equals zero where strict positivity is needed (e.g. denominator of Student *t*).
    ZeroVariance {
        /// Name of the routine encountering zero variance.
        context: &'static str,
    },
    /// Referenced dataset, predictor, response, or factor column index is out of bounds for the source table.
    MissingColumn {
        /// Zero-based column/dataset index requested by the caller or spec.
        index: usize,
    },
    /// Iterative optimization (e.g. REML, IRLS logistic/Poisson, Cox NR) failed to reach convergence tolerance.
    NotConverged {
        /// Name of the iterative algorithm that failed to converge.
        algorithm: &'static str,
    },
    /// Error reconstructing `(Mean, SD/SEM/%CV, N)` summarized point statistics (`SummaryPointError`).
    SummaryPoint(SummaryPointError),
    /// Lower-level numerical/special-function/distribution failure propagated from `curvebook-math`.
    Math(MathError),
    /// Document validation or sheet-schema error propagated from `curvebook-doc`.
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
