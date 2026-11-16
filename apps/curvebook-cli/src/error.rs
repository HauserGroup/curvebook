//! Structured CLI error categories and POSIX/automation exit code mapping.

use std::fmt;

use curvebook_chart::ChartError;
use curvebook_doc::{DocError, ErrorCategory, ErrorCode};
use curvebook_engine::EngineError;
use curvebook_format::FormatError;
use curvebook_pdf::PdfError;
use curvebook_platform::PlatformError;
use curvebook_render::RenderError;
use curvebook_svg::SvgError;
use serde::Serialize;
use serde_json::Value;

/// Exit code indicating successful execution.
pub const EXIT_OK: u8 = 0;

/// Exit code indicating a runtime, I/O, data, format, or analysis execution failure.
pub const EXIT_RUNTIME: u8 = 1;

/// Exit code indicating CLI usage error, invalid JSON/manifest schema, or `--workdir` path escape.
pub const EXIT_USAGE_OR_POLICY: u8 = 2;

/// Exit code indicating operator or host cancellation/interruption (`SIGINT`-aligned).
pub const EXIT_INTERRUPTED: u8 = 130;

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
    /// Operation was cancelled or interrupted before completion (exit code `130`).
    Interrupted(String),
    /// Structured domain/engine error preserving machine-readable metadata.
    Engine(EngineError),
}

/// Machine-readable inner payload for `--json` structured CLI errors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StructuredCliError {
    /// Canonical `snake_case` error identifier.
    pub code: ErrorCode,
    /// High-level error category.
    pub category: ErrorCategory,
    /// Human-readable diagnostic message.
    pub message: String,
    /// Whether retrying the operation without input changes may succeed.
    pub retryable: bool,
    /// Optional actionable guidance for resolving the error.
    pub remediation: Option<String>,
    /// Optional structured diagnostic metadata.
    pub details: Option<Value>,
}

/// Top-level JSON envelope emitted on CLI failure when `--json` is active.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CliErrorEnvelope {
    /// Structured error payload.
    pub error: StructuredCliError,
}

impl CliError {
    /// Process exit code associated with this error variant (`1`, `2`, or `130`).
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) | Self::Schema(_) | Self::PathEscape(_) => EXIT_USAGE_OR_POLICY,
            Self::Runtime(_) => EXIT_RUNTIME,
            Self::Interrupted(_) => EXIT_INTERRUPTED,
            Self::Engine(err) => match err.code {
                ErrorCode::Cancelled => EXIT_INTERRUPTED,
                ErrorCode::PathEscape => EXIT_USAGE_OR_POLICY,
                _ => EXIT_RUNTIME,
            },
        }
    }

    /// Build the canonical structured error envelope `{"error": { ... }}` for `--json` output.
    #[must_use]
    pub fn to_json_envelope(&self) -> CliErrorEnvelope {
        let error = match self {
            Self::Usage(msg) => StructuredCliError {
                code: ErrorCode::BadInput,
                category: ErrorCategory::Input,
                message: format!("usage error: {msg}"),
                retryable: false,
                remediation: Some(String::from("Run `curvebook-cli --help` to inspect valid commands and flags.")),
                details: None,
            },
            Self::Schema(msg) => StructuredCliError {
                code: ErrorCode::BadInput,
                category: ErrorCategory::Input,
                message: format!("schema error: {msg}"),
                retryable: false,
                remediation: Some(String::from("Verify the JSON script or batch manifest structure.")),
                details: None,
            },
            Self::PathEscape(msg) => StructuredCliError {
                code: ErrorCode::PathEscape,
                category: ErrorCategory::Policy,
                message: format!("path containment violation: {msg}"),
                retryable: false,
                remediation: Some(String::from("Keep all file paths relative to and inside `--workdir`.")),
                details: None,
            },
            Self::Runtime(msg) => {
                let is_io = msg.starts_with("I/O error:")
                    || msg.starts_with("failed to read ")
                    || msg.starts_with("failed to write ")
                    || msg.starts_with("failed to create ");
                StructuredCliError {
                    code: if is_io { ErrorCode::IoError } else { ErrorCode::InternalError },
                    category: if is_io { ErrorCategory::Io } else { ErrorCategory::Internal },
                    message: msg.clone(),
                    retryable: is_io,
                    remediation: None,
                    details: None,
                }
            }
            Self::Interrupted(msg) => StructuredCliError {
                code: ErrorCode::Cancelled,
                category: ErrorCategory::Compute,
                message: msg.clone(),
                retryable: true,
                remediation: Some(String::from("Retry the interrupted command.")),
                details: None,
            },
            Self::Engine(err) => StructuredCliError {
                code: err.code,
                category: err.category,
                message: err.message().to_owned(),
                retryable: err.retryable,
                remediation: err.remediation.clone(),
                details: err.details.clone(),
            },
        };
        CliErrorEnvelope { error }
    }

    /// Serialize this error as a pretty-printed `--json` error envelope string.
    #[must_use]
    pub fn to_json_string(&self) -> String {
        serde_json::to_string_pretty(&self.to_json_envelope()).unwrap_or_else(|_| {
            String::from(
                r#"{"error":{"code":"internal_error","category":"internal","message":"failed to serialize CLI error","retryable":false,"remediation":null,"details":null}}"#,
            )
        })
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(msg) => write!(f, "usage error: {msg}"),
            Self::Schema(msg) => write!(f, "schema error: {msg}"),
            Self::PathEscape(msg) => write!(f, "path containment violation: {msg}"),
            Self::Runtime(msg) => write!(f, "{msg}"),
            Self::Interrupted(msg) => write!(f, "interrupted: {msg}"),
            Self::Engine(err) => write!(f, "engine error: {err}"),
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
        Self::from(EngineError::from(err))
    }
}

impl From<FormatError> for CliError {
    fn from(err: FormatError) -> Self {
        Self::from(EngineError::from(err))
    }
}

impl From<EngineError> for CliError {
    fn from(err: EngineError) -> Self {
        match err.code {
            ErrorCode::Cancelled => Self::Interrupted(err.message().to_owned()),
            ErrorCode::PathEscape => Self::PathEscape(err.message().to_owned()),
            _ => Self::Engine(err),
        }
    }
}

impl From<ChartError> for CliError {
    fn from(err: ChartError) -> Self {
        Self::from(EngineError::from(err))
    }
}

impl From<SvgError> for CliError {
    fn from(err: SvgError) -> Self {
        Self::from(EngineError::from(err))
    }
}

impl From<PdfError> for CliError {
    fn from(err: PdfError) -> Self {
        Self::from(EngineError::from(err))
    }
}

impl From<RenderError> for CliError {
    fn from(err: RenderError) -> Self {
        Self::from(EngineError::from(err))
    }
}

impl From<PlatformError> for CliError {
    fn from(err: PlatformError) -> Self {
        match err {
            PlatformError::PathEscape { .. } => Self::PathEscape(err.to_string()),
            PlatformError::InvalidPath { reason } => Self::Usage(reason),
            other => Self::from(EngineError::from(other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{ErrorCategory, ErrorCode};
    use curvebook_engine::EngineError;
    use serde_json::{Value, json};

    use super::{CliError, EXIT_INTERRUPTED, EXIT_OK, EXIT_RUNTIME, EXIT_USAGE_OR_POLICY};

    #[test]
    fn exit_codes_cover_ok_runtime_usage_and_interrupted() {
        assert_eq!(EXIT_OK, 0);
        assert_eq!(EXIT_RUNTIME, 1);
        assert_eq!(EXIT_USAGE_OR_POLICY, 2);
        assert_eq!(EXIT_INTERRUPTED, 130);

        assert_eq!(CliError::Runtime(String::from("boom")).exit_code(), 1);
        assert_eq!(CliError::Usage(String::from("bad flag")).exit_code(), 2);
        assert_eq!(CliError::Schema(String::from("bad json")).exit_code(), 2);
        assert_eq!(CliError::PathEscape(String::from("../out")).exit_code(), 2);
        assert_eq!(CliError::Interrupted(String::from("cancelled")).exit_code(), 130);
    }

    #[test]
    fn structured_json_error_contains_all_required_fields() {
        let engine_err = CliError::from(
            EngineError::with_code(ErrorCode::NotFound, "sheet `missing` not found")
                .with_remediation("Pass an existing sheet ID.")
                .with_details(json!({ "sheet_id": "missing" })),
        );
        assert_eq!(engine_err.exit_code(), EXIT_RUNTIME);
        let parsed: Value = serde_json::from_str(&engine_err.to_json_string()).unwrap();
        assert_eq!(parsed["error"]["code"], json!("not_found"));
        assert_eq!(parsed["error"]["category"], json!("state"));
        assert_eq!(parsed["error"]["message"], json!("sheet `missing` not found"));
        assert_eq!(parsed["error"]["retryable"], json!(false));
        assert_eq!(parsed["error"]["remediation"], json!("Pass an existing sheet ID."));
        assert_eq!(parsed["error"]["details"], json!({ "sheet_id": "missing" }));

        let interrupted = CliError::from(EngineError::with_code(ErrorCode::Cancelled, "aborted by caller").with_retryable(true));
        assert!(matches!(interrupted, CliError::Interrupted(_)));
        assert_eq!(interrupted.exit_code(), EXIT_INTERRUPTED);
        let int_json: Value = serde_json::from_str(&interrupted.to_json_string()).unwrap();
        assert_eq!(int_json["error"]["code"], json!(ErrorCode::Cancelled.as_str()));
        assert_eq!(int_json["error"]["category"], json!(ErrorCategory::Compute.as_str()));
        assert_eq!(int_json["error"]["retryable"], json!(true));
        assert!(int_json["error"].get("details").is_some());
    }
}
