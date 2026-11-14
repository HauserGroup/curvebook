//! Core session, undo/redo history, reactive `Data -> Analysis -> Graph -> Layout` recalculation
//! pipeline, command registry and panic recovery boundary for LumenCraft.
//!
//! Layer L5 crate: shared identically by the desktop app, browser WASM build, headless CLI and MCP
//! server. Nothing in or below this crate depends on a UI toolkit.
//! Governing spec: `docs/architecture.md`, `docs/data-tables-and-transforms.md` (§6), and `docs/development.md`.
#![forbid(unsafe_code)]

pub mod cmd;
pub use cmd::Caller;
pub mod dag;
pub mod guard;
pub mod methods_report;

use std::collections::VecDeque;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use lumencraft_chart::{ChartError, ChartScene};
use lumencraft_doc::{DocError, ProjectDocument, SheetId, SummaryPointError};
pub use lumencraft_doc::{ErrorCategory, ErrorCode, RemediationKind};
use lumencraft_fit::FitError;
#[cfg(not(target_arch = "wasm32"))]
use lumencraft_format::DirectoryJournalBackend;
use lumencraft_format::FormatError;
use lumencraft_format::recovery::{CrashRecoveryJournal, JournalBackend, RecoveryReport};
use lumencraft_math::MathError;
use lumencraft_pdf::PdfError;
use lumencraft_platform::PlatformError;
use lumencraft_render::RenderError;
use lumencraft_stats::StatsError;
use lumencraft_svg::SvgError;
use lumencraft_transform::TransformError;
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
            DocError::TooManySheets { count, max } => {
                (ErrorCode::ResourceLimit, Some(json!({ "count": count, "max": max })), Some("Delete unused sheets before adding more."))
            }
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
            FormatError::UnsafePath { path } => {
                (ErrorCode::PathEscape, Some(json!({ "path": path })), Some("Ensure all archive or file paths stay inside the allowed root."), false)
            }
            FormatError::DuplicateEntry { path } => (ErrorCode::AlreadyExists, Some(json!({ "path": path })), None, false),
            FormatError::MissingArchiveEntry { path } => (ErrorCode::CorruptData, Some(json!({ "path": path })), None, false),
            FormatError::UnsupportedSchemaVersion { version, max_supported } => (
                ErrorCode::UnsupportedVersion,
                Some(json!({ "version": version, "max_supported": max_supported })),
                Some("Upgrade Lumencraft to open files created with a newer schema version."),
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
    use lumencraft_platform::TimeoutKind;

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
            Ok(CommandOutcome { command_id: String::from("test.read"), mutated: false, message: String::from("ok"), payload: json!({}) })
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
}

impl CommandOutcome {
    /// Construct a mutating command outcome.
    #[must_use]
    pub fn mutated(command_id: impl Into<String>, message: impl Into<String>, payload: serde_json::Value) -> Self {
        Self { command_id: command_id.into(), mutated: true, message: message.into(), payload }
    }

    /// Construct a read-only command outcome (leaves undo stack and dirty state untouched).
    #[must_use]
    pub fn read_only(command_id: impl Into<String>, message: impl Into<String>, payload: serde_json::Value) -> Self {
        Self { command_id: command_id.into(), mutated: false, message: message.into(), payload }
    }
}

/// Maximum number of document snapshots retained on a session's undo stack.
pub const MAX_UNDO_ENTRIES: usize = 64;

/// Maximum cumulative allocated data-table cell slots retained across undo snapshots.
///
/// The newest snapshot is always retained even if it alone exceeds this budget.
pub const MAX_UNDO_CELL_BUDGET: usize = 25_000_000;

#[derive(Debug, Clone, PartialEq)]
struct UndoEntry {
    doc: ProjectDocument,
    cell_slots: usize,
}

impl UndoEntry {
    fn from_document(doc: ProjectDocument) -> Self {
        let cell_slots = document_allocated_cell_slots(&doc);
        Self { doc, cell_slots }
    }
}

fn document_allocated_cell_slots(doc: &ProjectDocument) -> usize {
    doc.data_sheets.iter().fold(0usize, |acc, sheet| acc.saturating_add(sheet.archetype.allocated_cell_slots()))
}

fn is_preview_only_convert(params: &serde_json::Value) -> bool {
    ["preview_only", "previewOnly", "preview", "dry_run", "dryRun"]
        .iter()
        .any(|key| params.get(key).and_then(serde_json::Value::as_bool) == Some(true))
}

/// Stateful project session owning undo/redo history, dirty tracking, and guarded command execution.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    doc: ProjectDocument,
    file_path: Option<PathBuf>,
    dirty: bool,
    undo_stack: VecDeque<UndoEntry>,
    redo_stack: Vec<ProjectDocument>,
}

/// Optional crash-recovery WAL journal hooks backed by `lumencraft_format::recovery`.
impl Session {
    /// Open or crash-recover a [`Session`] from `backend`, sanitizing any circular hooks and
    /// replaying committed journal commands on top of the latest snapshot.
    pub fn open_with_recovery<B: JournalBackend>(
        backend: B,
        owner_id: impl Into<String>,
        ttl_ms: u64,
        now_epoch_ms: u64,
    ) -> Result<(Self, CrashRecoveryJournal<B>, RecoveryReport), EngineError> {
        let (journal, report) = CrashRecoveryJournal::open_or_recover(backend, owner_id, ttl_ms, now_epoch_ms)?;
        let doc = match &report.snapshot {
            Some(bytes) => lumencraft_format::open_project_bytes(bytes)?,
            None => ProjectDocument::default(),
        };
        let mut session = Self::from_document(doc);
        for cmd in &report.recovered_commands {
            let command_id = cmd
                .get("command_id")
                .or_else(|| cmd.get("id"))
                .or_else(|| cmd.get("command"))
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| EngineError::new("recovered journal entry missing `command_id` string"))?;
            let params = cmd.get("params").cloned().unwrap_or_else(|| json!({}));
            let _ = session.execute(command_id, params)?;
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

    /// Execute `command_id` transactionally and append to `journal` only when the command succeeds and mutates state.
    pub fn execute_and_record<B: JournalBackend>(
        &mut self,
        journal: &mut CrashRecoveryJournal<B>,
        command_id: &str,
        params: serde_json::Value,
    ) -> Result<CommandOutcome, EngineError> {
        let outcome = self.execute(command_id, params.clone())?;
        if outcome.mutated {
            let _ = self.record_to_journal(journal, command_id, &params)?;
        }
        Ok(outcome)
    }

    /// Persist a full `.lcproj` snapshot of the current document into `journal` and prune replayed WAL entries.
    pub fn checkpoint_journal<B: JournalBackend>(&self, journal: &mut CrashRecoveryJournal<B>) -> Result<(), EngineError> {
        let snapshot_bytes = lumencraft_format::save_project_bytes(&self.doc)?;
        journal.checkpoint(&snapshot_bytes)?;
        Ok(())
    }

    /// Flush a clean final snapshot, remove the WAL log and lease lockfile, and return the underlying backend.
    pub fn close_journal_cleanly<B: JournalBackend>(&mut self, journal: CrashRecoveryJournal<B>) -> Result<B, EngineError> {
        let final_snapshot_bytes = lumencraft_format::save_project_bytes(&self.doc)?;
        let backend = journal.close_cleanly(&final_snapshot_bytes)?;
        self.dirty = false;
        Ok(backend)
    }
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
        let _ = lumencraft_format::sanitize_circular_hooks(&mut document);
        Self { doc: document, file_path: None, dirty: false, undo_stack: VecDeque::new(), redo_stack: Vec::new() }
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

    fn push_undo_snapshot(&mut self, doc: ProjectDocument) {
        self.undo_stack.push_back(UndoEntry::from_document(doc));
        let mut total_cells = self.undo_cell_slots();
        while self.undo_stack.len() > 1 && (self.undo_stack.len() > MAX_UNDO_ENTRIES || total_cells > MAX_UNDO_CELL_BUDGET) {
            if let Some(evicted) = self.undo_stack.pop_front() {
                total_cells = total_cells.saturating_sub(evicted.cell_slots);
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
        self.execute_registered_spec(spec, params, caller, cancelled)
    }

    fn execute_registered_spec(
        &mut self,
        spec: &cmd::CommandSpec,
        params: Value,
        caller: Caller,
        cancelled: &AtomicBool,
    ) -> Result<CommandOutcome, EngineError> {
        if !(spec.enabled)(self) {
            return Err(EngineError::with_code(ErrorCode::Conflict, format!("command `{}` is currently disabled", spec.id))
                .with_details(json!({ "command_id": spec.id })));
        }
        if spec.id == "edit.undo" || spec.id == "edit.redo" {
            return guard::run_guarded(spec.id, || {
                let mut ctx = cmd::Ctx { session: self, caller, cancelled };
                (spec.run)(&mut ctx, params)
            });
        }

        let read_only =
            !spec.effects.contains(cmd::Effects::MUTATES_DOCUMENT) || (spec.id == "table.convertArchetype" && is_preview_only_convert(&params));
        if read_only {
            return guard::run_guarded(spec.id, || {
                let mut ctx = cmd::Ctx { session: self, caller, cancelled };
                (spec.run)(&mut ctx, params)
            });
        }

        let before = self.doc.clone();
        let before_path = self.file_path.clone();
        let before_dirty = self.dirty;
        let outcome = guard::run_guarded(spec.id, || {
            let mut ctx = cmd::Ctx { session: self, caller, cancelled };
            (spec.run)(&mut ctx, params)
        });

        match outcome {
            Ok(res) => {
                if res.mutated {
                    self.push_undo_snapshot(before);
                    self.redo_stack.clear();
                    self.dirty = true;
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

    /// Restore the previous document snapshot onto the active document.
    pub fn undo(&mut self) -> Result<bool, EngineError> {
        let Some(prev) = self.undo_stack.pop_back() else {
            return Ok(false);
        };
        let current = std::mem::replace(&mut self.doc, prev.doc);
        self.redo_stack.push(current);
        self.dirty = true;
        Ok(true)
    }

    /// Re-apply the most recently undone document snapshot.
    pub fn redo(&mut self) -> Result<bool, EngineError> {
        let Some(next) = self.redo_stack.pop() else {
            return Ok(false);
        };
        let current = std::mem::replace(&mut self.doc, next);
        self.push_undo_snapshot(current);
        self.dirty = true;
        Ok(true)
    }

    /// Re-evaluate all upstream-to-downstream dependencies in topological order.
    pub fn recalculate_all(&mut self) -> Result<(), EngineError> {
        guard::run_guarded("project.recalculate", || {
            let _ = dag::recalculate_project_sanitized(&mut self.doc)?;
            Ok(())
        })
    }

    /// Compile a graph or layout sheet into a resolution-independent [`ChartScene`].
    pub fn render_sheet_scene(&self, sheet_id: &SheetId) -> Result<ChartScene, EngineError> {
        cmd::helpers::compile_sheet_scene(&self.doc, sheet_id)
    }
}

#[cfg(test)]
mod tests {
    use lumencraft_doc::SheetId;
    use serde_json::json;

    use super::{EngineError, Session, cmd};

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
}
