//! Structured MCP error payloads aligned with `curvebook_engine::EngineError` diagnostics.

use curvebook_doc::{ErrorCategory, ErrorCode};
use curvebook_engine::EngineError;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::tools::{ContentBlock, ToolCallError, ToolCallResult};

/// Canonical structured error payload serialized inside MCP `isError: true` responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredErrorPayload {
    pub code: ErrorCode,
    pub category: ErrorCategory,
    pub message: String,
    pub retryable: bool,
    pub remediation: Option<String>,
    pub details: Option<Value>,
}

impl StructuredErrorPayload {
    /// Construct a structured error payload from an [`ErrorCode`] and human-readable message.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, category: code.category(), message: message.into(), retryable: false, remediation: None, details: None }
    }

    /// Convert an [`EngineError`] into its canonical MCP structured error representation.
    #[must_use]
    pub fn from_engine_error(err: &EngineError) -> Self {
        Self {
            code: err.code,
            category: err.category,
            message: err.message().to_owned(),
            retryable: err.retryable,
            remediation: err.remediation.clone(),
            details: err.details.clone(),
        }
    }

    /// Infer a structured error payload from a plain diagnostic string when structured metadata
    /// was not supplied explicitly.
    #[must_use]
    pub fn from_message(message: impl Into<String>) -> Self {
        let msg = message.into();
        let lower = msg.to_ascii_lowercase();
        let (code, retryable, remediation) = if lower.contains("failed to connect") || lower.contains("timed out") {
            (ErrorCode::IoError, true, Some(String::from("Ensure the target desktop control port or host resource is reachable and retry.")))
        } else if lower.contains("not found") {
            (ErrorCode::NotFound, false, None)
        } else if lower.contains("requires attaching") {
            (
                ErrorCode::Conflict,
                false,
                Some(String::from("Attach `--control-port <port>` to a live desktop session or use a headless engine command.")),
            )
        } else if lower.contains("path escape") || lower.contains("outside workdir") {
            (ErrorCode::PathEscape, false, Some(String::from("Supply a path that stays strictly inside the configured MCP workdir.")))
        } else {
            (ErrorCode::InternalError, false, None)
        };
        Self { code, category: code.category(), message: msg, retryable, remediation, details: None }
    }

    /// Set whether retrying the operation without modifying inputs may succeed.
    #[must_use]
    pub const fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    /// Attach an actionable remediation hint.
    #[must_use]
    pub fn with_remediation(mut self, remediation: impl Into<String>) -> Self {
        self.remediation = Some(remediation.into());
        self
    }

    /// Attach structured machine-readable diagnostic context.
    #[must_use]
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    /// Convert this payload into a JSON [`Value`] preserving all canonical keys.
    #[must_use]
    pub fn to_value(&self) -> Value {
        json!({
            "code": self.code,
            "category": self.category,
            "message": self.message,
            "retryable": self.retryable,
            "remediation": self.remediation,
            "details": self.details,
        })
    }
}

/// Convert an [`EngineError`] into an MCP [`ToolCallResult`] with `isError: true` and both JSON
/// text and `structuredContent`.
#[must_use]
pub fn result_from_engine_error(err: &EngineError) -> ToolCallResult {
    result_from_payload(&StructuredErrorPayload::from_engine_error(err))
}

/// Construct an MCP error [`ToolCallResult`] from an explicit [`ErrorCode`] and message.
#[must_use]
pub fn result_from_code(code: ErrorCode, message: impl Into<String>) -> ToolCallResult {
    result_from_payload(&StructuredErrorPayload::new(code, message))
}

/// Construct an MCP error [`ToolCallResult`] from a [`StructuredErrorPayload`].
#[must_use]
pub fn result_from_payload(payload: &StructuredErrorPayload) -> ToolCallResult {
    result_from_payload_with_extra(payload, Map::new())
}

/// Construct an MCP error [`ToolCallResult`] merging batch/context keys alongside the canonical
/// structured error fields (`code`, `category`, `message`, `retryable`, `remediation`, `details`).
#[must_use]
pub fn result_from_payload_with_extra(payload: &StructuredErrorPayload, extra: Map<String, Value>) -> ToolCallResult {
    let mut val = payload.to_value();
    if let Value::Object(ref mut obj) = val {
        for (k, v) in extra {
            obj.insert(k, v);
        }
    }
    let text = serde_json::to_string_pretty(&val).unwrap_or_else(|_| payload.message.clone());
    ToolCallResult { content: vec![ContentBlock::Text { text }], is_error: true, structured_content: Some(val) }
}

/// Convert a plain execution failure string into a structured [`ToolCallResult`] unless it already
/// encodes a complete JSON structured error object.
#[must_use]
pub fn result_from_error_message(message: String) -> ToolCallResult {
    if let Ok(parsed) = serde_json::from_str::<Value>(&message)
        && parsed.get("code").is_some()
        && parsed.get("category").is_some()
        && parsed.get("message").is_some()
    {
        return ToolCallResult { content: vec![ContentBlock::Text { text: message }], is_error: true, structured_content: Some(parsed) };
    }
    result_from_payload(&StructuredErrorPayload::from_message(message))
}

/// Convert a [`ToolCallError`] execution failure into `Ok(ToolCallResult)` when surfacing domain errors.
pub fn execution_err_to_result(err: ToolCallError) -> Result<ToolCallResult, ToolCallError> {
    match err {
        ToolCallError::Execution(msg) => Ok(result_from_error_message(msg)),
        other => Err(other),
    }
}
