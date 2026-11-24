//! Core session, undo/redo history, reactive `Data -> Analysis -> Graph -> Layout` recalculation
//! pipeline, command registry and panic recovery boundary for CurveBook.
//!
//! Layer L5 crate: shared identically by the desktop app, browser WASM build, headless CLI and MCP
//! server. Nothing in or below this crate depends on a UI toolkit.
//! Governing spec: `docs/architecture.md`, `docs/implementation/data-tables-and-transforms.md` (§6), and `docs/development.md`.
#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

pub mod cmd;
pub use cmd::Caller;
pub mod dag;
pub mod guard;
pub mod methods_report;
pub mod path_policy;
pub use path_policy::ExternalPathPolicy;
pub mod services;
pub use services::{BatchCommandStep, BatchStepFailure};

use std::collections::VecDeque;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use curvebook_chart::{ChartError, ChartScene};
use curvebook_doc::{DocError, ProjectDocument, SheetId, SummaryPointError};
pub use curvebook_doc::{ErrorCategory, ErrorCode, RemediationKind};
use curvebook_fit::FitError;
#[cfg(not(target_arch = "wasm32"))]
use curvebook_format::DirectoryJournalBackend;
use curvebook_format::recovery::{CrashRecoveryJournal, JournalBackend, JournalRecoveryWarning, RecoveryReport};
use curvebook_format::{FormatError, normalize_command_params_in_place};
use curvebook_math::MathError;
use curvebook_pdf::PdfError;
use curvebook_platform::PlatformError;
use curvebook_render::RenderError;
use curvebook_stats::StatsError;
use curvebook_svg::SvgError;
use curvebook_transform::TransformError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Structured error returned by engine commands and file I/O operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineError {
    pub code: ErrorCode,
    pub category: ErrorCategory,
    message: String,
    pub retryable: bool,
    pub remediation: Option<String>,
    pub details: Option<Value>,
}

impl EngineError {
    /// Construct an engine error with [`ErrorCode::BadInput`] and a user-facing diagnostic message.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self::with_code(ErrorCode::BadInput, message)
    }

    /// Construct a structured engine error with an explicit [`ErrorCode`].
    #[must_use]
    pub fn with_code(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, category: code.category(), message: message.into(), retryable: false, remediation: None, details: None }
    }

    /// Attach an actionable remediation suggestion for callers and users.
    #[must_use]
    pub fn with_remediation(mut self, remediation: impl Into<String>) -> Self {
        self.remediation = Some(remediation.into());
        self
    }

    /// Attach structured diagnostic details metadata.
    #[must_use]
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    /// Mark whether retrying the operation might succeed without input changes.
    #[must_use]
    pub const fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    /// Diagnostic message text.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for EngineError {}

fn build_mapped_error(code: ErrorCode, message: String, details: Option<Value>, remediation: Option<&'static str>, retryable: bool) -> EngineError {
    let mut err = EngineError::with_code(code, message).with_retryable(retryable);
    if let Some(d) = details {
        err = err.with_details(d);
    }
    if let Some(r) = remediation {
        err = err.with_remediation(r);
    }
    err
}

impl From<DocError> for EngineError {
    fn from(err: DocError) -> Self {
        let msg = err.to_string();
        let (code, details, remediation) = match err {
            DocError::EmptyIdentifier => (ErrorCode::BadInput, None, Some("Provide a non-empty identifier.")),
            DocError::InvalidIdentifier { reason } => (
                ErrorCode::BadInput,
                Some(json!({ "reason": reason })),
                Some("Provide a valid sheet identifier up to 256 UTF-8 bytes without whitespace, control characters, or hook delimiters."),
            ),
            DocError::TooManyRows { count, max } => {
                (ErrorCode::ResourceLimit, Some(json!({ "count": count, "max": max })), Some("Reduce row count below the project maximum."))
            }
            DocError::TooManyDataSets { count, max } => (
                ErrorCode::ResourceLimit,
                Some(json!({ "count": count, "max": max })),
                Some("Reduce dataset column count below the project maximum."),
            ),
            DocError::TooManySubcolumns { count, max } => (
                ErrorCode::ResourceLimit,
                Some(json!({ "count": count, "max": max })),
                Some("Reduce replicate subcolumn count below the project maximum."),
            ),
            DocError::TooManyCells { count, max } => (
                ErrorCode::ResourceLimit,
                Some(json!({ "count": count, "max": max })),
                Some("Reduce rows, data sets, or replicate subcolumns below the cell limit."),
            ),
            DocError::TooManyLayoutItems { count, max } => {
                (ErrorCode::ResourceLimit, Some(json!({ "count": count, "max": max })), Some("Reduce layout item count below the project maximum."))
            }
            DocError::TooManySheets { count, max } => {
                (ErrorCode::ResourceLimit, Some(json!({ "count": count, "max": max })), Some("Delete unused sheets before adding more."))
            }
            DocError::TooManyImageAssets { count, max } => (
                ErrorCode::ResourceLimit,
                Some(json!({ "count": count, "max": max })),
                Some("Remove unused embedded images before adding more assets."),
            ),
            DocError::TitleTooLong { byte_len, max } => {
                (ErrorCode::ResourceLimit, Some(json!({ "byte_len": byte_len, "max": max })), Some("Shorten the title to fit within the byte limit."))
            }
            DocError::InvalidSubcolumnCount { count } => (ErrorCode::BadInput, Some(json!({ "count": count })), None),
            DocError::IncompatibleSubcolumnMode { archetype, mode } => (
                ErrorCode::BadInput,
                Some(json!({ "archetype": format!("{archetype:?}"), "mode": format!("{mode:?}") })),
                Some("Choose a subcolumn mode supported by this table archetype."),
            ),
            DocError::SheetFrozen { sheet_id } => (
                ErrorCode::FrozenSheet,
                Some(json!({ "sheet_id": sheet_id.as_str() })),
                Some("Unfreeze the sheet before editing its cells or links."),
            ),
            DocError::CellOutOfBounds { row, col, subcol } => (ErrorCode::BadInput, Some(json!({ "row": row, "col": col, "subcol": subcol })), None),
            DocError::InvalidCellValue { archetype, row, col, reason } => {
                (ErrorCode::BadInput, Some(json!({ "archetype": format!("{archetype:?}"), "row": row, "col": col, "reason": reason })), None)
            }
            DocError::SheetNotFound { sheet_id } => {
                (ErrorCode::NotFound, Some(json!({ "sheet_id": sheet_id.as_str() })), Some("Inspect project sheets to locate a valid sheet ID."))
            }
            DocError::SheetIndexOutOfBounds { folder, index, len } => {
                (ErrorCode::BadInput, Some(json!({ "folder": format!("{folder:?}"), "index": index, "len": len })), None)
            }
            DocError::DuplicateSheetId { sheet_id } => {
                (ErrorCode::AlreadyExists, Some(json!({ "sheet_id": sheet_id.as_str() })), Some("Choose a unique sheet identifier."))
            }
            DocError::InvalidConfidenceLevel { percent } => {
                (ErrorCode::BadInput, Some(json!({ "percent": percent })), Some("Specify a confidence level strictly between 0 and 100."))
            }
            DocError::InvalidAxisSegments { count } => (ErrorCode::BadInput, Some(json!({ "count": count })), Some("Provide 1 to 3 axis segments.")),
            DocError::IncompatibleRetargetTable { analysis_title, target_kind } => (
                ErrorCode::BadInput,
                Some(json!({ "analysis_title": analysis_title, "target_kind": format!("{target_kind:?}") })),
                Some("Select a target table whose archetype matches this analysis."),
            ),
        };
        build_mapped_error(code, msg, details, remediation, false)
    }
}

impl From<SummaryPointError> for EngineError {
    fn from(err: SummaryPointError) -> Self {
        let msg = err.to_string();
        let (code, details, remediation) = match err {
            SummaryPointError::NotParametricSummaryMode { mode } => (ErrorCode::BadInput, Some(json!({ "mode": format!("{mode:?}") })), None),
            SummaryPointError::MissingSpread => (ErrorCode::InsufficientData, None, Some("Provide SD or SEM spread values for every summary point.")),
            SummaryPointError::MissingSampleSize => (ErrorCode::InsufficientData, None, Some("Provide sample size N >= 1 for every summary point.")),
            SummaryPointError::SampleSizeBelowOne { n } => (ErrorCode::InsufficientData, Some(json!({ "n": n })), Some("Use sample size N >= 1.")),
            SummaryPointError::NegativeSpread { spread } => {
                (ErrorCode::BadInput, Some(json!({ "spread": spread })), Some("Use non-negative spread values."))
            }
        };
        build_mapped_error(code, msg, details, remediation, false)
    }
}

impl From<MathError> for EngineError {
    fn from(err: MathError) -> Self {
        let msg = err.to_string();
        let (code, details, remediation, retryable) = match err {
            MathError::EmptyInput => (ErrorCode::InsufficientData, None, Some("Provide at least one finite value."), false),
            MathError::DimensionMismatch { expected, actual } => {
                (ErrorCode::BadInput, Some(json!({ "expected": expected, "actual": actual })), None, false)
            }
            MathError::InvalidParameter { name, value } => (ErrorCode::BadInput, Some(json!({ "name": name, "value": value })), None, false),
            MathError::DomainError { name, value } => (ErrorCode::BadInput, Some(json!({ "name": name, "value": value })), None, false),
            MathError::SingularMatrix { rank, expected_rank } => (
                ErrorCode::SolverFailed,
                Some(json!({ "rank": rank, "expected_rank": expected_rank })),
                Some("Check for collinear predictors, duplicate rows, or constant columns."),
                false,
            ),
            MathError::NotPositiveDefinite { index, pivot } => {
                (ErrorCode::SolverFailed, Some(json!({ "index": index, "pivot": pivot })), Some("Inspect the matrix for linear dependence."), false)
            }
            MathError::NotConverged { algorithm, iterations } => (
                ErrorCode::SolverFailed,
                Some(json!({ "algorithm": algorithm, "iterations": iterations })),
                Some("Adjust initial guesses, scaling, or maximum iteration count."),
                true,
            ),
        };
        build_mapped_error(code, msg, details, remediation, retryable)
    }
}

impl From<FormatError> for EngineError {
    fn from(err: FormatError) -> Self {
        let msg = err.to_string();
        let (code, details, remediation, retryable) = match err {
            FormatError::Doc(doc_err) => return Self::from(doc_err),
            FormatError::Io(detail) => {
                (ErrorCode::IoError, Some(json!({ "io_error": detail })), Some("Verify file path permissions and disk availability."), true)
            }
            FormatError::Json(detail) => (ErrorCode::CorruptData, Some(json!({ "json_error": detail })), None, false),
            FormatError::InvalidZip { reason } => (ErrorCode::CorruptData, Some(json!({ "reason": reason })), None, false),
            FormatError::CorruptCrc { entry, expected, actual } => (
                ErrorCode::CorruptData,
                Some(json!({ "entry": entry, "expected": expected, "actual": actual })),
                Some("Restore from backup or checkpoint; archive CRC verification failed."),
                false,
            ),
            FormatError::UnsupportedCompression { entry, method } => {
                (ErrorCode::UnsupportedFormat, Some(json!({ "entry": entry, "method": method })), None, false)
            }
            FormatError::ZipBomb { reason } => {
                (ErrorCode::ResourceLimit, Some(json!({ "reason": reason })), Some("Archive exceeds decompression safety limits."), false)
            }
            FormatError::FileTooLarge { path, actual_bytes, limit_bytes } => (
                ErrorCode::ResourceLimit,
                Some(json!({ "path": path, "actual_bytes": actual_bytes, "limit_bytes": limit_bytes })),
                Some("Reduce the input file size below the format limit."),
                false,
            ),
            FormatError::UnsafePath { path } => {
                (ErrorCode::PathEscape, Some(json!({ "path": path })), Some("Ensure all archive or file paths stay inside the allowed root."), false)
            }
            FormatError::DuplicateEntry { path } => (ErrorCode::AlreadyExists, Some(json!({ "path": path })), None, false),
            FormatError::MissingArchiveEntry { path } => (ErrorCode::CorruptData, Some(json!({ "path": path })), None, false),
            FormatError::UnsupportedSchemaVersion { version, max_supported } => (
                ErrorCode::UnsupportedVersion,
                Some(json!({ "version": version, "max_supported": max_supported })),
                Some("Upgrade CurveBook to open files created with a newer schema version."),
                false,
            ),
            FormatError::InvalidArchiveKind { expected, found } => {
                (ErrorCode::UnsupportedFormat, Some(json!({ "expected": expected, "found": found })), None, false)
            }
            FormatError::DependencyCycle { cycle } => {
                (ErrorCode::CycleDetected, Some(json!({ "cycle": cycle })), Some("Break circular table link or formula references."), false)
            }
            FormatError::NonFiniteFloat { location } => (ErrorCode::CorruptData, Some(json!({ "location": location })), None, false),
            FormatError::CsvParse { line, reason } => (
                ErrorCode::CorruptData,
                Some(json!({ "line": line, "reason": reason })),
                Some("Verify delimiter, quoting, and decimal separator settings."),
                false,
            ),
            FormatError::XmlParse { offset, reason } => (ErrorCode::CorruptData, Some(json!({ "offset": offset, "reason": reason })), None, false),
            FormatError::XmlDepthExceeded { depth, max } => (ErrorCode::ResourceLimit, Some(json!({ "depth": depth, "max": max })), None, false),
            FormatError::UnsupportedFormat { reason } => (ErrorCode::UnsupportedFormat, Some(json!({ "reason": reason })), None, false),
            FormatError::InvalidPipeline { reason } => (ErrorCode::BadInput, Some(json!({ "reason": reason })), None, false),
        };
        build_mapped_error(code, msg, details, remediation, retryable)
    }
}

impl From<TransformError> for EngineError {
    fn from(err: TransformError) -> Self {
        let msg = err.to_string();
        let (code, details, remediation) = match err {
            TransformError::Doc(e) => return Self::from(e),
            TransformError::Math(e) => return Self::from(e),
            TransformError::SummaryPoint(e) => return Self::from(e),
            TransformError::CircularDependency { cycle } => {
                (ErrorCode::CycleDetected, Some(json!({ "cycle": cycle })), Some("Remove circular formula dependencies."))
            }
            TransformError::ParseError { position, message } => (
                ErrorCode::BadInput,
                Some(json!({ "position": position, "reason": message })),
                Some("Check formula syntax near the reported character offset."),
            ),
            TransformError::EvalError { message } => (ErrorCode::SolverFailed, Some(json!({ "reason": message })), None),
            TransformError::UnknownVariable { name } => {
                (ErrorCode::BadInput, Some(json!({ "variable": name })), Some("Verify column or calculated-variable spelling."))
            }
            TransformError::UnknownFunction { name } => (ErrorCode::BadInput, Some(json!({ "function": name })), None),
            TransformError::ArityMismatch { function, expected, actual } => {
                (ErrorCode::BadInput, Some(json!({ "function": function, "expected": expected, "actual": actual })), None)
            }
            TransformError::InvalidSpec { message } => (ErrorCode::BadInput, Some(json!({ "reason": message })), None),
            TransformError::IncompatibleTable { operation, kind } => {
                (ErrorCode::BadInput, Some(json!({ "operation": operation, "kind": format!("{kind:?}") })), None)
            }
            TransformError::MissingHook { description } => {
                (ErrorCode::NotFound, Some(json!({ "hook": description })), Some("Create or bind the referenced sheet, column, or constant hook."))
            }
            TransformError::UnknownEquation { equation_id } => (ErrorCode::NotFound, Some(json!({ "equation_id": equation_id })), None),
            TransformError::InvalidInput { message } => (ErrorCode::BadInput, Some(json!({ "reason": message })), None),
        };
        build_mapped_error(code, msg, details, remediation, false)
    }
}

impl From<StatsError> for EngineError {
    fn from(err: StatsError) -> Self {
        let msg = err.to_string();
        let (code, details, remediation, retryable) = match err {
            StatsError::Doc(e) => return Self::from(e),
            StatsError::Math(e) => return Self::from(e),
            StatsError::SummaryPoint(e) => return Self::from(e),
            StatsError::EmptyInput => (ErrorCode::InsufficientData, None, Some("Provide finite observations in the selected columns."), false),
            StatsError::InsufficientSampleSize { context, required, actual } => (
                ErrorCode::InsufficientData,
                Some(json!({ "context": context, "required": required, "actual": actual })),
                Some("Add more observations to meet the test sample-size requirement."),
                false,
            ),
            StatsError::InvalidParameter { name, value } => (ErrorCode::BadInput, Some(json!({ "name": name, "value": value })), None, false),
            StatsError::DimensionMismatch { expected, actual } => {
                (ErrorCode::BadInput, Some(json!({ "expected": expected, "actual": actual })), None, false)
            }
            StatsError::RequiresRawReplicates => {
                (ErrorCode::BadInput, None, Some("Provide raw replicate values rather than pre-averaged summary statistics."), false)
            }
            StatsError::IncompatibleTable { analysis, actual } => (
                ErrorCode::BadInput,
                Some(json!({ "analysis": analysis, "actual": format!("{actual:?}") })),
                Some("Select a table archetype compatible with this statistical analysis."),
                false,
            ),
            StatsError::IncompatibleAnalysisSpec { expected } => (ErrorCode::BadInput, Some(json!({ "expected": expected })), None, false),
            StatsError::ZeroVariance { context } => {
                (ErrorCode::InsufficientData, Some(json!({ "context": context })), Some("Provide data with non-zero variance."), false)
            }
            StatsError::MissingColumn { index } => (ErrorCode::BadInput, Some(json!({ "column": index })), None, false),
            StatsError::NotConverged { algorithm } => {
                (ErrorCode::SolverFailed, Some(json!({ "algorithm": algorithm })), Some("Adjust model complexity or convergence settings."), true)
            }
        };
        build_mapped_error(code, msg, details, remediation, retryable)
    }
}

impl From<FitError> for EngineError {
    fn from(err: FitError) -> Self {
        let msg = err.to_string();
        let (code, details, remediation, retryable) = match err {
            FitError::Doc(e) => return Self::from(e),
            FitError::Math(e) => return Self::from(e),
            FitError::EmptyInput => (ErrorCode::InsufficientData, None, Some("Provide finite (X, Y) points to fit."), false),
            FitError::InsufficientData { required, actual } => (
                ErrorCode::InsufficientData,
                Some(json!({ "required": required, "actual": actual })),
                Some("Provide at least as many distinct points as fitted parameters."),
                false,
            ),
            FitError::UnknownEquation { id } => (
                ErrorCode::NotFound,
                Some(json!({ "equation_id": id })),
                Some("Use a built-in equation ID or specify a custom model DSL formula."),
                false,
            ),
            FitError::InvalidParameter { name, reason } => (ErrorCode::BadInput, Some(json!({ "name": name, "reason": reason })), None, false),
            FitError::IncompatibleTable { kind } => (
                ErrorCode::BadInput,
                Some(json!({ "kind": format!("{kind:?}") })),
                Some("Linear and nonlinear regression require an XY data table."),
                false,
            ),
            FitError::ImpossibleWeights { reason } => {
                (ErrorCode::BadInput, Some(json!({ "reason": reason })), Some("Switch weighting mode or remove non-positive Y values."), false)
            }
            FitError::RequiresRawReplicates => (ErrorCode::BadInput, None, Some("Provide raw Y replicates instead of summary-only entries."), false),
            FitError::BadInitialValues { reason } => (
                ErrorCode::BadInput,
                Some(json!({ "reason": reason })),
                Some("Provide finite starting parameter guesses within the model domain."),
                false,
            ),
            FitError::DslCompileError { message, line, column } => (
                ErrorCode::BadInput,
                Some(json!({ "reason": message, "line": line, "column": column })),
                Some("Fix DSL syntax or undefined identifiers at the reported line and column."),
                false,
            ),
            FitError::DslEvalError { message } => (
                ErrorCode::SolverFailed,
                Some(json!({ "reason": message })),
                Some("Check parameter bounds to avoid domain errors during evaluation."),
                false,
            ),
            FitError::OdeIntegrationFailed { reason } => {
                (ErrorCode::SolverFailed, Some(json!({ "reason": reason })), Some("Inspect ODE initial values, stiffness, or X domain bounds."), true)
            }
            FitError::ImplicitSolveFailed { reason } => {
                (ErrorCode::SolverFailed, Some(json!({ "reason": reason })), Some("Inspect implicit root bracketing or parameter guesses."), true)
            }
            FitError::HookResolutionError { message } => (
                ErrorCode::NotFound,
                Some(json!({ "reason": message })),
                Some("Verify referenced info constants or upstream hook sources exist."),
                false,
            ),
        };
        build_mapped_error(code, msg, details, remediation, retryable)
    }
}

impl From<ChartError> for EngineError {
    fn from(err: ChartError) -> Self {
        let msg = err.to_string();
        let (code, details) = match err {
            ChartError::SheetNotFound { sheet_id } => (ErrorCode::NotFound, Some(json!({ "sheet_id": sheet_id }))),
            ChartError::InvalidSpec { message } => (ErrorCode::BadInput, Some(json!({ "reason": message }))),
            ChartError::ResourceLimit { message } => (ErrorCode::ResourceLimit, Some(json!({ "reason": message }))),
            ChartError::CompileFailed { message } => (ErrorCode::InternalError, Some(json!({ "reason": message }))),
        };
        build_mapped_error(code, msg, details, None, false)
    }
}

impl From<SvgError> for EngineError {
    fn from(err: SvgError) -> Self {
        let msg = err.to_string();
        let (code, details) = match err {
            SvgError::InvalidScene { message } => (ErrorCode::BadInput, Some(json!({ "reason": message }))),
            SvgError::ValidationFailed { message } => (ErrorCode::InternalError, Some(json!({ "reason": message }))),
        };
        build_mapped_error(code, msg, details, None, false)
    }
}

impl From<PdfError> for EngineError {
    fn from(err: PdfError) -> Self {
        let msg = err.to_string();
        let (code, details, remediation) = match err {
            PdfError::EmptyDocument => (ErrorCode::BadInput, None, Some("Add at least one page before exporting PDF.")),
            PdfError::InvalidScene { message } => (ErrorCode::BadInput, Some(json!({ "reason": message })), None),
            PdfError::ResourceLimit { message } => {
                (ErrorCode::ResourceLimit, Some(json!({ "reason": message })), Some("Reduce the number of PDF pages or chart complexity."))
            }
            PdfError::CompressionFailed { message } => (ErrorCode::InternalError, Some(json!({ "reason": message })), None),
        };
        build_mapped_error(code, msg, details, remediation, false)
    }
}

impl From<RenderError> for EngineError {
    fn from(err: RenderError) -> Self {
        let msg = err.to_string();
        let (code, details) = match err {
            RenderError::InvalidTarget { message } => (ErrorCode::BadInput, Some(json!({ "reason": message }))),
            RenderError::BufferSizeMismatch { expected, actual } => (ErrorCode::BadInput, Some(json!({ "expected": expected, "actual": actual }))),
            RenderError::EncodingFailed { message } => (ErrorCode::InternalError, Some(json!({ "reason": message }))),
        };
        build_mapped_error(code, msg, details, None, false)
    }
}

impl From<PlatformError> for EngineError {
    fn from(err: PlatformError) -> Self {
        let msg = err.to_string();
        let (code, details, remediation, retryable) = match err {
            PlatformError::ProgramNotFound { program } => (
                ErrorCode::NotFound,
                Some(json!({ "program": program })),
                Some("Install the required external tool or ensure it is available on PATH."),
                false,
            ),
            PlatformError::TimedOut { program, kind, limit_ms } => (
                ErrorCode::IoError,
                Some(json!({ "program": program, "timeout_kind": kind.to_string(), "limit_ms": limit_ms })),
                Some("Retry with a smaller input or a longer timeout."),
                true,
            ),
            PlatformError::OutputLimitExceeded { program, stream, limit_bytes } => {
                (ErrorCode::ResourceLimit, Some(json!({ "program": program, "stream": stream, "limit_bytes": limit_bytes })), None, false)
            }
            PlatformError::FileTooLarge { path, actual_bytes, limit_bytes } => (
                ErrorCode::ResourceLimit,
                Some(json!({ "path": path, "actual_bytes": actual_bytes, "limit_bytes": limit_bytes })),
                Some("Reduce the input file size below the format limit."),
                false,
            ),
            PlatformError::InvalidPath { reason } => (ErrorCode::BadInput, Some(json!({ "reason": reason })), None, false),
            PlatformError::PathEscape { path, workdir } => (
                ErrorCode::PathEscape,
                Some(json!({ "path": path, "workdir": workdir })),
                Some("Keep file paths confined inside the allowed working directory."),
                false,
            ),
            PlatformError::Io { context, message } => (ErrorCode::IoError, Some(json!({ "context": context, "reason": message })), None, true),
        };
        build_mapped_error(code, msg, details, remediation, retryable)
    }
}

#[cfg(test)]
mod structured_error_and_dispatch_tests {
    use curvebook_platform::TimeoutKind;

    use super::*;
    use crate::cmd::params::empty_object_schema;
    use crate::cmd::{CommandGroup, CommandSpec, Ctx, Effects};

    #[test]
    fn engine_error_builders_display_and_serde_roundtrip() {
        let basic = EngineError::new("bad input text");
        assert_eq!(basic.code, ErrorCode::BadInput);
        assert_eq!(basic.category, ErrorCategory::Input);
        assert_eq!(basic.message(), "bad input text");
        assert_eq!(basic.to_string(), "bad input text");
        assert!(!basic.retryable);
        assert!(basic.remediation.is_none());
        assert!(basic.details.is_none());

        let rich = EngineError::with_code(ErrorCode::SolverFailed, "did not converge")
            .with_remediation("Adjust initial guesses.")
            .with_details(json!({ "iterations": 500 }))
            .with_retryable(true);
        assert_eq!(rich.category, ErrorCategory::Compute);
        assert!(rich.retryable);
        assert_eq!(rich.remediation.as_deref(), Some("Adjust initial guesses."));

        let encoded = serde_json::to_string(&rich).expect("serialize EngineError");
        let decoded: EngineError = serde_json::from_str(&encoded).expect("deserialize EngineError");
        assert_eq!(decoded, rich);
    }

    #[test]
    fn from_doc_math_format_transform_errors_map_structured_codes() {
        let sheet = SheetId::new("data_1");

        let e_frozen = EngineError::from(DocError::SheetFrozen { sheet_id: sheet.clone() });
        assert_eq!(e_frozen.code, ErrorCode::FrozenSheet);
        assert_eq!(e_frozen.category, ErrorCategory::State);
        assert_eq!(e_frozen.details.as_ref().and_then(|d| d.get("sheet_id")), Some(&json!("data_1")));
        assert!(e_frozen.remediation.is_some());

        assert_eq!(EngineError::from(DocError::SheetNotFound { sheet_id: sheet.clone() }).code, ErrorCode::NotFound);
        assert_eq!(EngineError::from(DocError::DuplicateSheetId { sheet_id: sheet.clone() }).code, ErrorCode::AlreadyExists);
        assert_eq!(EngineError::from(DocError::TooManyRows { count: 999, max: 100 }).code, ErrorCode::ResourceLimit);
        assert_eq!(EngineError::from(SummaryPointError::MissingSpread).code, ErrorCode::InsufficientData);

        let e_math = EngineError::from(MathError::NotConverged { algorithm: "lm", iterations: 200 });
        assert_eq!(e_math.code, ErrorCode::SolverFailed);
        assert!(e_math.retryable);

        assert_eq!(EngineError::from(FormatError::UnsupportedSchemaVersion { version: 9, max_supported: 3 }).code, ErrorCode::UnsupportedVersion);
        assert_eq!(
            EngineError::from(FormatError::DependencyCycle { cycle: vec![SheetId::new("a"), SheetId::new("b")] }).code,
            ErrorCode::CycleDetected
        );
        assert_eq!(EngineError::from(FormatError::UnsafePath { path: String::from("../secret") }).code, ErrorCode::PathEscape);
        assert_eq!(EngineError::from(FormatError::Doc(DocError::SheetFrozen { sheet_id: sheet })).code, ErrorCode::FrozenSheet);

        let e_tx_parse = EngineError::from(TransformError::ParseError { position: 4, message: String::from("unexpected token") });
        assert_eq!(e_tx_parse.code, ErrorCode::BadInput);
        assert_eq!(e_tx_parse.details.as_ref().and_then(|d| d.get("position")), Some(&json!(4)));
    }

    #[test]
    fn from_stats_fit_render_platform_errors_map_structured_codes() {
        let e_stats = EngineError::from(StatsError::InsufficientSampleSize { context: "t_test", required: 3, actual: 1 });
        assert_eq!(e_stats.code, ErrorCode::InsufficientData);
        assert_eq!(e_stats.details.as_ref().and_then(|d| d.get("required")), Some(&json!(3)));

        let e_fit_dsl = EngineError::from(FitError::DslCompileError { message: String::from("unknown symbol"), line: 2, column: 7 });
        assert_eq!(e_fit_dsl.code, ErrorCode::BadInput);
        assert_eq!(e_fit_dsl.details.as_ref().and_then(|d| d.get("line")), Some(&json!(2)));
        assert_eq!(e_fit_dsl.details.as_ref().and_then(|d| d.get("column")), Some(&json!(7)));

        let e_fit_ode = EngineError::from(FitError::OdeIntegrationFailed { reason: String::from("step underflow") });
        assert_eq!(e_fit_ode.code, ErrorCode::SolverFailed);
        assert!(e_fit_ode.retryable);

        assert_eq!(EngineError::from(ChartError::SheetNotFound { sheet_id: String::from("g1") }).code, ErrorCode::NotFound);
        assert_eq!(EngineError::from(SvgError::ValidationFailed { message: String::from("bad") }).code, ErrorCode::InternalError);
        assert_eq!(EngineError::from(PdfError::EmptyDocument).code, ErrorCode::BadInput);
        assert_eq!(EngineError::from(RenderError::BufferSizeMismatch { expected: 16, actual: 8 }).code, ErrorCode::BadInput);
        assert_eq!(
            EngineError::from(PlatformError::PathEscape { path: String::from("../out"), workdir: String::from("/safe") }).code,
            ErrorCode::PathEscape
        );
        let e_timeout = EngineError::from(PlatformError::TimedOut { program: String::from("helper"), kind: TimeoutKind::Wall, limit_ms: 500 });
        assert_eq!(e_timeout.code, ErrorCode::IoError);
        assert!(e_timeout.retryable);
    }

    #[test]
    fn execute_with_caller_handles_cancellation_conflict_readonly_and_rollback() {
        let mut session = Session::new();
        let cancelled = AtomicBool::new(true);
        let err =
            session.execute_with_caller("project.inspect", json!({}), Caller::Mcp, &cancelled).expect_err("pre-cancelled command must fail loudly");
        assert_eq!(err.code, ErrorCode::Cancelled);
        assert!(err.retryable);

        fn disabled(_session: &Session) -> bool {
            false
        }
        fn always_ok(_session: &Session) -> bool {
            true
        }
        fn noop_run(_ctx: &mut Ctx<'_>, _params: Value) -> Result<CommandOutcome, EngineError> {
            Ok(CommandOutcome {
                command_id: String::from("test.read"),
                mutated: false,
                message: String::from("ok"),
                payload: json!({}),
                warnings: Vec::new(),
            })
        }
        fn mutating_fail(ctx: &mut Ctx<'_>, _params: Value) -> Result<CommandOutcome, EngineError> {
            ctx.session.doc.title = String::from("Corrupted Title");
            ctx.session.dirty = true;
            Err(EngineError::with_code(ErrorCode::BadInput, "boom"))
        }

        let disabled_spec = CommandSpec {
            id: "test.disabled",
            label: "Disabled",
            menu_path: "",
            shortcut: "",
            summary: "Disabled command",
            beginner: "",
            group: CommandGroup::Project,
            effects: Effects::NONE,
            input_schema: empty_object_schema,
            enabled: disabled,
            run: noop_run,
        };
        let active = AtomicBool::new(false);
        let conflict =
            session.execute_registered_spec(&disabled_spec, json!({}), Caller::Cli, &active).expect_err("disabled spec should emit Conflict");
        assert_eq!(conflict.code, ErrorCode::Conflict);

        let ro_spec = CommandSpec { id: "test.read", enabled: always_ok, ..disabled_spec };
        let res = session.execute_registered_spec(&ro_spec, json!({}), Caller::Control, &active).expect("read-only spec executes cleanly");
        assert!(!res.mutated);
        assert_eq!(session.undo_depth(), 0);
        assert!(!session.is_dirty());

        let original_title = session.doc.title.clone();
        let fail_spec = CommandSpec { id: "test.fail", effects: Effects::MUTATES_DOCUMENT, run: mutating_fail, ..ro_spec };
        let rollback_err =
            session.execute_registered_spec(&fail_spec, json!({}), Caller::Replay, &active).expect_err("failing mutating command must fail");
        assert_eq!(rollback_err.code, ErrorCode::BadInput);
        assert_eq!(session.doc.title, original_title);
        assert!(!session.is_dirty());
        assert_eq!(session.undo_depth(), 0);
    }
}

/// Structured result emitted after executing a command on [`Session`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandOutcome {
    pub command_id: String,
    pub mutated: bool,
    pub message: String,
    pub payload: serde_json::Value,
    /// Non-fatal warnings emitted during execution (for example retired parameter aliases).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl CommandOutcome {
    /// Construct a mutating command outcome.
    #[must_use]
    pub fn mutated(command_id: impl Into<String>, message: impl Into<String>, payload: serde_json::Value) -> Self {
        Self { command_id: command_id.into(), mutated: true, message: message.into(), payload, warnings: Vec::new() }
    }

    /// Construct a read-only command outcome (leaves undo stack and dirty state untouched).
    #[must_use]
    pub fn read_only(command_id: impl Into<String>, message: impl Into<String>, payload: serde_json::Value) -> Self {
        Self { command_id: command_id.into(), mutated: false, message: message.into(), payload, warnings: Vec::new() }
    }
}

/// Maximum number of document snapshots retained on a session's undo stack.
pub const MAX_UNDO_ENTRIES: usize = 64;

/// Maximum cumulative allocated data-table cell slots retained across undo snapshots.
///
/// The newest snapshot is always retained even if it alone exceeds this budget.
pub const MAX_UNDO_CELL_BUDGET: usize = 25_000_000;

/// Maximum cumulative checkpoint snapshot JSON bytes retained across undo snapshots.
pub const MAX_UNDO_CHECKPOINT_BYTES_BUDGET: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
struct UndoEntry {
    doc: ProjectDocument,
    file_path: Option<PathBuf>,
    cell_slots: usize,
    checkpoint_bytes: usize,
}

impl UndoEntry {
    fn from_snapshot(doc: ProjectDocument, file_path: Option<PathBuf>) -> Self {
        let cell_slots = document_allocated_cell_slots(&doc);
        let checkpoint_bytes = document_checkpoint_bytes(&doc);
        Self { doc, file_path, cell_slots, checkpoint_bytes }
    }
}

fn document_allocated_cell_slots(doc: &ProjectDocument) -> usize {
    doc.data_sheets.iter().fold(0usize, |acc, sheet| acc.saturating_add(sheet.archetype.allocated_cell_slots()))
}

fn document_checkpoint_bytes(doc: &ProjectDocument) -> usize {
    let cp_bytes = doc.checkpoints.iter().fold(0usize, |acc, cp| acc.saturating_add(cp.snapshot_json.len()));
    let asset_bytes = doc.image_assets.iter().fold(0usize, |acc, asset| acc.saturating_add(asset.bytes.len()));
    cp_bytes.saturating_add(asset_bytes)
}

fn is_preview_only_convert(params: &serde_json::Value) -> bool {
    params.get("preview_only").and_then(serde_json::Value::as_bool) == Some(true)
}

/// Execution channel restriction for interactive vs. direct automation callers (`docs/gui-workflows.md` §1).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuiOnlyPolicy {
    /// Default mode: all callers (`Ui`, `Mcp`, `Control`, `Cli`, `Replay`) may invoke commands directly.
    #[default]
    Disabled,
    /// Strict GUI-only mode: non-UI automation callers (`Mcp`, `Control`, `Cli`) may invoke only
    /// strictly read-only inspection commands; direct domain mutations, computes, and file writes
    /// return [`ErrorCode::GuiOnlyViolation`].
    Strict,
}

fn is_strict_gui_only_allowed(spec: &cmd::CommandSpec, caller: Caller, params: &serde_json::Value) -> bool {
    if !matches!(caller, Caller::Mcp | Caller::Control | Caller::Cli) {
        return true;
    }
    if spec.id == "table.convertArchetype" && is_preview_only_convert(params) {
        return true;
    }
    spec.effects.strict_gui_only_allowed()
}

const UNCONSTRAINED_GUI_ONLY_PATH_KEYS: [&str; 33] = [
    "path",
    "file",
    "file_path",
    "filePath",
    "out",
    "output",
    "output_path",
    "outputPath",
    "in",
    "input",
    "input_path",
    "inputPath",
    "source_path",
    "sourcePath",
    "target_path",
    "targetPath",
    "destination_path",
    "destinationPath",
    "template_path",
    "templatePath",
    "project",
    "project_path",
    "projectPath",
    "csv_path",
    "csvPath",
    "ttf_font",
    "ttfFont",
    "font_path",
    "fontPath",
    "unpacked_dir",
    "unpackedDir",
    "directory",
    "dir",
];

fn document_has_external_links(doc: &ProjectDocument) -> bool {
    doc.data_sheets
        .iter()
        .any(|ds| ds.linked_regions.iter().any(|region| matches!(region.spec.source, curvebook_doc::LinkSource::ExternalDelimitedFile { .. })))
}

/// Returns `true` when `val` recursively contains an `ExternalDelimitedFile` link object key.
#[must_use]
pub fn contains_external_delimited_link(val: &Value) -> bool {
    match val {
        Value::Object(map) => {
            if map.keys().any(|k| k.eq_ignore_ascii_case("externaldelimitedfile") || k == "external_delimited_file") {
                return true;
            }
            map.values().any(contains_external_delimited_link)
        }
        Value::Array(items) => items.iter().any(contains_external_delimited_link),
        _ => false,
    }
}

/// Return the first non-empty host filesystem path parameter key recursively present in `val`.
#[must_use]
pub fn find_host_path_param_key(val: &Value) -> Option<&'static str> {
    match val {
        Value::Object(map) => {
            for &key in &UNCONSTRAINED_GUI_ONLY_PATH_KEYS {
                if let Some(v) = map.get(key)
                    && !v.is_null()
                    && v.as_str().is_none_or(|s| !s.trim().is_empty())
                {
                    return Some(key);
                }
            }
            map.values().find_map(find_host_path_param_key)
        }
        Value::Array(items) => items.iter().find_map(find_host_path_param_key),
        _ => None,
    }
}

fn has_non_empty_json_object(val: &Value) -> bool {
    !val.is_null() && !val.as_object().is_some_and(serde_json::Map::is_empty)
}

fn has_non_false_flag(val: &Value, keys: &[&str]) -> bool {
    val.as_object().is_some_and(|map| keys.iter().any(|&key| map.get(key).is_some_and(|v| !v.is_null() && v.as_bool() != Some(false))))
}

fn is_unconstrained_archive_or_file_cmd(cmd_id: &str) -> bool {
    matches!(
        cmd_id,
        "project.open"
            | "file.open"
            | "file.open_unpacked_dir"
            | "project.instantiateTemplate"
            | "table.importPrism"
            | "table.importPzfx"
            | "table.importXlsx"
            | "project.save"
            | "project.saveAs"
            | "file.save"
            | "file.save_as"
    )
}

fn link_spec_allows_internal_only(extra_params: &Value) -> bool {
    let Some(map) = extra_params.as_object() else {
        return true;
    };
    for key in ["link_spec", "linkSpec"] {
        if let Some(spec) = map.get(key)
            && !spec.is_null()
        {
            let Some(source) = spec.get("source").and_then(Value::as_object) else {
                return false;
            };
            let is_safe = source.contains_key("InternalSheet") || source.contains_key("Clipboard");
            if !is_safe || contains_external_delimited_link(spec) {
                return false;
            }
        }
    }
    true
}

/// Validate `extra_params` passed via menu invocation (`ui_invoke_menu` / `ui.invoke_menu`) when
/// `--gui-only` is active without `--workdir` confinement.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::GuiOnlyViolation`] if `extra_params` injects host
/// filesystem paths, external delimited-file links, archive/XML payloads, or external link refresh flags.
pub fn enforce_unconstrained_gui_only_menu_params(cmd_id: Option<&str>, extra_params: &Value) -> Result<(), EngineError> {
    if let Some(path_key) = find_host_path_param_key(extra_params) {
        return Err(EngineError::with_code(
            ErrorCode::GuiOnlyViolation,
            format!("unconstrained host filesystem `{path_key}` parameter via `ui.invoke_menu` is blocked under --gui-only without --workdir"),
        )
        .with_remediation("Configure `--workdir` confinement or use in-memory workflows without host path parameters."));
    }
    if contains_external_delimited_link(extra_params) || (cmd_id == Some("table.pasteLink") && !link_spec_allows_internal_only(extra_params)) {
        return Err(EngineError::with_code(
            ErrorCode::GuiOnlyViolation,
            "external delimited-file link parameters via `ui.invoke_menu` are blocked under --gui-only without --workdir",
        )
        .with_remediation("Use internal sheet links or configure `--workdir` confinement."));
    }
    if let Some(cid) = cmd_id
        && is_unconstrained_archive_or_file_cmd(cid)
        && has_non_empty_json_object(extra_params)
    {
        return Err(EngineError::with_code(
            ErrorCode::GuiOnlyViolation,
            format!("file/archive parameter injection on `{cid}` via `ui.invoke_menu` is blocked under --gui-only without --workdir"),
        )
        .with_remediation("Configure `--workdir` confinement or use visible interactive workspace controls without parameter overrides."));
    }
    if cmd_id == Some("project.recalculate") && has_non_false_flag(extra_params, &["include_external_links", "includeExternalLinks"]) {
        return Err(EngineError::with_code(
            ErrorCode::GuiOnlyViolation,
            "external link recalculation (`include_external_links`) via `ui.invoke_menu` is blocked under --gui-only without --workdir",
        )
        .with_remediation("Configure `--workdir` confinement or trigger parameterless recalculation on in-memory sheets."));
    }
    if cmd_id.is_some_and(|c| matches!(c, "table.importCsv" | "table.importTidyLong")) && has_non_false_flag(extra_params, &["link"]) {
        return Err(EngineError::with_code(
            ErrorCode::GuiOnlyViolation,
            "persistent file linking via `ui.invoke_menu` is blocked under --gui-only without --workdir",
        )
        .with_remediation("Import inline data without persistent file links, or configure `--workdir` confinement."));
    }
    Ok(())
}

fn has_inline_archive_bytes(params: &Value) -> bool {
    params
        .as_object()
        .is_some_and(|map| ["bytes_base64", "bytesBase64", "archive_base64"].into_iter().any(|k| map.get(k).is_some_and(|v| !v.is_null())))
}

fn enforce_unconstrained_gui_only_recalculate(doc: &ProjectDocument, params: &mut Value) -> Result<(), EngineError> {
    let mut explicit_vals = Vec::new();
    if let Some(map) = params.as_object() {
        for key in ["include_external_links", "includeExternalLinks"] {
            if let Some(v) = map.get(key)
                && !v.is_null()
            {
                let Some(b) = v.as_bool() else {
                    return Err(EngineError::with_code(
                        ErrorCode::GuiOnlyViolation,
                        format!("invalid `{key}` value on `project.recalculate` under --gui-only without --workdir"),
                    )
                    .with_remediation("Omit `include_external_links` or pass `false`, or configure `--workdir`."));
                };
                explicit_vals.push(b);
            }
        }
    }
    if explicit_vals.len() > 1 && explicit_vals.windows(2).any(|w| matches!(w, [a, b] if a != b)) {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            "conflicting `include_external_links` alias values supplied for `project.recalculate`",
        ));
    }
    let explicit = explicit_vals.first().copied();
    if explicit == Some(true) || (explicit.is_none() && document_has_external_links(doc)) {
        return Err(EngineError::with_code(
            ErrorCode::GuiOnlyViolation,
            "external delimited-file link recalculation is blocked under --gui-only without --workdir",
        )
        .with_remediation(
            "Restart with `--workdir <dir>` to confine external link reads, or pass `include_external_links: false` for in-memory recalculation.",
        ));
    }
    if params.is_null() {
        *params = json!({ "include_external_links": false });
    } else if let Some(map) = params.as_object_mut() {
        map.remove("includeExternalLinks");
        map.insert(String::from("include_external_links"), Value::Bool(false));
    }
    Ok(())
}

fn enforce_unconstrained_gui_only_session_rules(doc: &ProjectDocument, spec_id: &str, params: &mut Value) -> Result<(), EngineError> {
    if spec_id == "project.recalculate" {
        return enforce_unconstrained_gui_only_recalculate(doc, params);
    }
    if spec_id == "table.pasteLink"
        && (contains_external_delimited_link(params) || find_host_path_param_key(params).is_some() || !link_spec_allows_internal_only(params))
    {
        return Err(EngineError::with_code(
            ErrorCode::GuiOnlyViolation,
            "external delimited-file links in `table.pasteLink` are blocked under --gui-only without --workdir",
        )
        .with_remediation("Link to an in-memory sheet via `source_sheet_id`, or configure `--workdir`."));
    }
    if matches!(spec_id, "table.importCsv" | "table.importTidyLong")
        && params.get("link").is_some_and(|v| !v.is_null() && v.as_bool() != Some(false))
        && find_host_path_param_key(params).is_some()
    {
        return Err(EngineError::with_code(
            ErrorCode::GuiOnlyViolation,
            format!("persistent external file links in `{spec_id}` are blocked under --gui-only without --workdir"),
        )
        .with_remediation("Import inline data without persistent external links, or configure `--workdir`."));
    }
    Ok(())
}

/// Stateful project session owning undo/redo history, dirty tracking, and guarded command execution.
#[derive(Debug, Clone)]
pub struct Session {
    doc: ProjectDocument,
    file_path: Option<PathBuf>,
    dirty: bool,
    gui_only: GuiOnlyPolicy,
    path_policy: ExternalPathPolicy,
    undo_stack: VecDeque<UndoEntry>,
    redo_stack: Vec<UndoEntry>,
    mutation_epoch: u64,
    history_epoch: u64,
    clean_epoch: u64,
}

impl PartialEq for Session {
    fn eq(&self, other: &Self) -> bool {
        self.doc == other.doc
            && self.file_path == other.file_path
            && self.dirty == other.dirty
            && self.gui_only == other.gui_only
            && self.path_policy == other.path_policy
            && self.undo_stack == other.undo_stack
            && self.redo_stack == other.redo_stack
    }
}

fn docs_equal_ignoring_selection(a: &ProjectDocument, b: &ProjectDocument) -> bool {
    let ProjectDocument {
        schema_version: sv_a,
        title: title_a,
        data_sheets: data_a,
        info_sheets: info_a,
        result_sheets: res_a,
        graph_sheets: graph_a,
        layout_sheets: layout_a,
        active_sheet_id: _,
        family_anchor_sheet_id: _,
        checkpoints: cp_a,
        image_assets: img_a,
    } = a;
    let ProjectDocument {
        schema_version: sv_b,
        title: title_b,
        data_sheets: data_b,
        info_sheets: info_b,
        result_sheets: res_b,
        graph_sheets: graph_b,
        layout_sheets: layout_b,
        active_sheet_id: _,
        family_anchor_sheet_id: _,
        checkpoints: cp_b,
        image_assets: img_b,
    } = b;
    sv_a == sv_b
        && title_a == title_b
        && data_a == data_b
        && info_a == info_b
        && res_a == res_b
        && graph_a == graph_b
        && layout_a == layout_b
        && cp_a == cp_b
        && img_a == img_b
}

/// Optional crash-recovery WAL journal hooks backed by `curvebook_format::recovery`.
impl Session {
    /// Open or crash-recover a [`Session`] from `backend`, sanitizing any circular hooks and
    /// replaying committed journal commands strictly on top of the latest snapshot.
    ///
    /// If opening the baseline snapshot or replaying any recovered command fails, the acquired
    /// journal lock lease is released before returning [`EngineError`].
    pub fn open_with_recovery<B: JournalBackend>(
        backend: B,
        owner_id: impl Into<String>,
        ttl_ms: u64,
        now_epoch_ms: u64,
    ) -> Result<(Self, CrashRecoveryJournal<B>, RecoveryReport), EngineError> {
        Self::open_with_recovery_mode(backend, owner_id, ttl_ms, now_epoch_ms, false)
    }

    /// Open or crash-recover a [`Session`] from `backend` in opt-in salvage mode, skipping
    /// individual journal records that cannot be replayed while recording structured
    /// [`JournalRecoveryWarning`] entries in [`RecoveryReport::warnings`] and checkpointing the
    /// salvaged document state.
    pub fn open_with_best_effort_recovery<B: JournalBackend>(
        backend: B,
        owner_id: impl Into<String>,
        ttl_ms: u64,
        now_epoch_ms: u64,
    ) -> Result<(Self, CrashRecoveryJournal<B>, RecoveryReport), EngineError> {
        Self::open_with_recovery_mode(backend, owner_id, ttl_ms, now_epoch_ms, true)
    }

    fn open_with_recovery_mode<B: JournalBackend>(
        backend: B,
        owner_id: impl Into<String>,
        ttl_ms: u64,
        now_epoch_ms: u64,
        skip_unreplayable: bool,
    ) -> Result<(Self, CrashRecoveryJournal<B>, RecoveryReport), EngineError> {
        let (mut journal, mut report) = CrashRecoveryJournal::open_or_recover(backend, owner_id, ttl_ms, now_epoch_ms)?;
        let doc = match &report.snapshot {
            Some(bytes) => match curvebook_format::open_project_bytes(bytes) {
                Ok(doc) => doc,
                Err(err) => {
                    let _ = journal.release_lease();
                    return Err(err.into());
                }
            },
            None => ProjectDocument::default(),
        };
        let mut session = Self::from_document(doc);
        let skipped = match replay_recovered_commands(&mut session, &report, skip_unreplayable) {
            Ok(warnings) => warnings,
            Err(err) => {
                let _ = journal.release_lease();
                return Err(err);
            }
        };
        if !skipped.is_empty() {
            report.warnings.extend(skipped);
            if let Err(err) = session.checkpoint_journal(&mut journal) {
                let _ = journal.release_lease();
                return Err(err);
            }
        }
        Ok((session, journal, report))
    }

    /// Native-only helper opening a [`Session`] backed by [`DirectoryJournalBackend`] in `dir`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_dir_with_recovery(
        dir: impl AsRef<Path>,
        owner_id: impl Into<String>,
        ttl_ms: u64,
        now_epoch_ms: u64,
    ) -> Result<(Self, CrashRecoveryJournal<DirectoryJournalBackend>, RecoveryReport), EngineError> {
        let backend = DirectoryJournalBackend::open_dir(dir)?;
        Self::open_with_recovery(backend, owner_id, ttl_ms, now_epoch_ms)
    }

    /// Native-only opt-in helper opening a [`Session`] backed by [`DirectoryJournalBackend`] in
    /// `dir`, skipping unreplayable WAL records with [`RecoveryReport::warnings`] diagnostics.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_dir_with_best_effort_recovery(
        dir: impl AsRef<Path>,
        owner_id: impl Into<String>,
        ttl_ms: u64,
        now_epoch_ms: u64,
    ) -> Result<(Self, CrashRecoveryJournal<DirectoryJournalBackend>, RecoveryReport), EngineError> {
        let backend = DirectoryJournalBackend::open_dir(dir)?;
        Self::open_with_best_effort_recovery(backend, owner_id, ttl_ms, now_epoch_ms)
    }

    /// Record a command invocation `{ command_id, params }` into the WAL `journal`, returning its sequence number.
    pub fn record_to_journal<B: JournalBackend>(
        &self,
        journal: &mut CrashRecoveryJournal<B>,
        command_id: &str,
        params: &serde_json::Value,
    ) -> Result<u64, EngineError> {
        let entry = json!({
            "command_id": command_id,
            "params": params,
        });
        Ok(journal.record_command(entry)?)
    }

    /// Whether replaying `command_id` from `{ command_id, params }` alone is unsafe across crash
    /// recovery (undo/redo depend on ephemeral stacks; wall-clock-defaulting checkpoint commands
    /// mint non-deterministic timestamps when omitted; `READS_FILESYSTEM` commands depend on
    /// external files that may change or disappear before recovery) and must instead checkpoint an
    /// atomic document snapshot.
    #[must_use]
    pub fn command_requires_wal_checkpoint(command_id: &str) -> bool {
        if matches!(command_id, "edit.undo" | "edit.redo" | "project.createCheckpoint") {
            return true;
        }
        cmd::registry().get(command_id).is_some_and(|spec| spec.effects.contains(cmd::Effects::READS_FILESYSTEM))
    }

    /// Record an already-executed command's effect to `journal` cleanly when `mutated` is true:
    /// checkpoints a full snapshot for undo/redo or external-file-reading commands, or appends a
    /// checksummed WAL record for deterministic mutations.
    pub fn record_outcome_to_journal<B: JournalBackend>(
        &self,
        journal: &mut CrashRecoveryJournal<B>,
        command_id: &str,
        params: &serde_json::Value,
        mutated: bool,
    ) -> Result<(), EngineError> {
        if !mutated {
            return Ok(());
        }
        if Self::command_requires_wal_checkpoint(command_id) {
            return self.checkpoint_journal(journal);
        }
        let _ = self.record_to_journal(journal, command_id, params)?;
        Ok(())
    }

    /// Execute `command_id` transactionally and persist its effect to `journal` only when the
    /// command succeeds and mutates state (atomically checkpointing undo/redo and external-file
    /// readers, or appending deterministic commands to `ops.journal`).
    ///
    /// If persisting to `journal` fails, the in-memory document, undo/redo history, `file_path`,
    /// and `dirty` state are rolled back atomically.
    pub fn execute_and_record<B: JournalBackend>(
        &mut self,
        journal: &mut CrashRecoveryJournal<B>,
        command_id: &str,
        params: serde_json::Value,
    ) -> Result<CommandOutcome, EngineError> {
        let cancelled = AtomicBool::new(false);
        self.execute_with_caller_and_record(journal, command_id, params, Caller::Ui, &cancelled)
    }

    /// Execute `command_id` on behalf of `caller` and persist its effect to `journal` when it
    /// mutates state, rolling back all session mutations atomically if journaling fails.
    pub fn execute_with_caller_and_record<B: JournalBackend>(
        &mut self,
        journal: &mut CrashRecoveryJournal<B>,
        command_id: &str,
        params: serde_json::Value,
        caller: Caller,
        cancelled: &AtomicBool,
    ) -> Result<CommandOutcome, EngineError> {
        let mut journal_params = params.clone();
        let _ = normalize_command_params_in_place(command_id, &mut journal_params);
        self.execute_with_caller_and_hook(command_id, params, caller, cancelled, |session, outcome| {
            session.record_outcome_to_journal(journal, command_id, &journal_params, outcome.mutated)
        })
    }

    /// Persist a full `.curvebook` snapshot of the current document into `journal` and prune replayed WAL entries.
    pub fn checkpoint_journal<B: JournalBackend>(&self, journal: &mut CrashRecoveryJournal<B>) -> Result<(), EngineError> {
        let snapshot_bytes = curvebook_format::save_project_bytes(&self.doc)?;
        journal.checkpoint(&snapshot_bytes)?;
        Ok(())
    }

    /// Flush a clean final snapshot, remove the WAL log and lease lockfile, and return the underlying backend.
    pub fn close_journal_cleanly<B: JournalBackend>(&mut self, journal: CrashRecoveryJournal<B>) -> Result<B, EngineError> {
        let final_snapshot_bytes = curvebook_format::save_project_bytes(&self.doc)?;
        let backend = journal.close_cleanly(&final_snapshot_bytes)?;
        self.dirty = false;
        Ok(backend)
    }
}

fn attach_replay_failure_context(mut err: EngineError, seq: u64, replay_index: usize, command_id: &str) -> EngineError {
    if err.remediation.is_none() {
        err = err.with_remediation("Use Session::open_with_best_effort_recovery to skip unreplayable journal entries while preserving intact edits.");
    }
    let mut details_map = match err.details.take() {
        Some(Value::Object(map)) => map,
        Some(other) => {
            let mut map = serde_json::Map::new();
            map.insert(String::from("cause"), other);
            map
        }
        None => serde_json::Map::new(),
    };
    details_map.entry(String::from("seq")).or_insert_with(|| json!(seq));
    details_map.entry(String::from("replay_index")).or_insert_with(|| json!(replay_index));
    details_map.entry(String::from("command_id")).or_insert_with(|| json!(command_id));
    err.details = Some(Value::Object(details_map));
    err
}

fn replay_recovered_commands(
    session: &mut Session,
    report: &RecoveryReport,
    skip_unreplayable: bool,
) -> Result<Vec<JournalRecoveryWarning>, EngineError> {
    let mut skipped = Vec::new();
    for (idx, cmd) in report.recovered_commands.iter().enumerate() {
        let seq = report.recovered_seqs.get(idx).copied().unwrap_or(0);
        let Some(command_id) = cmd.get("command_id").or_else(|| cmd.get("id")).or_else(|| cmd.get("command")).and_then(serde_json::Value::as_str)
        else {
            if skip_unreplayable {
                skipped.push(JournalRecoveryWarning {
                    line: 0,
                    kind: String::from("skipped_unreplayable_command"),
                    message: format!("skipped recovered journal entry at seq {seq} (index {idx}): missing `command_id` string"),
                });
                continue;
            }
            return Err(EngineError::new("recovered journal entry missing `command_id` string")
                .with_details(json!({ "seq": seq, "replay_index": idx }))
                .with_remediation(
                    "Use Session::open_with_best_effort_recovery to skip unreplayable journal entries while preserving intact edits.",
                ));
        };
        let params = cmd.get("params").cloned().unwrap_or_else(|| json!({}));
        let cancel = AtomicBool::new(false);
        if let Err(err) = session.execute_with_caller(command_id, params, Caller::Replay, &cancel) {
            if skip_unreplayable {
                skipped.push(JournalRecoveryWarning {
                    line: 0,
                    kind: String::from("skipped_unreplayable_command"),
                    message: format!("skipped recovered journal command `{command_id}` at seq {seq} (index {idx}): {err}"),
                });
                continue;
            }
            return Err(attach_replay_failure_context(err, seq, idx, command_id));
        }
    }
    Ok(skipped)
}

fn invoke_spec_run(
    session: &mut Session,
    spec: &cmd::CommandSpec,
    params: Value,
    caller: Caller,
    cancelled: &AtomicBool,
) -> Result<CommandOutcome, EngineError> {
    // Retained until Stage 4 retires r16_5_cli_and_callers.rs
    let scope = cmd::params::DeprecationScope::enter();
    let mut outcome = guard::run_guarded(spec.id, || {
        let mut ctx = cmd::Ctx { session, caller, cancelled };
        (spec.run)(&mut ctx, params)
    })?;
    let warnings = scope.finish_warnings(spec.id);
    if caller != Caller::Replay {
        outcome.warnings.extend(warnings);
    }
    Ok(outcome)
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// Create a fresh empty project session.
    #[must_use]
    pub fn new() -> Self {
        Self::from_document(ProjectDocument::default())
    }

    /// Wrap an existing [`ProjectDocument`] in a clean session.
    #[must_use]
    pub fn from_document(mut document: ProjectDocument) -> Self {
        let _ = curvebook_format::sanitize_circular_hooks(&mut document);
        Self {
            doc: document,
            file_path: None,
            dirty: false,
            gui_only: GuiOnlyPolicy::Disabled,
            path_policy: ExternalPathPolicy::Unconstrained,
            undo_stack: VecDeque::new(),
            redo_stack: Vec::new(),
            mutation_epoch: 0,
            history_epoch: 0,
            clean_epoch: 0,
        }
    }

    /// Return the active [`GuiOnlyPolicy`] restricting direct non-UI command execution.
    #[must_use]
    pub const fn gui_only(&self) -> GuiOnlyPolicy {
        self.gui_only
    }

    /// Set the active [`GuiOnlyPolicy`] on this session.
    pub fn set_gui_only(&mut self, policy: GuiOnlyPolicy) {
        self.gui_only = policy;
    }

    /// Builder helper returning this session configured with `policy`.
    #[must_use]
    pub const fn with_gui_only(mut self, policy: GuiOnlyPolicy) -> Self {
        self.gui_only = policy;
        self
    }

    /// Active external filesystem path policy on this session.
    #[must_use]
    pub const fn path_policy(&self) -> &ExternalPathPolicy {
        &self.path_policy
    }

    /// Replace the active external filesystem path policy on this session.
    pub fn set_path_policy(&mut self, policy: ExternalPathPolicy) {
        self.path_policy = policy;
    }

    /// Fluent builder setting the active external filesystem path policy on this session.
    #[must_use]
    pub fn with_path_policy(mut self, policy: ExternalPathPolicy) -> Self {
        self.path_policy = policy;
        self
    }

    /// Validate a host filesystem write destination against this session's [`ExternalPathPolicy`]
    /// before generating output bytes.
    ///
    /// # Errors
    /// Returns [`EngineError`] with [`ErrorCode::PathEscape`] on policy/escape violation,
    /// [`ErrorCode::BadInput`] if `raw_path` is blank or contains NUL bytes, or
    /// [`ErrorCode::UnsupportedFormat`] on `wasm32`.
    pub fn validate_host_write_path(&self, raw_path: &str) -> Result<String, EngineError> {
        let mut probe = json!({ "path": raw_path });
        self.path_policy.enforce_command_path_params(&mut probe)?;
        #[cfg(target_arch = "wasm32")]
        {
            Err(EngineError::with_code(ErrorCode::UnsupportedFormat, format!("writing `{raw_path}` to local filesystem is unsupported on wasm32"))
                .with_details(json!({ "path": raw_path })))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            probe
                .get("path")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| EngineError::with_code(ErrorCode::InternalError, "validated path missing string value"))
        }
    }

    /// Atomically write `bytes` to `raw_path`, enforcing this session's [`ExternalPathPolicy`]
    /// before touching the host filesystem.
    ///
    /// # Errors
    /// Returns [`EngineError`] with [`ErrorCode::PathEscape`] on policy/escape violation,
    /// [`ErrorCode::BadInput`] if `raw_path` is blank or contains NUL bytes,
    /// [`ErrorCode::IoError`] on filesystem write failure, or
    /// [`ErrorCode::UnsupportedFormat`] on `wasm32`.
    pub fn write_host_file_atomic(&self, raw_path: &str, bytes: &[u8]) -> Result<(), EngineError> {
        self.path_policy.write_atomic_bytes(raw_path, bytes)
    }

    /// Execute an engine command explicitly on behalf of the GUI (`Caller::Ui`), bypassing `GuiOnlyPolicy::Strict`.
    ///
    /// # Errors
    /// Returns [`EngineError`] if the command fails validation, preconditions, or panics inside the guard boundary.
    pub fn execute_as_ui(&mut self, command_id: &str, params: serde_json::Value) -> Result<CommandOutcome, EngineError> {
        let cancelled = AtomicBool::new(false);
        self.execute_with_caller(command_id, params, Caller::Ui, &cancelled)
    }

    /// Borrow the active [`ProjectDocument`].
    #[must_use]
    pub fn document(&self) -> &ProjectDocument {
        &self.doc
    }

    /// Borrow the associated archive file path, if saved or opened from disk.
    #[must_use]
    pub fn file_path(&self) -> Option<&Path> {
        self.file_path.as_deref()
    }

    /// Associate or clear the archive file path and reset dirty tracking.
    pub fn set_file_path(&mut self, path: Option<PathBuf>, mark_clean: bool) {
        self.file_path = path;
        if mark_clean {
            self.dirty = false;
        }
    }

    pub(crate) fn mark_saved_clean(&mut self, path: PathBuf) {
        self.set_file_path(Some(path), true);
        self.clean_epoch = self.clean_epoch.wrapping_add(1);
    }

    /// Whether the project has unsaved edits since creation or the last clean save/load.
    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Current number of snapshots retained on the undo stack.
    #[must_use]
    pub fn undo_depth(&self) -> usize {
        self.undo_stack.len()
    }

    /// Current number of snapshots retained on the redo stack.
    #[must_use]
    pub fn redo_depth(&self) -> usize {
        self.redo_stack.len()
    }

    /// Total allocated data-table cell slots across all retained undo snapshots.
    #[must_use]
    pub fn undo_cell_slots(&self) -> usize {
        self.undo_stack.iter().fold(0usize, |acc, entry| acc.saturating_add(entry.cell_slots))
    }

    fn undo_checkpoint_bytes(&self) -> usize {
        self.undo_stack.iter().fold(0usize, |acc, entry| acc.saturating_add(entry.checkpoint_bytes))
    }

    fn push_undo_snapshot(&mut self, doc: ProjectDocument, file_path: Option<PathBuf>) {
        self.undo_stack.push_back(UndoEntry::from_snapshot(doc, file_path));
        let mut total_cells = self.undo_cell_slots();
        let mut total_cp_bytes = self.undo_checkpoint_bytes();
        while self.undo_stack.len() > 1
            && (self.undo_stack.len() > MAX_UNDO_ENTRIES || total_cells > MAX_UNDO_CELL_BUDGET || total_cp_bytes > MAX_UNDO_CHECKPOINT_BYTES_BUDGET)
        {
            if let Some(evicted) = self.undo_stack.pop_front() {
                total_cells = total_cells.saturating_sub(evicted.cell_slots);
                total_cp_bytes = total_cp_bytes.saturating_sub(evicted.checkpoint_bytes);
            }
        }
    }

    /// Collapse a sequence of mutating operations executed after `baseline` into a single atomic
    /// undo step on `self`, returning `true` when the document content or history changed.
    ///
    /// Read-only sheet selection changes (`active_sheet_id` / `family_anchor_sheet_id`) do not
    /// push spurious undo entries, wipe `redo_stack`, or dirty the session; clean `project.save` /
    /// `project.open` transitions preserve `dirty == false`; and batches containing `edit.undo` /
    /// `edit.redo` retain their exact undo/redo stack state instead of inverting history.
    pub fn collapse_atomic_group(&mut self, mut baseline: Self) -> bool {
        let content_changed = !docs_equal_ignoring_selection(&self.doc, &baseline.doc)
            || (self.mutation_epoch > baseline.mutation_epoch && self.file_path != baseline.file_path);
        let history_changed = self.history_epoch > baseline.history_epoch;
        let saved_or_opened_clean = self.clean_epoch > baseline.clean_epoch;

        if history_changed {
            return true;
        }

        if content_changed {
            let pre_doc = std::mem::take(&mut baseline.doc);
            let pre_path = baseline.file_path.take();
            self.undo_stack = baseline.undo_stack;
            self.push_undo_snapshot(pre_doc, pre_path);
            self.redo_stack.clear();
            if !saved_or_opened_clean {
                self.dirty = true;
            }
            true
        } else {
            self.undo_stack = baseline.undo_stack;
            self.redo_stack = baseline.redo_stack;
            if !saved_or_opened_clean {
                self.dirty = baseline.dirty;
            }
            false
        }
    }

    /// Execute a multi-step session operation `f` as a single atomic transaction: if `f` returns
    /// `Err`, the entire [`Session`] (document, undo/redo stacks, dirty state, path) rolls back
    /// cleanly; if `f` returns `Ok` and mutated the document, intermediate undo entries collapse into
    /// a single undo step.
    pub fn execute_atomic_group<T, E>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, E>) -> Result<T, E> {
        let baseline = self.clone();
        match f(self) {
            Ok(val) => {
                let _ = self.collapse_atomic_group(baseline);
                Ok(val)
            }
            Err(err) => {
                *self = baseline;
                Err(err)
            }
        }
    }

    /// Execute an engine command with panic recovery and transactional undo recording.
    ///
    /// # Errors
    /// Returns [`EngineError`] if the command identifier is unknown, parameters are invalid, or a
    /// guarded panic occurs; mutating commands roll back atomically on failure.
    pub fn execute(&mut self, command_id: &str, params: serde_json::Value) -> Result<CommandOutcome, EngineError> {
        let cancelled = AtomicBool::new(false);
        self.execute_with_caller(command_id, params, Caller::Ui, &cancelled)
    }

    /// Execute an engine command on behalf of `caller` with cooperative cancellation,
    /// declarative [`cmd::Registry`] lookup, panic recovery, and transactional undo recording.
    ///
    /// # Errors
    /// Returns [`EngineError`] with [`ErrorCode::Cancelled`] if `cancelled` is set before dispatch,
    /// [`ErrorCode::Conflict`] if the registered command is disabled in this session,
    /// [`ErrorCode::NotFound`] if the command is unknown, [`ErrorCode::InternalPanic`] on caught
    /// unwinds, or domain errors; mutating commands roll back atomically on failure.
    pub fn execute_with_caller(
        &mut self,
        command_id: &str,
        params: Value,
        caller: Caller,
        cancelled: &AtomicBool,
    ) -> Result<CommandOutcome, EngineError> {
        self.execute_with_caller_and_hook(command_id, params, caller, cancelled, |_, _| Ok(()))
    }

    fn execute_with_caller_and_hook<F>(
        &mut self,
        command_id: &str,
        params: Value,
        caller: Caller,
        cancelled: &AtomicBool,
        after_commit: F,
    ) -> Result<CommandOutcome, EngineError>
    where
        F: FnOnce(&Self, &CommandOutcome) -> Result<(), EngineError>,
    {
        if cancelled.load(Ordering::Relaxed) {
            return Err(EngineError::with_code(ErrorCode::Cancelled, format!("command `{command_id}` cancelled"))
                .with_retryable(true)
                .with_details(json!({ "command_id": command_id })));
        }

        let spec = cmd::registry().get(command_id).ok_or_else(|| {
            EngineError::with_code(ErrorCode::NotFound, format!("unknown engine command `{command_id}`"))
                .with_remediation("Inspect the command registry to discover valid command IDs.")
                .with_details(json!({ "command_id": command_id }))
        })?;
        self.execute_registered_spec_with_hook(spec, params, caller, cancelled, after_commit)
    }

    #[cfg(test)]
    fn execute_registered_spec(
        &mut self,
        spec: &cmd::CommandSpec,
        params: Value,
        caller: Caller,
        cancelled: &AtomicBool,
    ) -> Result<CommandOutcome, EngineError> {
        self.execute_registered_spec_with_hook(spec, params, caller, cancelled, |_, _| Ok(()))
    }

    fn execute_registered_spec_with_hook<F>(
        &mut self,
        spec: &cmd::CommandSpec,
        mut params: Value,
        caller: Caller,
        cancelled: &AtomicBool,
        after_commit: F,
    ) -> Result<CommandOutcome, EngineError>
    where
        F: FnOnce(&Self, &CommandOutcome) -> Result<(), EngineError>,
    {
        if self.gui_only == GuiOnlyPolicy::Strict && !is_strict_gui_only_allowed(spec, caller, &params) {
            return Err(EngineError::with_code(
                ErrorCode::GuiOnlyViolation,
                format!("command `{}` cannot be executed directly by `{caller:?}` under strict gui_only policy", spec.id),
            )
            .with_remediation("Blocked by --gui-only: drive the interface through GUI actions instead of direct domain commands.")
            .with_details(json!({ "command_id": spec.id, "caller": caller })));
        }
        if self.gui_only == GuiOnlyPolicy::Strict && !self.path_policy.is_constrained() {
            enforce_unconstrained_gui_only_session_rules(&self.doc, spec.id, &mut params)?;
        }
        if !(spec.enabled)(self) {
            return Err(EngineError::with_code(ErrorCode::Conflict, format!("command `{}` is currently disabled", spec.id))
                .with_details(json!({ "command_id": spec.id })));
        }
        if self.path_policy.is_constrained() && spec.effects.intersects(cmd::Effects::READS_FILESYSTEM.with(cmd::Effects::WRITES_FILESYSTEM)) {
            self.path_policy.enforce_command_path_params(&mut params)?;
        }
        if spec.id == "edit.undo" || spec.id == "edit.redo" {
            return self.execute_undo_redo_with_hook(spec, params, caller, cancelled, after_commit);
        }

        let read_only =
            !spec.effects.contains(cmd::Effects::MUTATES_DOCUMENT) || (spec.id == "table.convertArchetype" && is_preview_only_convert(&params));
        if read_only {
            let before_active = self.doc.active_sheet_id.clone();
            let before_anchor = self.doc.family_anchor_sheet_id.clone();
            let before_path = self.file_path.clone();
            let before_dirty = self.dirty;
            let before_clean_epoch = self.clean_epoch;
            let res = invoke_spec_run(self, spec, params, caller, cancelled)?;
            if let Err(hook_err) = after_commit(self, &res) {
                self.doc.active_sheet_id = before_active;
                self.doc.family_anchor_sheet_id = before_anchor;
                self.file_path = before_path;
                self.dirty = before_dirty;
                self.clean_epoch = before_clean_epoch;
                return Err(hook_err);
            }
            return Ok(res);
        }

        self.execute_mutating_spec_with_hook(spec, params, caller, cancelled, after_commit)
    }

    fn execute_undo_redo_with_hook<F>(
        &mut self,
        spec: &cmd::CommandSpec,
        params: Value,
        caller: Caller,
        cancelled: &AtomicBool,
        after_commit: F,
    ) -> Result<CommandOutcome, EngineError>
    where
        F: FnOnce(&Self, &CommandOutcome) -> Result<(), EngineError>,
    {
        let before_dirty = self.dirty;
        let before_history_epoch = self.history_epoch;
        let redo_may_evict = spec.id == "edit.redo"
            && !self.redo_stack.is_empty()
            && (self.undo_stack.len() >= MAX_UNDO_ENTRIES
                || self.undo_cell_slots().saturating_add(document_allocated_cell_slots(&self.doc)) > MAX_UNDO_CELL_BUDGET
                || self.undo_checkpoint_bytes().saturating_add(document_checkpoint_bytes(&self.doc)) > MAX_UNDO_CHECKPOINT_BYTES_BUDGET);
        let undo_backup = redo_may_evict.then(|| self.undo_stack.clone());

        let res = invoke_spec_run(self, spec, params, caller, cancelled)?;
        if let Err(hook_err) = after_commit(self, &res) {
            if res.mutated {
                if spec.id == "edit.undo" {
                    if let Some(before_entry) = self.redo_stack.pop() {
                        let undone_doc = std::mem::replace(&mut self.doc, before_entry.doc);
                        let undone_path = std::mem::replace(&mut self.file_path, before_entry.file_path);
                        self.undo_stack.push_back(UndoEntry::from_snapshot(undone_doc, undone_path));
                    }
                } else if let Some(prev_entry) = self.undo_stack.pop_back() {
                    let redone_doc = std::mem::replace(&mut self.doc, prev_entry.doc);
                    let redone_path = std::mem::replace(&mut self.file_path, prev_entry.file_path);
                    self.redo_stack.push(UndoEntry::from_snapshot(redone_doc, redone_path));
                    if let Some(full_undo) = undo_backup {
                        self.undo_stack = full_undo;
                    }
                }
                self.dirty = before_dirty;
                self.history_epoch = before_history_epoch;
            }
            return Err(hook_err);
        }
        Ok(res)
    }

    fn execute_mutating_spec_with_hook<F>(
        &mut self,
        spec: &cmd::CommandSpec,
        params: Value,
        caller: Caller,
        cancelled: &AtomicBool,
        after_commit: F,
    ) -> Result<CommandOutcome, EngineError>
    where
        F: FnOnce(&Self, &CommandOutcome) -> Result<(), EngineError>,
    {
        let opens_project_path = spec.id == "project.open" && params.get("path").and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty());
        let clears_file_path_on_mutate = matches!(spec.id, "file.new" | "project.new" | "project.instantiateTemplate")
            || (matches!(spec.id, "table.importPrism" | "table.importPzfx" | "table.importXlsx")
                && !params.get("merge").and_then(Value::as_bool).unwrap_or(false))
            || (spec.id == "project.open" && !opens_project_path);
        let has_inline_bytes = has_inline_archive_bytes(&params);
        let before = self.doc.clone();
        let before_path = self.file_path.clone();
        let before_dirty = self.dirty;
        let outcome = invoke_spec_run(self, spec, params, caller, cancelled);

        match outcome {
            Ok(res) => {
                if res.mutated {
                    if let Err(err) = curvebook_format::validate_project_document(&self.doc) {
                        self.doc = before;
                        self.file_path = before_path;
                        self.dirty = before_dirty;
                        return Err(err.into());
                    }
                    if self.gui_only == GuiOnlyPolicy::Strict
                        && !self.path_policy.is_constrained()
                        && matches!(spec.id, "project.open" | "project.instantiateTemplate")
                        && has_inline_bytes
                        && document_has_external_links(&self.doc)
                    {
                        self.doc = before;
                        self.file_path = before_path;
                        self.dirty = before_dirty;
                        return Err(EngineError::with_code(
                            ErrorCode::GuiOnlyViolation,
                            format!(
                                "inline archive containing external delimited-file links is blocked for `{}` under --gui-only without --workdir",
                                spec.id
                            ),
                        )
                        .with_remediation("Configure `--workdir` confinement or remove external delimited-file links from the archive."));
                    }
                    if clears_file_path_on_mutate {
                        self.file_path = None;
                    }
                    self.dirty = !opens_project_path;
                    if let Err(hook_err) = after_commit(self, &res) {
                        self.doc = before;
                        self.file_path = before_path;
                        self.dirty = before_dirty;
                        return Err(hook_err);
                    }
                    self.push_undo_snapshot(before, before_path);
                    self.redo_stack.clear();
                    self.mutation_epoch = self.mutation_epoch.wrapping_add(1);
                    if opens_project_path {
                        self.clean_epoch = self.clean_epoch.wrapping_add(1);
                    }
                } else {
                    after_commit(self, &res)?;
                }
                Ok(res)
            }
            Err(err) => {
                self.doc = before;
                self.file_path = before_path;
                self.dirty = before_dirty;
                Err(err)
            }
        }
    }

    /// Restore the previous document snapshot onto the active document (along with its paired file path).
    pub fn undo(&mut self) -> Result<bool, EngineError> {
        let Some(prev) = self.undo_stack.pop_back() else {
            return Ok(false);
        };
        let current_doc = std::mem::replace(&mut self.doc, prev.doc);
        let current_path = std::mem::replace(&mut self.file_path, prev.file_path);
        self.redo_stack.push(UndoEntry::from_snapshot(current_doc, current_path));
        self.dirty = true;
        self.history_epoch = self.history_epoch.wrapping_add(1);
        Ok(true)
    }

    /// Re-apply the most recently undone document snapshot (along with its paired file path).
    pub fn redo(&mut self) -> Result<bool, EngineError> {
        let Some(next) = self.redo_stack.pop() else {
            return Ok(false);
        };
        let current_doc = std::mem::replace(&mut self.doc, next.doc);
        let current_path = std::mem::replace(&mut self.file_path, next.file_path);
        self.push_undo_snapshot(current_doc, current_path);
        self.dirty = true;
        self.history_epoch = self.history_epoch.wrapping_add(1);
        Ok(true)
    }

    /// Re-evaluate all upstream-to-downstream dependencies in topological order, returning the full report.
    pub fn recalculate_all_with_report(&mut self) -> Result<dag::RecalcReport, EngineError> {
        guard::run_guarded("project.recalculate", || {
            let (report, _) = dag::recalculate_project_sanitized_with_report(&mut self.doc)?;
            Ok(report)
        })
    }

    /// Re-evaluate all upstream-to-downstream dependencies in topological order.
    pub fn recalculate_all(&mut self) -> Result<(), EngineError> {
        let _ = self.recalculate_all_with_report()?;
        Ok(())
    }

    /// Return structured failure details for any result sheets currently in [`curvebook_doc::ResultStatus::Failed`].
    #[must_use]
    pub fn failed_result_sheets(&self) -> Vec<dag::FailedResultSheet> {
        dag::collect_failed_result_sheets(&self.doc)
    }

    /// Compile a graph or layout sheet into a resolution-independent [`ChartScene`].
    pub fn render_sheet_scene(&self, sheet_id: &SheetId) -> Result<ChartScene, EngineError> {
        guard::run_guarded("session.render_sheet_scene", || cmd::helpers::compile_sheet_scene(&self.doc, sheet_id))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use curvebook_doc::{ErrorCode, SheetId};
    use serde_json::json;

    use super::{Caller, EngineError, GuiOnlyPolicy, Session, cmd};

    #[test]
    fn engine_error_formats_message() {
        let err = EngineError::new("table not found");
        assert_eq!(err.to_string(), "table not found");
    }

    #[test]
    fn session_execute_undo_redo_and_render_smoke() {
        assert!(cmd::registry().get("sheet.createData").is_some());

        let mut session = Session::new();
        assert!(!session.is_dirty());

        let outcome =
            session.execute("sheet.createData", json!({ "sheet_id": "data-1", "title": "Dose", "graph_sheet_id": "graph-1" })).expect("create data");
        assert!(outcome.mutated);
        assert!(session.is_dirty());
        assert!(session.document().data_sheet(&SheetId::new("data-1")).is_some());

        session.execute("table.setCell", json!({ "sheet_id": "data-1", "row": 0, "col": 0, "value": 42.5 })).expect("set cell");
        let scene = session.render_sheet_scene(&SheetId::new("graph-1")).expect("render scene");
        assert!(scene.width_pt > 0.0);

        assert!(session.undo().expect("undo cell"));
        assert!(session.redo().expect("redo cell"));
        session.recalculate_all().expect("recalculate");
    }

    #[test]
    fn gui_only_policy_blocks_non_ui_mutations_while_allowing_ui_and_inspection() {
        let mut session = Session::new().with_gui_only(GuiOnlyPolicy::Strict);
        assert_eq!(session.gui_only(), GuiOnlyPolicy::Strict);
        let cancelled = AtomicBool::new(false);

        for blocked_caller in [Caller::Mcp, Caller::Control, Caller::Cli] {
            let err = session
                .execute_with_caller("sheet.createData", json!({ "sheet_id": "d1" }), blocked_caller, &cancelled)
                .expect_err("mutating command should fail in Strict mode");
            assert_eq!(err.code, ErrorCode::GuiOnlyViolation);
            assert!(err.remediation.as_deref().unwrap_or("").contains("Blocked by --gui-only"));

            let inspect = session
                .execute_with_caller("project.inspect", json!({}), blocked_caller, &cancelled)
                .expect("read-only project inspection remains permitted");
            assert!(!inspect.mutated);
        }

        let created = session
            .execute_as_ui("sheet.createData", json!({ "sheet_id": "d1", "title": "UI Created", "graph_sheet_id": "g1" }))
            .expect("UI caller always allowed");
        assert!(created.mutated);

        // UI-state-changing commands like `sheet.select` must also be rejected for non-UI callers in Strict mode,
        // while remaining allowed for `Caller::Ui`.
        for blocked_caller in [Caller::Mcp, Caller::Control, Caller::Cli] {
            let select_err = session
                .execute_with_caller("sheet.select", json!({ "sheet_id": "g1" }), blocked_caller, &cancelled)
                .expect_err("sheet.select must fail for non-UI callers in Strict mode");
            assert_eq!(select_err.code, ErrorCode::GuiOnlyViolation);
        }
        let ui_selected = session.execute_as_ui("sheet.select", json!({ "sheet_id": "g1" })).expect("UI caller may select sheet");
        assert!(!ui_selected.mutated);

        // Compute and export commands are also blocked for non-UI callers under Strict policy.
        let compute_err = session
            .execute_with_caller("analysis.run", json!({ "data_sheet_id": "d1", "kind": "descriptive_stats" }), Caller::Mcp, &cancelled)
            .expect_err("direct analysis compute blocked for MCP");
        assert_eq!(compute_err.code, ErrorCode::GuiOnlyViolation);

        session.set_gui_only(GuiOnlyPolicy::Disabled);
        assert_eq!(session.gui_only(), GuiOnlyPolicy::Disabled);
        session
            .execute_with_caller("table.setCell", json!({ "sheet_id": "d1", "row": 0, "col": 0, "value": 3.5 }), Caller::Cli, &cancelled)
            .expect("Disabled policy allows Cli again");
    }

    #[test]
    fn host_write_helpers_enforce_reject_external_and_workdir_containment() {
        use std::path::Path;

        use crate::ExternalPathPolicy;

        let reject = Session::new().with_path_policy(ExternalPathPolicy::RejectExternal);
        let err = reject.validate_host_write_path("/tmp/nope.png").expect_err("RejectExternal must reject validation");
        assert_eq!(err.code, ErrorCode::PathEscape);
        let write_err = reject.write_host_file_atomic("/tmp/nope.png", b"data").expect_err("RejectExternal must reject write");
        assert_eq!(write_err.code, ErrorCode::PathEscape);

        let wd = std::env::temp_dir().join(format!("cb-engine-host-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&wd);
        std::fs::create_dir_all(&wd).expect("create temp workdir");
        let guard = curvebook_platform::WorkdirGuard::new(&wd).expect("guard");
        let confined = Session::new().with_path_policy(ExternalPathPolicy::from_guard(guard));

        let esc = confined.validate_host_write_path("../outside.pdf").expect_err("traversal must fail");
        assert_eq!(esc.code, ErrorCode::PathEscape);

        let valid = confined.validate_host_write_path("nested/out.png").expect("valid inside workdir");
        assert!(Path::new(&valid).starts_with(wd.canonicalize().expect("canon wd")));
        confined.write_host_file_atomic("nested/out.png", b"png-bytes").expect("atomic write inside workdir");
        assert_eq!(std::fs::read(wd.join("nested/out.png")).expect("read written file"), b"png-bytes");

        let _ = std::fs::remove_dir_all(&wd);
    }

    #[test]
    fn strict_gui_only_unconstrained_blocks_inline_external_links_and_external_recalc() {
        use curvebook_doc::{CellCoord, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, LinkSource, LinkedDataRegion, SheetCellRange};

        let mut builder = Session::new();
        builder.execute("sheet.createData", json!({ "sheet_id": "d1", "title": "Data", "kind": "xy" })).expect("create data sheet");
        let mut doc = builder.document().clone();
        doc.data_sheet_mut(&SheetId::new("d1")).expect("d1").linked_regions.push(LinkedDataRegion {
            id: String::from("ext-1"),
            spec: ImportLinkSpec {
                source: LinkSource::ExternalDelimitedFile { path: String::from("/etc/passwd"), delimiter: ',', decimal_separator: '.' },
                filter: ImportFilterSpec::default(),
                placement: ImportPlacementSpec::default(),
            },
            target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
            provenance_summary: String::from("external"),
        });
        let archive_bytes = curvebook_format::save_project_bytes(&doc).expect("save bytes");
        let b64 = crate::cmd::helpers::encode_base64(&archive_bytes);

        let mut session = Session::new().with_gui_only(GuiOnlyPolicy::Strict);
        let open_err = session
            .execute_as_ui("project.open", json!({ "bytes_base64": b64 }))
            .expect_err("inline archive with external link blocked under unconstrained gui-only");
        assert_eq!(open_err.code, ErrorCode::GuiOnlyViolation);
        assert!(session.document().data_sheets.is_empty(), "failed open must roll back doc");

        // Pre-loaded doc containing external link must still refuse implicit/explicit external recalc.
        let mut preloaded = Session::from_document(doc).with_gui_only(GuiOnlyPolicy::Strict);
        let exp_err = preloaded
            .execute_as_ui("project.recalculate", json!({ "include_external_links": true }))
            .expect_err("explicit external link recalc blocked");
        assert_eq!(exp_err.code, ErrorCode::GuiOnlyViolation);
        let imp_err = preloaded.execute_as_ui("project.recalculate", json!({})).expect_err("implicit external link recalc blocked");
        assert_eq!(imp_err.code, ErrorCode::GuiOnlyViolation);
        let safe_ok = preloaded.execute_as_ui("project.recalculate", json!({ "include_external_links": false }));
        assert!(safe_ok.is_ok(), "explicit in-memory recalc remains allowed: {safe_ok:?}");

        // Pure in-memory session allows Analyze/Recalculate All `{}` cleanly.
        let mut pure_mem = Session::new().with_gui_only(GuiOnlyPolicy::Strict);
        pure_mem.execute_as_ui("sheet.createData", json!({ "sheet_id": "m1", "kind": "Column" })).expect("create m1");
        pure_mem.execute_as_ui("project.recalculate", json!({})).expect("in-memory recalc succeeds");
    }
}
