//! Structured CLI error categories and POSIX/automation exit code mapping.

use std::fmt;

use lumencraft_chart::ChartError;
use lumencraft_doc::DocError;
use lumencraft_engine::EngineError;
use lumencraft_format::FormatError;
use lumencraft_pdf::PdfError;
use lumencraft_render::RenderError;
use lumencraft_svg::SvgError;

/// Exit code indicating successful execution.
pub const EXIT_OK: u8 = 0;

/// Exit code indicating a runtime, I/O, data, format, or analysis execution failure.
pub const EXIT_RUNTIME: u8 = 1;

/// Exit code indicating CLI usage error, invalid JSON/manifest schema, or `--workdir` path escape.
pub const EXIT_USAGE_OR_POLICY: u8 = 2;

/// Categorized CLI error carrying its deterministic process exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    /// Invalid CLI syntax, unknown flag, or wrong positional arity (exit code `2`).
    Usage(String),
    /// Invalid command stream or batch manifest schema (exit code `2`).
    Schema(String),
    /// Attempted path traversal or escape outside `--workdir` (exit code `2`).
    PathEscape(String),
    /// Runtime, filesystem, archive, solver, or rendering error (exit code `1`).
    Runtime(String),
}

impl CliError {
    /// Process exit code associated with this error variant (`1` or `2`).
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) | Self::Schema(_) | Self::PathEscape(_) => EXIT_USAGE_OR_POLICY,
            Self::Runtime(_) => EXIT_RUNTIME,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(msg) => write!(f, "usage error: {msg}"),
            Self::Schema(msg) => write!(f, "schema error: {msg}"),
            Self::PathEscape(msg) => write!(f, "path containment violation: {msg}"),
            Self::Runtime(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for CliError {}

impl From<std::io::Error> for CliError {
    fn from(err: std::io::Error) -> Self {
        Self::Runtime(format!("I/O error: {err}"))
    }
}

impl From<DocError> for CliError {
    fn from(err: DocError) -> Self {
        Self::Runtime(format!("document error: {err}"))
    }
}

impl From<FormatError> for CliError {
    fn from(err: FormatError) -> Self {
        Self::Runtime(format!("format error: {err}"))
    }
}

impl From<EngineError> for CliError {
    fn from(err: EngineError) -> Self {
        Self::Runtime(format!("engine error: {err}"))
    }
}

impl From<ChartError> for CliError {
    fn from(err: ChartError) -> Self {
        Self::Runtime(format!("chart compilation error: {err}"))
    }
}

impl From<SvgError> for CliError {
    fn from(err: SvgError) -> Self {
        Self::Runtime(format!("SVG render error: {err}"))
    }
}

impl From<PdfError> for CliError {
    fn from(err: PdfError) -> Self {
        Self::Runtime(format!("PDF render error: {err}"))
    }
}

impl From<RenderError> for CliError {
    fn from(err: RenderError) -> Self {
        Self::Runtime(format!("raster render error: {err}"))
    }
}
