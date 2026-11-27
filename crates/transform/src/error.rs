//! Structured error types for `curvebook-transform`.

use std::fmt;

use curvebook_doc::{DocError, SubcolumnMode, SummaryPointError, TableKind};
use curvebook_math::MathError;

/// Error produced by formula parsing/evaluation, table transformations, curve calculus, or imports.
#[derive(Debug, Clone, PartialEq)]
pub enum TransformError {
    /// Document-layer validation or mutation error.
    Doc(DocError),
    /// Numerical kernel domain or dimension error.
    Math(MathError),
    /// Pre-summarized `(Mean, Spread, N)` reconstruction failure.
    SummaryPoint(SummaryPointError),
    /// Cycle detected across calculated variables or linked data regions.
    CircularDependency {
        /// Ordered chain of variable or sheet identifiers forming the dependency cycle.
        cycle: Vec<String>,
    },
    /// Syntax error in a formula or expression string.
    ParseError {
        /// Zero-based UTF-8 byte offset in the input formula where parsing failed.
        position: usize,
        /// Human-readable syntax diagnostic.
        message: String,
    },
    /// Runtime error during formula evaluation or mathematical transform.
    EvalError {
        /// Human-readable evaluation diagnostic.
        message: String,
    },
    /// Reference to an unknown column, variable, or constant.
    UnknownVariable {
        /// Unresolved identifier or column name.
        name: String,
    },
    /// Reference to an unknown function name.
    UnknownFunction {
        /// Unrecognized function identifier.
        name: String,
    },
    /// Function called with an unsupported number of arguments.
    ArityMismatch {
        /// Canonical name of the called function.
        function: String,
        /// Human-readable description of accepted argument count(s) (for example `"2"` or `"1..=3"`).
        expected: String,
        /// Actual argument count passed at the call site.
        actual: usize,
    },
    /// Invalid transform configuration or parameter value.
    InvalidSpec {
        /// Explanation of why the transform specification is invalid.
        message: String,
    },
    /// Table archetype cannot support the requested transformation.
    IncompatibleTable {
        /// Static name of the attempted transform or curve-calculus operation.
        operation: &'static str,
        /// Actual [`TableKind`] of the rejected input sheet.
        kind: TableKind,
    },
    /// Transform or curve operation does not support the input table's pre-summarized [`SubcolumnMode`].
    UnsupportedSubcolumnMode {
        /// Static name of the attempted transform operation.
        operation: &'static str,
        /// Input table archetype (`Column`, `Xy`, or `Grouped`).
        table_kind: TableKind,
        /// Pre-summarized subcolumn layout encountered on the input sheet.
        mode: SubcolumnMode,
        /// Neutral explanation of why this mode is refused.
        reason: &'static str,
    },
    /// Referenced `HookSource` could not be resolved to a finite numeric constant.
    MissingHook {
        /// Diagnostic describing which upstream sheet, constant, or scalar failed to resolve.
        description: String,
    },
    /// Unrecognized equation identifier in `Simulate`.
    UnknownEquation {
        /// Unrecognized built-in equation ID string.
        equation_id: String,
    },
    /// Invalid or insufficient input data for curve calculus or import.
    InvalidInput {
        /// Explanation of why the input points or payload could not be processed.
        message: String,
    },
}

impl fmt::Display for TransformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Doc(e) => write!(f, "document error: {e}"),
            Self::Math(e) => write!(f, "math error: {e}"),
            Self::SummaryPoint(e) => write!(f, "summary point error: {e}"),
            Self::CircularDependency { cycle } => {
                write!(f, "circular dependency detected: {}", cycle.join(" -> "))
            }
            Self::ParseError { position, message } => {
                write!(f, "formula parse error at byte {position}: {message}")
            }
            Self::EvalError { message } => write!(f, "evaluation error: {message}"),
            Self::UnknownVariable { name } => write!(f, "unknown variable or column `{name}`"),
            Self::UnknownFunction { name } => write!(f, "unknown function `{name}`"),
            Self::ArityMismatch { function, expected, actual } => {
                write!(f, "function `{function}` expects {expected} argument(s), got {actual}")
            }
            Self::InvalidSpec { message } => write!(f, "invalid transform specification: {message}"),
            Self::IncompatibleTable { operation, kind } => {
                write!(f, "operation `{operation}` is not compatible with {kind:?} tables")
            }
            Self::UnsupportedSubcolumnMode { operation, table_kind, mode, reason } => {
                write!(f, "operation `{operation}` does not support {mode:?} subcolumns on {table_kind:?} tables: {reason}")
            }
            Self::MissingHook { description } => write!(f, "missing hooked constant: {description}"),
            Self::UnknownEquation { equation_id } => write!(f, "unknown simulation equation `{equation_id}`"),
            Self::InvalidInput { message } => write!(f, "invalid input: {message}"),
        }
    }
}

impl std::error::Error for TransformError {}

impl From<DocError> for TransformError {
    fn from(value: DocError) -> Self {
        Self::Doc(value)
    }
}

impl From<MathError> for TransformError {
    fn from(value: MathError) -> Self {
        Self::Math(value)
    }
}

impl From<SummaryPointError> for TransformError {
    fn from(value: SummaryPointError) -> Self {
        Self::SummaryPoint(value)
    }
}
