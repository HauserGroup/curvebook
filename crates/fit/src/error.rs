//! Structured error and diagnostic status types for `curvebook-fit`.

use std::fmt;

use curvebook_doc::{DocError, TableKind};
use curvebook_math::MathError;

/// Convergence or validity outcome of a linear or nonlinear regression fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum FitStatus {
    /// Solver satisfied both objective and step convergence tolerances.
    Converged,
    /// Solver reached the configured maximum iteration count before full convergence.
    HitMaxIters,
    /// Fit converged or stopped at a rank-deficient / collinear point (`Dependency > 0.9999`).
    Ambiguous,
    /// Starting parameter estimates produced non-finite values and simplex warm-start failed.
    BadInitialValues,
    /// Selected weighting scheme encountered invalid zero/negative domain values (e.g. `1/Y^2` with `Y = 0`).
    ImpossibleWeights,
    /// Objective diverged, stalled away from a stationary point, or could not evaluate finite residuals.
    Diverged,
}

/// Identifiability assessment for a nonlinear fit.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum FitIdentifiability {
    /// The assessment was disabled or was unavailable for this fit mode.
    #[default]
    NotAssessed,
    /// The fit passed the configured identifiability checks.
    Identifiable,
    /// At least one check found evidence of unstable parameter estimation.
    Ambiguous { reasons: Vec<String> },
}

impl FitIdentifiability {
    /// Whether the assessment detected instability.
    #[must_use]
    pub const fn is_ambiguous(&self) -> bool {
        matches!(self, Self::Ambiguous { .. })
    }
}

/// Diagnostic note attached to an individual fitted parameter estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ParameterNote {
    /// Parameter is fixed to a literal constant.
    Constant,
    /// Parameter is hooked to an upstream Info constant, Result scalar, or column title.
    Hooked,
    /// Parameter is shared globally across all datasets.
    SharedGlobal,
    /// Parameter converged onto its configured lower box bound.
    HitLowerBound,
    /// Parameter converged onto its configured upper box bound.
    HitUpperBound,
    /// Parameter is unidentifiable or severely collinear (`Dependency > 0.9999`).
    Ambiguous,
    /// Parameter flagged by the opt-in identifiability assessment.
    Unstable,
    /// Profile-likelihood confidence interval is unbounded on at least one side.
    UnboundedCi,
}

/// Structured error type returned by `curvebook-fit` routines.
#[derive(Debug, Clone, PartialEq)]
pub enum FitError {
    /// No active non-excluded observations were available.
    EmptyInput,
    /// Fewer active observations than required by model degrees of freedom.
    InsufficientData { required: usize, actual: usize },
    /// Equation identifier was not found in the built-in catalog.
    UnknownEquation { id: String },
    /// Invalid solver or analysis parameter configuration.
    InvalidParameter { name: &'static str, reason: String },
    /// Upstream `DataSheet` archetype is not compatible with regression.
    IncompatibleTable { kind: TableKind },
    /// Weighting or Poisson objective domain check failed (e.g. `1/Y^2` when `Y == 0` or `Y < 0` for Poisson).
    ImpossibleWeights { reason: String },
    /// Robust / ROUT / normality diagnostics require raw replicates rather than pre-summarized moments.
    RequiresRawReplicates,
    /// Starting parameter values produced non-finite predictions and could not be rescued.
    BadInitialValues { reason: String },
    /// Custom equation DSL syntax or compilation failure.
    DslCompileError { message: String, line: usize, column: usize },
    /// Custom equation bytecode VM runtime failure (e.g. step budget exceeded).
    DslEvalError { message: String },
    /// Adaptive RK45 ODE integration failed to reach observation times within tolerances/budget.
    OdeIntegrationFailed { reason: String },
    /// Implicit equation root-finder failed to bracket or converge for `y(x)`.
    ImplicitSolveFailed { reason: String },
    /// Referenced `HookSource` could not be resolved to a finite scalar.
    HookResolutionError { message: String },
    /// Propagated linear-algebra or root-finding error from `curvebook-math`.
    Math(MathError),
    /// Propagated document validation error from `curvebook-doc`.
    Doc(DocError),
}

impl From<MathError> for FitError {
    fn from(value: MathError) -> Self {
        Self::Math(value)
    }
}

impl From<DocError> for FitError {
    fn from(value: DocError) -> Self {
        Self::Doc(value)
    }
}

impl fmt::Display for FitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyInput => f.write_str("regression input contained zero active observations"),
            Self::InsufficientData { required, actual } => {
                write!(f, "insufficient observations: need at least {required}, got {actual}")
            }
            Self::UnknownEquation { id } => write!(f, "unknown built-in equation `{id}`"),
            Self::InvalidParameter { name, reason } => write!(f, "invalid parameter `{name}`: {reason}"),
            Self::IncompatibleTable { kind } => write!(f, "table kind {kind:?} is not compatible with regression"),
            Self::ImpossibleWeights { reason } => write!(f, "impossible weights: {reason}"),
            Self::RequiresRawReplicates => f.write_str("selected robust/outlier/normality option requires raw replicate observations"),
            Self::BadInitialValues { reason } => write!(f, "bad initial parameter values: {reason}"),
            Self::DslCompileError { message, line, column } => {
                write!(f, "equation compile error at line {line}, column {column}: {message}")
            }
            Self::DslEvalError { message } => write!(f, "equation evaluation error: {message}"),
            Self::OdeIntegrationFailed { reason } => write!(f, "ODE integration failed: {reason}"),
            Self::ImplicitSolveFailed { reason } => write!(f, "implicit equation solve failed: {reason}"),
            Self::HookResolutionError { message } => write!(f, "hook resolution failed: {message}"),
            Self::Math(err) => write!(f, "math error: {err}"),
            Self::Doc(err) => write!(f, "document error: {err}"),
        }
    }
}

impl std::error::Error for FitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Math(err) => Some(err),
            Self::Doc(err) => Some(err),
            _ => None,
        }
    }
}
