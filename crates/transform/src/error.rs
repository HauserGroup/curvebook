//! Structured error types for `lumencraft-transform`.

use std::fmt;

use lumencraft_doc::{DocError, SummaryPointError, TableKind};
use lumencraft_math::MathError;

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
    CircularDependency { cycle: Vec<String> },
    /// Syntax error in a formula or expression string.
    ParseError { position: usize, message: String },
    /// Runtime error during formula evaluation or mathematical transform.
    EvalError { message: String },
    /// Reference to an unknown column, variable, or constant.
    UnknownVariable { name: String },
    /// Reference to an unknown function name.
    UnknownFunction { name: String },
    /// Function called with an unsupported number of arguments.
    ArityMismatch { function: String, expected: String, actual: usize },
    /// Invalid transform configuration or parameter value.
    InvalidSpec { message: String },
    /// Table archetype cannot support the requested transformation.
    IncompatibleTable { operation: &'static str, kind: TableKind },
    /// Referenced `HookSource` could not be resolved to a finite numeric constant.
    MissingHook { description: String },
    /// Unrecognized equation identifier in `Simulate`.
    UnknownEquation { equation_id: String },
    /// Invalid or insufficient input data for curve calculus or import.
    InvalidInput { message: String },
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
