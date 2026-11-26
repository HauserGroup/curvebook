//! JSON-RPC 2.0 transport and MCP message framing over arbitrary `BufRead + Write` streams.
//!
//! Supports both LSP-style `Content-Length: N\r\n\r\n` framing and newline-delimited JSON-RPC,
//! enforcing strict size caps and zero-panic error reporting on malformed or adversarial inputs.

use std::io::{self, BufRead, BufReader, BufWriter, Write};

#[cfg(not(target_arch = "wasm32"))]
use curvebook_doc::ErrorCode;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::SERVER_NAME;
use crate::errors::{self, StructuredErrorPayload};
use crate::prompts::{get_prompt, list_prompts};
#[cfg(not(target_arch = "wasm32"))]
use crate::resources::{ResourceContent, ResourceDescriptor};
use crate::resources::{ResourceError, list_resource_templates, list_resources, read_resource};
#[cfg(not(target_arch = "wasm32"))]
use crate::tools::decode_base64;
use crate::tools::{McpServerState, ToolCallError, call_tool, list_tools_for_state};

/// MCP protocol revision advertised by this server during `initialize`.
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Guidance string advertised to MCP clients during `initialize`.
pub const SERVER_INSTRUCTIONS: &str = concat!(
    "CurveBook MCP server for scientific tables, statistical analysis, curve fitting, and publication figures. ",
    "Inspect state with `doc_inspect` (supports `detail: 'compact'|'standard'|'full'`, `sections`, `sheet_id`, `tab_id`, `max_rows`, `max_scalars`) or resources (`sheet://<id>`, `result://<id>?include_curves=false`). ",
    "Discover commands with `command_list` (`filter`, `group`, `command_id`, `include_schemas`) and `list_toolsets` / `enable_toolset` (emits `notifications/tools/list_changed`). ",
    "Use `fit.run` `method: 'rout'` + `identify_unstable_parameters: true` for dose-response outlier detection and identifiability diagnostics, inspect tool `annotations` before mutating/filesystem calls, and pass string arguments to `prompts/list` + `prompts/get`. ",
    "Reference docs: https://curvebook.org/manual/v",
    env!("CARGO_PKG_VERSION_MAJOR"),
    ".",
    env!("CARGO_PKG_VERSION_MINOR"),
    "/automation/mcp.html",
);

/// Newer MCP protocol revision accepted when explicitly requested by a client.
const SUPPORTED_PROTOCOL_VERSIONS: [&str; 2] = ["2024-11-05", "2025-03-26"];

/// Maximum JSON-RPC payload size accepted by default (16 MiB).
pub const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;

/// Maximum oversized frame bytes safely drained before closing the transport (64 MiB).
pub const MAX_DRAIN_BYTES: usize = 64 * 1024 * 1024;

/// Maximum leading payload bytes inspected for best-effort JSON-RPC `"id"` extraction
/// when draining an oversized frame.
const ID_SNIFF_BYTES: usize = 16 * 1024;

/// Maximum bytes permitted in a single framing header line (8 KiB).
pub const MAX_HEADER_LINE_BYTES: usize = 8 * 1024;

/// Maximum number of header lines permitted in a single `Content-Length` frame.
pub const MAX_HEADER_COUNT: usize = 32;

/// Maximum elements allowed inside a single JSON-RPC batch request array.
pub const MAX_BATCH_LEN: usize = 256;

/// Bounded chunk size used when reading `Content-Length` bodies without pre-allocating huge buffers.
const READ_CHUNK_BYTES: usize = 8 * 1024;

/// JSON-RPC 2.0 Parse error code.
pub const ERR_PARSE_ERROR: i64 = -32700;

/// JSON-RPC 2.0 Invalid Request error code.
pub const ERR_INVALID_REQUEST: i64 = -32600;

/// JSON-RPC 2.0 Method not found error code.
pub const ERR_METHOD_NOT_FOUND: i64 = -32601;

/// JSON-RPC 2.0 Invalid params error code.
pub const ERR_INVALID_PARAMS: i64 = -32602;

/// JSON-RPC 2.0 Internal error code.
pub const ERR_INTERNAL_ERROR: i64 = -32603;

/// MCP Resource not found error code.
pub const ERR_RESOURCE_NOT_FOUND: i64 = -32002;

/// Wire framing mode detected for an inbound JSON-RPC message and mirrored on outbound replies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FramingMode {
    /// LSP / stdio MCP `Content-Length: N\r\n\r\n` header framing.
    #[default]
    ContentLength,
    /// Line-delimited JSON-RPC (`<json>\n`).
    Newline,
}

/// JSON-RPC 2.0 error payload object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Outbound JSON-RPC 2.0 response envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    /// Construct a successful JSON-RPC 2.0 response.
    #[must_use]
    pub fn ok(id: Value, result: Value) -> Self {
        Self { jsonrpc: "2.0", id, result: Some(result), error: None }
    }

    /// Construct an error JSON-RPC 2.0 response.
    #[must_use]
    pub fn err(id: Value, code: i64, message: impl Into<String>, data: Option<Value>) -> Self {
        Self { jsonrpc: "2.0", id, result: None, error: Some(JsonRpcError { code, message: message.into(), data }) }
    }
}

enum ReadLineOutcome {
    Line(Vec<u8>),
    Eof(Vec<u8>),
    TooLong { id: Value, looks_like_header: bool },
    DrainLimitExceeded,
}

enum NextFrame {
    Payload { bytes: Vec<u8>, mode: FramingMode },
    RecoverableError { id: Value, mode: FramingMode, code: i64, message: String },
    FatalError { id: Value, mode: FramingMode, code: i64, message: String },
    Eof,
}

/// Run the MCP server on process stdin/stdout using a fresh headless `Session`.
pub fn run_stdio() -> io::Result<()> {
    let mut state = McpServerState::new();
    run_stdio_with_state(&mut state)
}

/// Run the MCP server on process stdin/stdout driven by an explicit [`McpServerState`].
pub fn run_stdio_with_state(state: &mut McpServerState) -> io::Result<()> {
    let stdout = io::stdout();
    let mut writer = BufWriter::new(stdout.lock());
    #[cfg(not(target_arch = "wasm32"))]
    {
        let reader = BufReader::new(io::stdin());
        serve_concurrent(reader, &mut writer, state, MAX_REQUEST_BYTES)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let stdin = io::stdin();
        let mut reader = BufReader::new(stdin.lock());
        serve(&mut reader, &mut writer, state)
    }
}

/// Serve MCP JSON-RPC 2.0 requests using a dedicated reader thread so `notifications/cancelled`
/// frames arriving while a compute call is in flight immediately signal the active request's
/// cooperative `AtomicBool` cancel flag (`AUD-13-22`).
#[cfg(not(target_arch = "wasm32"))]
pub fn serve_concurrent<R: BufRead + Send + 'static, W: Write>(
    mut reader: R,
    writer: &mut W,
    state: &mut McpServerState,
    max_bytes: usize,
) -> io::Result<()> {
    let effective_max = max_bytes.clamp(1, MAX_REQUEST_BYTES);
    let cancel_registry = state.cancellation_registry();
    let (tx, rx) = std::sync::mpsc::channel::<io::Result<NextFrame>>();

    let _reader_thread = std::thread::Builder::new().name(String::from("mcp-stdio-reader")).spawn(move || {
        let mut last_mode = FramingMode::Newline;
        loop {
            match read_next_frame(&mut reader, effective_max, last_mode) {
                Ok(NextFrame::Eof) => {
                    let _ = tx.send(Ok(NextFrame::Eof));
                    break;
                }
                Ok(NextFrame::Payload { bytes, mode }) => {
                    last_mode = mode;
                    if intercept_cancellation_frame(&bytes, &cancel_registry) {
                        continue;
                    }
                    if tx.send(Ok(NextFrame::Payload { bytes, mode })).is_err() {
                        break;
                    }
                }
                Ok(frame @ NextFrame::RecoverableError { mode, .. }) => {
                    last_mode = mode;
                    if tx.send(Ok(frame)).is_err() {
                        break;
                    }
                }
                Ok(frame @ NextFrame::FatalError { .. }) => {
                    let _ = tx.send(Ok(frame));
                    break;
                }
                Err(err) => {
                    let _ = tx.send(Err(err));
                    break;
                }
            }
        }
    })?;

    while let Ok(frame_res) = rx.recv() {
        match frame_res? {
            NextFrame::Eof => return Ok(()),
            NextFrame::Payload { bytes, mode } => {
                if let Some(reply_value) = dispatch_payload(&bytes, state) {
                    write_json_value(writer, &reply_value, mode)?;
                }
                for notif in state.take_pending_notifications() {
                    write_json_value(writer, &notif, mode)?;
                }
            }
            NextFrame::RecoverableError { id, mode, code, message } => {
                let resp = JsonRpcResponse::err(id, code, message, None);
                write_response(writer, &resp, mode)?;
            }
            NextFrame::FatalError { id, mode, code, message } => {
                let resp = JsonRpcResponse::err(id, code, message, None);
                write_response(writer, &resp, mode)?;
                return Ok(());
            }
        }
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn intercept_cancellation_frame(bytes: &[u8], cancel_registry: &crate::tools::CancellationRegistry) -> bool {
    let Ok(val) = serde_json::from_slice::<Value>(bytes) else {
        return false;
    };
    match val {
        Value::Object(ref obj) => {
            if obj.get("id").is_none()
                && obj.get("method").and_then(Value::as_str) == Some("notifications/cancelled")
                && let Some(params) = obj.get("params")
            {
                if let Some(target_id) = extract_cancelled_request_id(params) {
                    let _ = cancel_registry.cancel(target_id);
                }
                return true;
            }
            false
        }
        Value::Array(ref items) => {
            for item in items {
                if let Some(obj) = item.as_object()
                    && obj.get("id").is_none()
                    && obj.get("method").and_then(Value::as_str) == Some("notifications/cancelled")
                    && let Some(params) = obj.get("params")
                    && let Some(target_id) = extract_cancelled_request_id(params)
                {
                    let _ = cancel_registry.cancel(target_id);
                }
            }
            false
        }
        _ => false,
    }
}

/// Run the MCP server on process stdin/stdout attached to a live desktop `--control <port>` loop.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_stdio_with_control_port(port: u16) -> Result<(), String> {
    run_stdio_with_control_port_options(port, None)
}

/// Run the MCP server on process stdin/stdout attached to a live desktop `--control <port>` loop
/// with an optional `--control-token`.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_stdio_with_control_port_options(port: u16, control_token: Option<&str>) -> Result<(), String> {
    let mut state = McpServerState::with_control_port(port)?;
    if let Some(token) = control_token {
        state.set_control_token(Some(token))?;
    }
    run_stdio_with_state(&mut state).map_err(|err| format!("stdio transport error: {err}"))
}

/// Serve MCP JSON-RPC 2.0 requests until EOF using [`MAX_REQUEST_BYTES`].
pub fn serve<R: BufRead, W: Write>(reader: &mut R, writer: &mut W, state: &mut McpServerState) -> io::Result<()> {
    serve_with_max_bytes(reader, writer, state, MAX_REQUEST_BYTES)
}

/// Serve MCP JSON-RPC 2.0 requests until EOF with an explicit per-request byte ceiling
/// clamped to `1..=MAX_REQUEST_BYTES`.
pub fn serve_with_max_bytes<R: BufRead, W: Write>(reader: &mut R, writer: &mut W, state: &mut McpServerState, max_bytes: usize) -> io::Result<()> {
    let mut last_mode = FramingMode::Newline;
    let effective_max = max_bytes.clamp(1, MAX_REQUEST_BYTES);
    loop {
        match read_next_frame(reader, effective_max, last_mode)? {
            NextFrame::Eof => return Ok(()),
            NextFrame::Payload { bytes, mode } => {
                last_mode = mode;
                if let Some(reply_value) = dispatch_payload(&bytes, state) {
                    write_json_value(writer, &reply_value, mode)?;
                }
                for notif in state.take_pending_notifications() {
                    write_json_value(writer, &notif, mode)?;
                }
            }
            NextFrame::RecoverableError { id, mode, code, message } => {
                last_mode = mode;
                let resp = JsonRpcResponse::err(id, code, message, None);
                write_response(writer, &resp, mode)?;
            }
            NextFrame::FatalError { id, mode, code, message } => {
                let resp = JsonRpcResponse::err(id, code, message, None);
                write_response(writer, &resp, mode)?;
                return Ok(());
            }
        }
    }
}

/// Dispatch a single raw JSON-RPC payload slice against `state`, returning `Some(Value)` when a
/// response is required or `None` when all processed items were notifications.
#[must_use]
pub fn dispatch_payload(payload: &[u8], state: &mut McpServerState) -> Option<Value> {
    let value: Value = match serde_json::from_slice(payload) {
        Ok(v) => v,
        Err(err) => {
            let resp = JsonRpcResponse::err(Value::Null, ERR_PARSE_ERROR, format!("parse error: {err}"), None);
            return serde_json::to_value(resp).ok();
        }
    };
    dispatch_json_value(value, state)
}

/// Dispatch an already-parsed JSON `Value` (single object or batch array) against `state`.
#[must_use]
pub fn dispatch_json_value(value: Value, state: &mut McpServerState) -> Option<Value> {
    match value {
        Value::Object(obj) => dispatch_single(&obj, state).and_then(|resp| serde_json::to_value(resp).ok()),
        Value::Array(items) => dispatch_batch(&items, state),
        _ => {
            let resp = JsonRpcResponse::err(Value::Null, ERR_INVALID_REQUEST, "invalid JSON-RPC request", None);
            serde_json::to_value(resp).ok()
        }
    }
}

fn dispatch_batch(items: &[Value], state: &mut McpServerState) -> Option<Value> {
    if items.is_empty() {
        let resp = JsonRpcResponse::err(Value::Null, ERR_INVALID_REQUEST, "empty JSON-RPC batch", None);
        return serde_json::to_value(resp).ok();
    }
    if items.len() > MAX_BATCH_LEN {
        let resp =
            JsonRpcResponse::err(Value::Null, ERR_INVALID_REQUEST, format!("batch length {} exceeds maximum {MAX_BATCH_LEN}", items.len()), None);
        return serde_json::to_value(resp).ok();
    }

    let mut replies = Vec::new();
    for item in items {
        let reply = match item {
            Value::Object(obj) => dispatch_single(obj, state),
            _ => Some(JsonRpcResponse::err(Value::Null, ERR_INVALID_REQUEST, "invalid batch request element", None)),
        };
        if let Some(r) = reply {
            replies.push(r);
        }
    }

    if replies.is_empty() { None } else { serde_json::to_value(replies).ok() }
}

fn dispatch_single(obj: &Map<String, Value>, state: &mut McpServerState) -> Option<JsonRpcResponse> {
    let Ok(req_id) = extract_request_id(obj) else {
        return Some(JsonRpcResponse::err(Value::Null, ERR_INVALID_REQUEST, "\"id\" must be string, number, or null", None));
    };
    let reply_id = req_id.clone().unwrap_or(Value::Null);

    if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(JsonRpcResponse::err(reply_id, ERR_INVALID_REQUEST, "missing or invalid \"jsonrpc\": \"2.0\" field", None));
    }

    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        return Some(JsonRpcResponse::err(reply_id, ERR_INVALID_REQUEST, "missing or non-string \"method\" field", None));
    };

    let params = match obj.get("params") {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(v @ (Value::Object(_) | Value::Array(_))) => v.clone(),
        Some(_) => {
            return Some(JsonRpcResponse::err(reply_id, ERR_INVALID_REQUEST, "\"params\" must be an object or array when present", None));
        }
    };

    dispatch_method(req_id, method, params, state)
}

fn extract_request_id(obj: &Map<String, Value>) -> Result<Option<Value>, ()> {
    let Some(raw) = obj.get("id") else {
        return Ok(None);
    };
    match raw {
        Value::Null | Value::String(_) | Value::Number(_) => Ok(Some(raw.clone())),
        Value::Bool(_) | Value::Array(_) | Value::Object(_) => Err(()),
    }
}

fn dispatch_method(req_id: Option<Value>, method: &str, params: Value, state: &mut McpServerState) -> Option<JsonRpcResponse> {
    if is_notification_method(method, req_id.is_none()) {
        // `handle_initialize` marks the session initialized; ignore `notifications/initialized`
        // (and legacy `initialized`) prior to `initialize` so a premature notification neither
        // bypasses `require_initialize` nor blocks the subsequent `initialize` handshake.
        if method == "notifications/cancelled"
            && let Some(target_id) = extract_cancelled_request_id(&params)
        {
            let _ = state.cancel_request(target_id);
        }
        return req_id.map(|id| JsonRpcResponse::ok(id, json!({})));
    }

    let id = req_id?;
    if method == "ping" {
        return Some(JsonRpcResponse::ok(id, json!({})));
    }
    if state.is_shutting_down() {
        return Some(JsonRpcResponse::err(id, ERR_INVALID_REQUEST, "server is shutting down", None));
    }
    if method == "initialize" {
        if state.is_initialized() {
            return Some(JsonRpcResponse::err(id, ERR_INVALID_REQUEST, "session is already initialized", None));
        }
        return Some(handle_initialize(id, &params, state));
    }
    if method == "shutdown" {
        state.mark_shutting_down();
        return Some(JsonRpcResponse::ok(id, json!({})));
    }
    if state.requires_initialize() && !state.is_initialized() {
        return Some(JsonRpcResponse::err(id, ERR_INVALID_REQUEST, "server is not initialized; call initialize first", None));
    }

    let (cancel_token, _active_guard) = state.cancellation_registry().begin_request(&id);
    let prev_token = state.set_active_cancel_token(Some(cancel_token));
    let response = match method {
        "tools/list" => handle_tools_list(id, &params, state),
        "tools/call" => handle_tools_call(id, params, state),
        "resources/list" => handle_resources_list(id, &params, state),
        "resources/templates/list" => handle_resources_templates_list(id, &params),
        "resources/read" => handle_resources_read(id, &params, state),
        "prompts/list" => handle_prompts_list(id, &params),
        "prompts/get" => handle_prompts_get(id, &params),
        _ => JsonRpcResponse::err(id, ERR_METHOD_NOT_FOUND, format!("method not found: {method}"), None),
    };
    let _ = state.set_active_cancel_token(prev_token);
    Some(response)
}

fn extract_cancelled_request_id(params: &Value) -> Option<&Value> {
    let map = params.as_object()?;
    let candidate = map.get("requestId").or_else(|| map.get("id"))?;
    match candidate {
        Value::String(s) if !s.is_empty() => Some(candidate),
        Value::Number(_) => Some(candidate),
        _ => None,
    }
}

fn is_notification_method(method: &str, is_notification: bool) -> bool {
    is_notification || matches!(method, "notifications/initialized" | "initialized" | "notifications/cancelled")
}

fn rpc_error_from_payload(id: Value, payload: StructuredErrorPayload) -> JsonRpcResponse {
    JsonRpcResponse::err(id, ERR_INTERNAL_ERROR, payload.message.clone(), Some(payload.to_value()))
}

fn run_guarded_rpc<F>(id: Value, op: &str, f: F) -> JsonRpcResponse
where
    F: FnOnce(&Value) -> Result<JsonRpcResponse, curvebook_engine::EngineError>,
{
    match curvebook_engine::guard::run_guarded(op, || f(&id)) {
        Ok(resp) => resp,
        Err(err) => rpc_error_from_payload(id, StructuredErrorPayload::from_engine_error(&err)),
    }
}

fn handle_initialize(id: Value, params: &Value, state: &mut McpServerState) -> JsonRpcResponse {
    if !params.is_object() {
        return JsonRpcResponse::err(id, ERR_INVALID_PARAMS, "initialize params must be an object", None);
    }
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let negotiated = negotiate_protocol_version(requested);
    state.mark_initialized();
    JsonRpcResponse::ok(
        id,
        json!({
            "protocolVersion": negotiated,
            "capabilities": {
                "tools": {
                    "listChanged": true
                },
                "resources": {
                    "subscribe": false,
                    "listChanged": false
                },
                "prompts": {
                    "listChanged": false
                }
            },
            "serverInfo": {
                "name": SERVER_NAME,
                "version": env!("CARGO_PKG_VERSION")
            },
            "instructions": SERVER_INSTRUCTIONS
        }),
    )
}

fn negotiate_protocol_version(requested: Option<&str>) -> &'static str {
    if let Some(req) = requested {
        for candidate in SUPPORTED_PROTOCOL_VERSIONS {
            if req == candidate {
                return candidate;
            }
        }
    }
    MCP_PROTOCOL_VERSION
}

fn handle_tools_list(id: Value, params: &Value, state: &mut McpServerState) -> JsonRpcResponse {
    if !params.is_object() {
        return JsonRpcResponse::err(id, ERR_INVALID_PARAMS, "tools/list params must be an object", None);
    }
    JsonRpcResponse::ok(id, json!({ "tools": list_tools_for_state(state) }))
}

fn handle_tools_call(id: Value, params: Value, state: &mut McpServerState) -> JsonRpcResponse {
    let Some(map) = params.as_object() else {
        return JsonRpcResponse::err(id, ERR_INVALID_PARAMS, "tools/call params must be an object", None);
    };
    let Some(name) = map.get("name").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) else {
        return JsonRpcResponse::err(id, ERR_INVALID_PARAMS, "tools/call requires a non-empty string \"name\"", None);
    };
    let arguments = match map.get("arguments") {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(v) if v.is_object() => v.clone(),
        Some(_) => {
            return JsonRpcResponse::err(id, ERR_INVALID_PARAMS, "tools/call \"arguments\" must be an object", None);
        }
    };

    match call_tool(state, name, arguments) {
        Ok(result) => match serde_json::to_value(result) {
            Ok(val) => JsonRpcResponse::ok(id, val),
            Err(err) => JsonRpcResponse::err(id, ERR_INTERNAL_ERROR, format!("failed to encode tool result: {err}"), None),
        },
        Err(ToolCallError::UnknownTool(unknown)) => {
            let hint = if let Some(spec) = crate::toolsets::find_command_spec_by_tool_name(&unknown) {
                format!(
                    "unknown tool: {unknown} (belongs to disabled `{}` toolset; call `enable_toolset` with `{{\"group\":\"{}\"}}` or run `command_run` with `command_id: \"{}\"`)",
                    spec.group.as_str(),
                    spec.group.as_str(),
                    spec.id,
                )
            } else {
                format!("unknown tool: {unknown} (check `tools/list` or call `list_toolsets` / `command_list`)")
            };
            JsonRpcResponse::err(id, ERR_INVALID_PARAMS, hint, None)
        }
        Err(ToolCallError::InvalidParams(msg)) => JsonRpcResponse::err(id, ERR_INVALID_PARAMS, msg, None),
        Err(ToolCallError::Execution(msg)) => {
            let err_result = errors::result_from_error_message(msg);
            match serde_json::to_value(err_result) {
                Ok(val) => JsonRpcResponse::ok(id, val),
                Err(err) => JsonRpcResponse::err(id, ERR_INTERNAL_ERROR, format!("failed to encode tool error: {err}"), None),
            }
        }
    }
}

fn handle_resources_list(id: Value, params: &Value, state: &mut McpServerState) -> JsonRpcResponse {
    if !params.is_object() {
        return JsonRpcResponse::err(id, ERR_INVALID_PARAMS, "resources/list params must be an object", None);
    }
    run_guarded_rpc(id, "mcp.resources_list", |req_id| {
        #[cfg(not(target_arch = "wasm32"))]
        if state.control_port().is_some() {
            return Ok(match list_live_resources(state) {
                Ok(resources) => JsonRpcResponse::ok(req_id.clone(), json!({ "resources": resources })),
                Err(payload) => rpc_error_from_payload(req_id.clone(), payload),
            });
        }
        Ok(JsonRpcResponse::ok(req_id.clone(), json!({ "resources": list_resources(state.session().document()) })))
    })
}

fn handle_resources_templates_list(id: Value, params: &Value) -> JsonRpcResponse {
    if !params.is_object() {
        return JsonRpcResponse::err(id, ERR_INVALID_PARAMS, "resources/templates/list params must be an object", None);
    }
    JsonRpcResponse::ok(id, json!({ "resourceTemplates": list_resource_templates() }))
}

fn handle_resources_read(id: Value, params: &Value, state: &mut McpServerState) -> JsonRpcResponse {
    let Some(map) = params.as_object() else {
        return JsonRpcResponse::err(id, ERR_INVALID_PARAMS, "resources/read params must be an object", None);
    };
    let Some(uri) = map.get("uri").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) else {
        return JsonRpcResponse::err(id, ERR_INVALID_PARAMS, "resources/read requires a non-empty string \"uri\"", None);
    };
    run_guarded_rpc(id, "mcp.resources_read", |req_id| {
        #[cfg(not(target_arch = "wasm32"))]
        if state.control_port().is_some() {
            return Ok(match read_live_resource(state, uri) {
                Ok(contents) => JsonRpcResponse::ok(req_id.clone(), json!({ "contents": contents })),
                Err(ResourceError::InvalidUri(msg)) => JsonRpcResponse::err(req_id.clone(), ERR_INVALID_PARAMS, msg, None),
                Err(ResourceError::NotFound(msg)) => JsonRpcResponse::err(req_id.clone(), ERR_RESOURCE_NOT_FOUND, msg, Some(json!({ "uri": uri }))),
                Err(ResourceError::Internal(msg)) => JsonRpcResponse::err(req_id.clone(), ERR_INTERNAL_ERROR, msg, None),
            });
        }

        Ok(match read_resource(state.session().document(), uri) {
            Ok(contents) => JsonRpcResponse::ok(req_id.clone(), json!({ "contents": contents })),
            Err(ResourceError::InvalidUri(msg)) => JsonRpcResponse::err(req_id.clone(), ERR_INVALID_PARAMS, msg, None),
            Err(ResourceError::NotFound(msg)) => JsonRpcResponse::err(req_id.clone(), ERR_RESOURCE_NOT_FOUND, msg, Some(json!({ "uri": uri }))),
            Err(ResourceError::Internal(msg)) => JsonRpcResponse::err(req_id.clone(), ERR_INTERNAL_ERROR, msg, None),
        })
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn list_live_resources(state: &mut McpServerState) -> Result<Vec<ResourceDescriptor>, StructuredErrorPayload> {
    fn json_array<'a>(v: &'a Value, key: &str) -> &'a [Value] {
        v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
    }
    fn json_str<'a>(v: &'a Value, key: &str, fallback: &'a str) -> &'a str {
        v.get(key).and_then(Value::as_str).unwrap_or(fallback)
    }
    fn json_u64(v: &Value, key: &str) -> u64 {
        v.get(key).and_then(Value::as_u64).unwrap_or(0)
    }

    let snap = state.send_live_control("doc.inspect", json!({}))?;
    let mut out = Vec::new();

    for ds in json_array(&snap, "data_sheets") {
        let id = json_str(ds, "id", "");
        let title = json_str(ds, "title", id);
        if !id.is_empty() {
            out.push(ResourceDescriptor {
                uri: format!("sheet://{id}"),
                name: title.to_owned(),
                description: format!("{title} (DataSheet: {}, {}x{})", json_str(ds, "kind", "data"), json_u64(ds, "rows"), json_u64(ds, "cols")),
                mime_type: String::from("application/json"),
            });
        }
    }
    for info in json_array(&snap, "info_sheets") {
        let id = json_str(info, "id", "");
        let title = json_str(info, "title", id);
        if !id.is_empty() {
            out.push(ResourceDescriptor {
                uri: format!("sheet://{id}"),
                name: title.to_owned(),
                description: format!("{title} (InfoSheet, {} constant(s))", json_u64(info, "constants")),
                mime_type: String::from("application/json"),
            });
        }
    }
    for rs in json_array(&snap, "result_sheets") {
        let id = json_str(rs, "id", "");
        let title = json_str(rs, "title", id);
        if !id.is_empty() {
            out.push(ResourceDescriptor {
                uri: format!("sheet://{id}"),
                name: title.to_owned(),
                description: format!("{title} (ResultSheet, {} tab(s))", json_u64(rs, "tabs")),
                mime_type: String::from("application/json"),
            });
            out.push(ResourceDescriptor {
                uri: format!("result://{id}"),
                name: format!("{title} JSON Export"),
                description: format!("{title} (ResultSheet JSON export)"),
                mime_type: String::from("application/json"),
            });
        }
    }
    for gs in json_array(&snap, "graph_sheets") {
        let id = json_str(gs, "id", "");
        let title = json_str(gs, "title", id);
        if !id.is_empty() {
            out.push(ResourceDescriptor {
                uri: format!("sheet://{id}"),
                name: title.to_owned(),
                description: format!("{title} (GraphSheet, {})", json_str(gs, "kind", "Graph")),
                mime_type: String::from("application/json"),
            });
        }
    }
    for ls in json_array(&snap, "layout_sheets") {
        let id = json_str(ls, "id", "");
        let title = json_str(ls, "title", id);
        if !id.is_empty() {
            out.push(ResourceDescriptor {
                uri: format!("sheet://{id}"),
                name: title.to_owned(),
                description: format!("{title} (LayoutSheet, {} item(s))", json_u64(ls, "items")),
                mime_type: String::from("application/json"),
            });
        }
    }
    Ok(out)
}

#[cfg(not(target_arch = "wasm32"))]
fn summarize_live_fitted_curve_value(curve: &Value) -> Value {
    let Some(obj) = curve.as_object() else {
        return curve.clone();
    };
    let points_count = obj.get("points").and_then(Value::as_array).map_or(0, Vec::len);
    json!({
        "data_set_index": obj.get("data_set_index").cloned().unwrap_or(Value::Null),
        "equation_id": obj.get("equation_id").cloned().unwrap_or(Value::Null),
        "converged": obj.get("converged").cloned().unwrap_or(Value::Bool(false)),
        "ambiguous": obj.get("ambiguous").cloned().unwrap_or(Value::Bool(false)),
        "points_count": points_count,
        "points_omitted": true,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn apply_live_resource_query_to_json(val: &mut Value, id_str: &str, query: &crate::resources::ResourceQuery) -> Result<(), ResourceError> {
    let Some(obj) = val.as_object_mut() else {
        return Ok(());
    };
    let include_curves = query.include_curves.unwrap_or(!query.detail_summary);
    if !include_curves && let Some(Value::Array(curves)) = obj.get_mut("fitted_curves") {
        for curve in curves.iter_mut() {
            *curve = summarize_live_fitted_curve_value(curve);
        }
    }

    if let Some(row_cap) = query.max_rows
        && let Some(csv) = obj.get("csv_preview").and_then(Value::as_str).map(str::to_owned)
    {
        let mut lines = csv.lines();
        if let Some(header) = lines.next() {
            let mut sliced = String::from(header);
            let mut kept = 0usize;
            let mut truncated = false;
            for line in lines {
                if kept < row_cap {
                    sliced.push('\n');
                    sliced.push_str(line);
                    kept += 1;
                } else {
                    truncated = true;
                    break;
                }
            }
            obj.insert(String::from("csv_preview"), Value::String(sliced));
            if truncated {
                obj.insert(String::from("csv_preview_truncated"), Value::Bool(true));
            }
        }
    }

    let tabs_key = if obj.get("tabs_data").is_some_and(Value::is_array) {
        Some("tabs_data")
    } else if obj.get("tabs").is_some_and(Value::is_array) {
        Some("tabs")
    } else {
        None
    };

    if let Some(target_tab) = query.tab_id.as_deref() {
        let available: Vec<String> = tabs_key
            .and_then(|k| obj.get(k))
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(|t| t.get("id").and_then(Value::as_str).or_else(|| t.as_str()).map(str::to_owned)).collect())
            .unwrap_or_default();
        if !available.iter().any(|id| id == target_tab) {
            return Err(ResourceError::NotFound(format!(
                "tab_id '{target_tab}' not found on '{id_str}'; available tab_ids: {}",
                available.join(", ")
            )));
        }
        if let Some(k) = tabs_key
            && let Some(Value::Array(arr)) = obj.get_mut(k)
        {
            arr.retain(|t| t.get("id").and_then(Value::as_str).or_else(|| t.as_str()) == Some(target_tab));
        }
    }

    if let Some(row_cap) = query.max_rows
        && let Some(k) = tabs_key
        && let Some(Value::Array(arr)) = obj.get_mut(k)
    {
        let mut any_truncated = false;
        for tab in arr.iter_mut() {
            if let Some(tab_obj) = tab.as_object_mut() {
                if let Some(Value::Array(rows)) = tab_obj.get_mut("rows")
                    && rows.len() > row_cap
                {
                    rows.truncate(row_cap);
                    any_truncated = true;
                }
                if let Some(Value::Array(labels)) = tab_obj.get_mut("row_labels")
                    && labels.len() > row_cap
                {
                    labels.truncate(row_cap);
                }
            }
        }
        if any_truncated {
            obj.insert(String::from("tabs_truncated"), Value::Bool(true));
        }
    }

    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn read_live_resource(state: &mut McpServerState, uri: &str) -> Result<Vec<ResourceContent>, ResourceError> {
    let trimmed = uri.trim();
    if let Some(sheet_raw) = trimmed.strip_prefix("sheet://") {
        let (id_part, query) = crate::resources::split_resource_query(sheet_raw)?;
        let id_str = id_part.trim();
        if id_str.is_empty() {
            return Err(ResourceError::InvalidUri(String::from("sheet:// URI requires a non-empty sheet id")));
        }
        let mut val = state
            .send_live_control("doc.inspect", json!({ "sheet_id": id_str, "include_cells": true }))
            .map_err(|err| map_live_resource_error(uri, &err))?;
        apply_live_resource_query_to_json(&mut val, id_str, &query)?;
        if let Some(obj) = val.as_object_mut() {
            obj.insert(String::from("uri"), Value::String(trimmed.to_owned()));
            if let Some(sec) = obj.get("section").and_then(Value::as_str) {
                obj.insert(String::from("section"), Value::String(sec.to_ascii_lowercase()));
            }
        }
        let text = serde_json::to_string_pretty(&val).map_err(|err| ResourceError::Internal(format!("resource JSON serialize failed: {err}")))?;
        return Ok(vec![ResourceContent { uri: trimmed.to_owned(), mime_type: String::from("application/json"), text }]);
    }
    if let Some(result_raw) = trimmed.strip_prefix("result://") {
        let (id_part, query) = crate::resources::split_resource_query(result_raw)?;
        let id_str = id_part.trim();
        if id_str.is_empty() {
            return Err(ResourceError::InvalidUri(String::from("result:// URI requires a non-empty result sheet id")));
        }
        let inspect = state.send_live_control("doc.inspect", json!({ "sheet_id": id_str })).map_err(|err| map_live_resource_error(uri, &err))?;
        let is_result =
            inspect.get("section").and_then(Value::as_str).is_some_and(|s| s.eq_ignore_ascii_case("results") || s.eq_ignore_ascii_case("analyses"));
        if !is_result {
            return Err(ResourceError::NotFound(format!("resource not found: {trimmed}")));
        }
        let exported = state
            .send_live_control("doc.export", json!({ "format": "json", "sheet_id": id_str, "pretty": true }))
            .map_err(|err| map_live_resource_error(uri, &err))?;
        let b64 = exported
            .get("data_base64")
            .and_then(Value::as_str)
            .ok_or_else(|| ResourceError::Internal(String::from("live doc.export response missing data_base64")))?;
        let bytes = decode_base64(b64).map_err(|err| ResourceError::Internal(format!("invalid base64 in live doc.export: {err}")))?;
        let raw_text = String::from_utf8(bytes).map_err(|err| ResourceError::Internal(format!("UTF-8 decode failed: {err}")))?;
        let include_curves = query.include_curves.unwrap_or(!query.detail_summary);
        if include_curves && query.tab_id.is_none() && query.max_rows.is_none() {
            return Ok(vec![ResourceContent { uri: trimmed.to_owned(), mime_type: String::from("application/json"), text: raw_text }]);
        }
        let mut parsed: Value =
            serde_json::from_str(&raw_text).map_err(|err| ResourceError::Internal(format!("live result JSON parse failed: {err}")))?;
        apply_live_resource_query_to_json(&mut parsed, id_str, &query)?;
        let filtered_text =
            serde_json::to_string(&parsed).map_err(|err| ResourceError::Internal(format!("resource JSON serialize failed: {err}")))?;
        return Ok(vec![ResourceContent { uri: trimmed.to_owned(), mime_type: String::from("application/json"), text: filtered_text }]);
    }
    Err(ResourceError::InvalidUri(format!("unsupported resource URI scheme '{trimmed}'; expected sheet://<id> or result://<id>")))
}

#[cfg(not(target_arch = "wasm32"))]
fn map_live_resource_error(uri: &str, err: &StructuredErrorPayload) -> ResourceError {
    if err.code == ErrorCode::NotFound || err.message.contains("not found") {
        ResourceError::NotFound(format!("resource not found: {uri}"))
    } else {
        ResourceError::Internal(err.message.clone())
    }
}

fn handle_prompts_list(id: Value, params: &Value) -> JsonRpcResponse {
    if !params.is_object() {
        return JsonRpcResponse::err(id, ERR_INVALID_PARAMS, "prompts/list params must be an object", None);
    }
    JsonRpcResponse::ok(id, json!({ "prompts": list_prompts() }))
}

fn handle_prompts_get(id: Value, params: &Value) -> JsonRpcResponse {
    let Some(map) = params.as_object() else {
        return JsonRpcResponse::err(id, ERR_INVALID_PARAMS, "prompts/get params must be an object", None);
    };
    let Some(name) = map.get("name").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) else {
        return JsonRpcResponse::err(id, ERR_INVALID_PARAMS, "prompts/get requires a non-empty string \"name\"", None);
    };
    let arguments = match map.get("arguments") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(obj)) => obj.clone(),
        Some(_) => {
            return JsonRpcResponse::err(id, ERR_INVALID_PARAMS, "prompts/get \"arguments\" must be an object", None);
        }
    };

    match get_prompt(name, &arguments) {
        Ok(result) => match serde_json::to_value(result) {
            Ok(val) => JsonRpcResponse::ok(id, val),
            Err(err) => JsonRpcResponse::err(id, ERR_INTERNAL_ERROR, format!("failed to encode prompt result: {err}"), None),
        },
        Err(msg) => JsonRpcResponse::err(id, ERR_INVALID_PARAMS, msg, None),
    }
}

fn read_next_frame<R: BufRead>(reader: &mut R, max_bytes: usize, fallback_mode: FramingMode) -> io::Result<NextFrame> {
    let effective_max = max_bytes.clamp(1, MAX_REQUEST_BYTES);
    let Some(first_byte) = skip_leading_ascii_whitespace(reader)? else {
        return Ok(NextFrame::Eof);
    };

    if first_byte == b'{' || first_byte == b'[' {
        return read_json_newline_frame(reader, effective_max);
    }

    match read_line_bytes(reader, MAX_HEADER_LINE_BYTES)? {
        ReadLineOutcome::Eof(first_line) if is_ascii_blank(&first_line) => Ok(NextFrame::Eof),
        ReadLineOutcome::Line(first_line) | ReadLineOutcome::Eof(first_line) => {
            if looks_like_header_line(&first_line) {
                return read_content_length_frame(reader, first_line, effective_max);
            }
            if first_line.len() > effective_max {
                return Ok(NextFrame::RecoverableError {
                    id: Value::Null,
                    mode: FramingMode::Newline,
                    code: ERR_INVALID_REQUEST,
                    message: format!("line exceeds maximum request size of {effective_max} bytes"),
                });
            }
            Ok(NextFrame::Payload { bytes: first_line, mode: fallback_or_newline(fallback_mode) })
        }
        ReadLineOutcome::TooLong { looks_like_header: true, .. } => Ok(NextFrame::FatalError {
            id: Value::Null,
            mode: FramingMode::ContentLength,
            code: ERR_INVALID_REQUEST,
            message: format!("header line exceeds maximum {MAX_HEADER_LINE_BYTES} bytes"),
        }),
        ReadLineOutcome::TooLong { id, looks_like_header: false } => Ok(NextFrame::RecoverableError {
            id,
            mode: FramingMode::Newline,
            code: ERR_INVALID_REQUEST,
            message: format!("line exceeds maximum request size of {} bytes", effective_max.min(MAX_HEADER_LINE_BYTES)),
        }),
        ReadLineOutcome::DrainLimitExceeded => Ok(NextFrame::FatalError {
            id: Value::Null,
            mode: FramingMode::ContentLength,
            code: ERR_INVALID_REQUEST,
            message: format!("oversized header/newline frame exceeds maximum drain ceiling of {MAX_DRAIN_BYTES} bytes"),
        }),
    }
}

fn fallback_or_newline(last_mode: FramingMode) -> FramingMode {
    let _ = last_mode;
    FramingMode::Newline
}

fn skip_leading_ascii_whitespace<R: BufRead>(reader: &mut R) -> io::Result<Option<u8>> {
    loop {
        let avail = reader.fill_buf()?;
        if avail.is_empty() {
            return Ok(None);
        }
        if let Some((pos, &first)) = avail.iter().enumerate().find(|(_, b)| !b.is_ascii_whitespace()) {
            reader.consume(pos);
            return Ok(Some(first));
        }
        let n = avail.len();
        reader.consume(n);
    }
}

fn read_json_newline_frame<R: BufRead>(reader: &mut R, effective_max: usize) -> io::Result<NextFrame> {
    match read_line_bytes(reader, effective_max)? {
        ReadLineOutcome::Line(bytes) | ReadLineOutcome::Eof(bytes) => Ok(NextFrame::Payload { bytes, mode: FramingMode::Newline }),
        ReadLineOutcome::TooLong { id, .. } => Ok(NextFrame::RecoverableError {
            id,
            mode: FramingMode::Newline,
            code: ERR_INVALID_REQUEST,
            message: format!("line exceeds maximum request size of {effective_max} bytes"),
        }),
        ReadLineOutcome::DrainLimitExceeded => Ok(NextFrame::FatalError {
            id: Value::Null,
            mode: FramingMode::Newline,
            code: ERR_INVALID_REQUEST,
            message: format!("oversized newline frame exceeds maximum drain ceiling of {MAX_DRAIN_BYTES} bytes"),
        }),
    }
}

fn read_content_length_frame<R: BufRead>(reader: &mut R, first_line: Vec<u8>, max_bytes: usize) -> io::Result<NextFrame> {
    let mut header_lines = vec![first_line];
    let mut terminated_cleanly = false;

    while header_lines.len() <= MAX_HEADER_COUNT {
        match read_line_bytes(reader, MAX_HEADER_LINE_BYTES)? {
            ReadLineOutcome::TooLong { .. } | ReadLineOutcome::DrainLimitExceeded => {
                return Ok(NextFrame::FatalError {
                    id: Value::Null,
                    mode: FramingMode::ContentLength,
                    code: ERR_INVALID_REQUEST,
                    message: format!("header line exceeds maximum {MAX_HEADER_LINE_BYTES} bytes"),
                });
            }
            ReadLineOutcome::Eof(tail) => {
                if is_ascii_blank(&tail) {
                    break;
                }
                return Ok(NextFrame::FatalError {
                    id: Value::Null,
                    mode: FramingMode::ContentLength,
                    code: ERR_PARSE_ERROR,
                    message: String::from("unexpected EOF while reading framing headers"),
                });
            }
            ReadLineOutcome::Line(line) => {
                if is_header_terminator(&line) {
                    terminated_cleanly = true;
                    break;
                }
                header_lines.push(line);
            }
        }
    }

    if !terminated_cleanly {
        return Ok(NextFrame::FatalError {
            id: Value::Null,
            mode: FramingMode::ContentLength,
            code: ERR_INVALID_REQUEST,
            message: String::from("framing header block missing blank separator or exceeded header cap"),
        });
    }

    let content_length = match parse_headers_content_length(&header_lines, max_bytes) {
        Ok(len) => len,
        Err((code, message)) => {
            return Ok(NextFrame::FatalError { id: Value::Null, mode: FramingMode::ContentLength, code, message });
        }
    };

    if content_length > MAX_DRAIN_BYTES {
        return Ok(NextFrame::FatalError {
            id: Value::Null,
            mode: FramingMode::ContentLength,
            code: ERR_INVALID_REQUEST,
            message: format!("Content-Length {content_length} exceeds maximum {max_bytes} bytes (and drain limit {MAX_DRAIN_BYTES} bytes)"),
        });
    }

    if content_length > max_bytes {
        return drain_oversized_content_length_frame(reader, content_length, max_bytes);
    }

    read_exact_body(reader, content_length)
}

fn parse_headers_content_length(header_lines: &[Vec<u8>], max_bytes: usize) -> Result<usize, (i64, String)> {
    let mut content_length: Option<usize> = None;

    for raw in header_lines {
        let line_str = std::str::from_utf8(raw).map_err(|_| (ERR_PARSE_ERROR, String::from("header contains invalid UTF-8")))?;
        let trimmed = line_str.trim_end_matches(['\r', '\n']);
        let Some((name, value)) = trimmed.split_once(':') else {
            return Err((ERR_INVALID_REQUEST, String::from("malformed framing header without ':'")));
        };
        if name.is_empty() || !name.bytes().all(is_http_token_byte) {
            return Err((ERR_INVALID_REQUEST, String::from("invalid framing header name")));
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err((ERR_INVALID_REQUEST, String::from("duplicate Content-Length header")));
            }
            let parsed = parse_content_length_value(value, max_bytes)?;
            content_length = Some(parsed);
        }
    }

    content_length.ok_or((ERR_INVALID_REQUEST, String::from("missing Content-Length header")))
}

fn parse_content_length_value(raw: &str, max_bytes: usize) -> Result<usize, (i64, String)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || !trimmed.bytes().all(|b| b.is_ascii_digit()) {
        return Err((ERR_INVALID_REQUEST, String::from("Content-Length must be a non-negative integer")));
    }
    trimmed.parse::<usize>().map_err(|_| (ERR_INVALID_REQUEST, format!("Content-Length exceeds maximum {max_bytes} bytes")))
}

fn read_exact_body<R: BufRead>(reader: &mut R, len: usize) -> io::Result<NextFrame> {
    if len == 0 {
        return Ok(NextFrame::Payload { bytes: Vec::new(), mode: FramingMode::ContentLength });
    }

    let mut body = Vec::new();
    while body.len() < len {
        let remaining = len - body.len();
        let avail = reader.fill_buf()?;
        if avail.is_empty() {
            return Ok(NextFrame::FatalError {
                id: Value::Null,
                mode: FramingMode::ContentLength,
                code: ERR_PARSE_ERROR,
                message: format!("unexpected EOF after {} of {len} Content-Length bytes", body.len()),
            });
        }
        let take = avail.len().min(remaining).min(READ_CHUNK_BYTES);
        let chunk = avail.get(..take).ok_or_else(|| io::Error::other("short fill_buf slice"))?;
        body.extend_from_slice(chunk);
        reader.consume(take);
    }

    Ok(NextFrame::Payload { bytes: body, mode: FramingMode::ContentLength })
}

fn drain_oversized_content_length_frame<R: BufRead>(reader: &mut R, content_length: usize, max_bytes: usize) -> io::Result<NextFrame> {
    let mut drained = 0usize;
    let mut sniff_prefix = Vec::new();
    let sniff_cap = ID_SNIFF_BYTES.min(content_length);

    while drained < content_length {
        let remaining = content_length - drained;
        let avail = reader.fill_buf()?;
        if avail.is_empty() {
            return Ok(NextFrame::FatalError {
                id: extract_jsonrpc_id_prefix(&sniff_prefix),
                mode: FramingMode::ContentLength,
                code: ERR_INVALID_REQUEST,
                message: format!("Content-Length {content_length} exceeds maximum {max_bytes} bytes (unexpected EOF after {drained} bytes)"),
            });
        }
        let take = avail.len().min(remaining).min(READ_CHUNK_BYTES);
        if sniff_prefix.len() < sniff_cap {
            let keep = take.min(sniff_cap - sniff_prefix.len());
            if let Some(chunk) = avail.get(..keep) {
                sniff_prefix.extend_from_slice(chunk);
            }
        }
        reader.consume(take);
        drained += take;
    }

    let id = extract_jsonrpc_id_prefix(&sniff_prefix);
    Ok(NextFrame::RecoverableError {
        id,
        mode: FramingMode::ContentLength,
        code: ERR_INVALID_REQUEST,
        message: format!("Content-Length {content_length} exceeds maximum {max_bytes} bytes"),
    })
}

fn read_line_bytes<R: BufRead>(reader: &mut R, max_bytes: usize) -> io::Result<ReadLineOutcome> {
    let mut buf = Vec::new();
    let sniff_cap = ID_SNIFF_BYTES.min(max_bytes);
    loop {
        let avail = reader.fill_buf()?;
        if avail.is_empty() {
            return Ok(ReadLineOutcome::Eof(buf));
        }

        if let Some(pos) = avail.iter().position(|&b| b == b'\n') {
            let take = pos + 1;
            if buf.len().saturating_add(take) > max_bytes {
                if buf.len() < sniff_cap {
                    let keep = take.min(sniff_cap - buf.len());
                    if let Some(chunk) = avail.get(..keep) {
                        buf.extend_from_slice(chunk);
                    }
                }
                reader.consume(take);
                return Ok(ReadLineOutcome::TooLong { id: extract_jsonrpc_id_prefix(&buf), looks_like_header: has_header_name_prefix(&buf) });
            }
            let chunk = avail.get(..take).ok_or_else(|| io::Error::other("short fill_buf line slice"))?;
            buf.extend_from_slice(chunk);
            reader.consume(take);
            return Ok(ReadLineOutcome::Line(buf));
        }

        if buf.len().saturating_add(avail.len()) > max_bytes {
            let avail_len = avail.len();
            if buf.len() < sniff_cap {
                let keep = avail_len.min(sniff_cap - buf.len());
                if let Some(chunk) = avail.get(..keep) {
                    buf.extend_from_slice(chunk);
                }
            }
            reader.consume(avail_len);
            if !drain_until_newline_bounded(reader, MAX_DRAIN_BYTES)? {
                return Ok(ReadLineOutcome::DrainLimitExceeded);
            }
            return Ok(ReadLineOutcome::TooLong { id: extract_jsonrpc_id_prefix(&buf), looks_like_header: has_header_name_prefix(&buf) });
        }

        let take = avail.len();
        buf.extend_from_slice(avail);
        reader.consume(take);
    }
}

fn drain_until_newline_bounded<R: BufRead>(reader: &mut R, max_drain: usize) -> io::Result<bool> {
    let mut drained = 0usize;
    loop {
        let avail = reader.fill_buf()?;
        if avail.is_empty() {
            return Ok(true);
        }
        if let Some(pos) = avail.iter().position(|&b| b == b'\n') {
            let take = pos + 1;
            if drained.saturating_add(take) > max_drain {
                return Ok(false);
            }
            reader.consume(take);
            return Ok(true);
        }
        let len = avail.len();
        drained = drained.saturating_add(len);
        if drained > max_drain {
            return Ok(false);
        }
        reader.consume(len);
    }
}

fn extract_jsonrpc_id_prefix(prefix: &[u8]) -> Value {
    let mut i = skip_ascii_ws(prefix, 0);
    if prefix.get(i).copied() != Some(b'{') {
        return Value::Null;
    }
    i += 1;

    loop {
        i = skip_ascii_ws(prefix, i);
        let Some((key, next_pos)) = scan_json_string(prefix, i) else {
            return Value::Null;
        };
        i = skip_ascii_ws(prefix, next_pos);
        if prefix.get(i).copied() != Some(b':') {
            return Value::Null;
        }
        i = skip_ascii_ws(prefix, i + 1);

        if key == "id" {
            return parse_jsonrpc_id_at(prefix, i);
        }

        let Some(after_val) = skip_json_value(prefix, i) else {
            return Value::Null;
        };
        i = skip_ascii_ws(prefix, after_val);
        if prefix.get(i).copied() != Some(b',') {
            return Value::Null;
        }
        i += 1;
    }
}

fn parse_jsonrpc_id_at(bytes: &[u8], start: usize) -> Value {
    let Some(&first) = bytes.get(start) else {
        return Value::Null;
    };
    match first {
        b'"' => scan_json_string(bytes, start).map_or(Value::Null, |(s, _)| Value::String(s)),
        b'n' => Value::Null,
        b'-' | b'0'..=b'9' => {
            let mut end = start;
            while let Some(&b) = bytes.get(end) {
                if b.is_ascii_digit() || matches!(b, b'-' | b'+' | b'.' | b'e' | b'E') {
                    end += 1;
                } else {
                    break;
                }
            }
            if end == bytes.len() {
                return Value::Null;
            }
            let Some(num_bytes) = bytes.get(start..end) else {
                return Value::Null;
            };
            match serde_json::from_slice::<Value>(num_bytes) {
                Ok(v @ Value::Number(_)) => v,
                _ => Value::Null,
            }
        }
        _ => Value::Null,
    }
}

fn skip_ascii_ws(bytes: &[u8], mut pos: usize) -> usize {
    while bytes.get(pos).is_some_and(u8::is_ascii_whitespace) {
        pos += 1;
    }
    pos
}

fn scan_json_string(bytes: &[u8], start: usize) -> Option<(String, usize)> {
    if bytes.get(start).copied() != Some(b'"') {
        return None;
    }
    let end = scan_json_string_end(bytes, start)?;
    let slice = bytes.get(start..end)?;
    let s: String = serde_json::from_slice(slice).ok()?;
    Some((s, end))
}

fn scan_json_string_end(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start).copied() != Some(b'"') {
        return None;
    }
    let mut pos = start + 1;
    let mut escaped = false;
    while let Some(&b) = bytes.get(pos) {
        pos += 1;
        if escaped {
            escaped = false;
        } else if b == b'\\' {
            escaped = true;
        } else if b == b'"' {
            return Some(pos);
        }
    }
    None
}

fn skip_json_value(bytes: &[u8], start: usize) -> Option<usize> {
    let pos = skip_ascii_ws(bytes, start);
    let &first = bytes.get(pos)?;
    match first {
        b'"' => scan_json_string_end(bytes, pos),
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut i = pos;
            while let Some(&b) = bytes.get(i) {
                match b {
                    b'"' => {
                        i = scan_json_string_end(bytes, i)?;
                    }
                    b'{' | b'[' => {
                        depth += 1;
                        i += 1;
                    }
                    b'}' | b']' => {
                        depth = depth.checked_sub(1)?;
                        i += 1;
                        if depth == 0 {
                            return Some(i);
                        }
                    }
                    _ => i += 1,
                }
            }
            None
        }
        _ => {
            let mut i = pos;
            while let Some(&b) = bytes.get(i) {
                if matches!(b, b',' | b'}' | b']') || b.is_ascii_whitespace() {
                    return Some(i);
                }
                i += 1;
            }
            None
        }
    }
}

fn has_header_name_prefix(bytes: &[u8]) -> bool {
    let trimmed = trim_ascii_whitespace(bytes);
    let Some(colon_pos) = trimmed.iter().position(|&b| b == b':') else {
        return false;
    };
    colon_pos > 0 && trimmed.get(..colon_pos).is_some_and(|name| name.iter().copied().all(is_http_token_byte))
}

fn looks_like_header_line(bytes: &[u8]) -> bool {
    let trimmed = trim_ascii_whitespace(bytes);
    let Some(colon_pos) = trimmed.iter().position(|&b| b == b':') else {
        return false;
    };
    if colon_pos == 0 {
        return false;
    }
    let Some(name) = trimmed.get(..colon_pos) else {
        return false;
    };
    if !name.iter().copied().all(is_http_token_byte) {
        return false;
    }
    name.eq_ignore_ascii_case(b"content-length") || name.eq_ignore_ascii_case(b"content-type") || bytes.ends_with(b"\r\n")
}

fn is_header_terminator(bytes: &[u8]) -> bool {
    bytes == b"\r\n" || bytes == b"\n"
}

fn is_ascii_blank(bytes: &[u8]) -> bool {
    bytes.iter().all(u8::is_ascii_whitespace)
}

fn trim_ascii_whitespace(mut bytes: &[u8]) -> &[u8] {
    while let Some((&b, rest)) = bytes.split_first() {
        if b.is_ascii_whitespace() {
            bytes = rest;
        } else {
            break;
        }
    }
    while let Some((&b, rest)) = bytes.split_last() {
        if b.is_ascii_whitespace() {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}

const fn is_http_token_byte(b: u8) -> bool {
    matches!(
        b,
        b'0'..=b'9'
            | b'A'..=b'Z'
            | b'a'..=b'z'
            | b'!'
            | b'#'
            | b'$'
            | b'%'
            | b'&'
            | b'\''
            | b'*'
            | b'+'
            | b'-'
            | b'.'
            | b'^'
            | b'_'
            | b'`'
            | b'|'
            | b'~'
    )
}

fn write_response<W: Write>(writer: &mut W, response: &JsonRpcResponse, mode: FramingMode) -> io::Result<()> {
    let bytes = serde_json::to_vec(response).map_err(io::Error::other)?;
    write_frame(writer, &bytes, mode)
}

fn write_json_value<W: Write>(writer: &mut W, value: &Value, mode: FramingMode) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    write_frame(writer, &bytes, mode)
}

fn write_frame<W: Write>(writer: &mut W, json_bytes: &[u8], mode: FramingMode) -> io::Result<()> {
    match mode {
        FramingMode::ContentLength => {
            write!(writer, "Content-Length: {}\r\n\r\n", json_bytes.len())?;
            writer.write_all(json_bytes)?;
        }
        FramingMode::Newline => {
            writer.write_all(json_bytes)?;
            writer.write_all(b"\n")?;
        }
    }
    writer.flush()
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use serde_json::{Value, json};

    use super::{
        ERR_INVALID_PARAMS, ERR_INVALID_REQUEST, ERR_METHOD_NOT_FOUND, ERR_PARSE_ERROR, FramingMode, MAX_DRAIN_BYTES, MAX_REQUEST_BYTES, serve,
        serve_with_max_bytes,
    };
    use crate::tools::McpServerState;

    fn frame_content_length(body: &str) -> Vec<u8> {
        format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    }

    fn decode_content_length_frames(raw: &[u8]) -> Vec<Value> {
        let mut cursor = raw;
        let mut out = Vec::new();
        while !cursor.is_empty() {
            let text = std::str::from_utf8(cursor).expect("valid utf8");
            let (headers, rest) = text.split_once("\r\n\r\n").expect("header separator");
            let len: usize = headers.strip_prefix("Content-Length: ").expect("content-length prefix").trim().parse().expect("length");
            let body = &rest.as_bytes()[..len];
            out.push(serde_json::from_slice(body).expect("json body"));
            cursor = &rest.as_bytes()[len..];
        }
        out
    }

    #[test]
    fn content_length_initialize_notification_ping_and_tools_list() {
        let mut input = Vec::new();
        input.extend(frame_content_length(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#));
        input.extend(frame_content_length(r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#));
        input.extend(frame_content_length(r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#));
        input.extend(frame_content_length(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#));

        let mut reader = Cursor::new(input);
        let mut output = Vec::new();
        let mut state = McpServerState::new();
        serve(&mut reader, &mut output, &mut state).expect("serve");

        let replies = decode_content_length_frames(&output);
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0]["id"], json!(1));
        assert_eq!(replies[0]["result"]["protocolVersion"], json!("2024-11-05"));
        assert_eq!(replies[0]["result"]["serverInfo"]["name"], json!("curvebook"));
        assert!(replies[0]["result"]["instructions"].as_str().is_some_and(|s| !s.is_empty()));
        assert_eq!(replies[0]["result"]["capabilities"]["tools"]["listChanged"], json!(true));
        assert!(replies[0]["result"]["capabilities"]["resources"].is_object());
        assert!(replies[0]["result"]["capabilities"]["prompts"].is_object());
        assert_eq!(replies[1]["id"], json!(2));
        assert_eq!(replies[1]["result"], json!({}));
        assert_eq!(replies[2]["id"], json!(3));
        assert_eq!(replies[2]["result"]["tools"].as_array().expect("tools").len(), 10);
    }

    #[test]
    fn newline_framing_errors_and_batch() {
        let script = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":\"p1\",\"method\":\"ping\"}\n",
            "{not valid json}\n",
            "{\"jsonrpc\":\"1.0\",\"id\":9,\"method\":\"ping\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":10,\"method\":\"nope\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":11,\"method\":\"tools/call\",\"params\":{\"name\":\"missing_tool\"}}\n",
            "[{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"},{\"jsonrpc\":\"2.0\",\"id\":12,\"method\":\"ping\"}]\n",
            "[]\n",
        );

        let mut reader = Cursor::new(script.as_bytes());
        let mut output = Vec::new();
        let mut state = McpServerState::new();
        serve(&mut reader, &mut output, &mut state).expect("serve");

        let lines: Vec<Value> = String::from_utf8(output).expect("utf8").lines().map(|l| serde_json::from_str(l).expect("json line")).collect();
        assert_eq!(lines.len(), 7);
        assert_eq!(lines[0]["id"], json!("p1"));
        assert_eq!(lines[1]["error"]["code"], json!(ERR_PARSE_ERROR));
        assert_eq!(lines[2]["error"]["code"], json!(ERR_INVALID_REQUEST));
        assert_eq!(lines[3]["error"]["code"], json!(ERR_METHOD_NOT_FOUND));
        assert_eq!(lines[4]["error"]["code"], json!(ERR_INVALID_PARAMS));
        assert_eq!(lines[5].as_array().expect("batch").len(), 1);
        assert_eq!(lines[5][0]["id"], json!(12));
        assert_eq!(lines[6]["error"]["code"], json!(ERR_INVALID_REQUEST));
    }

    #[test]
    fn oversized_content_length_is_rejected_without_oom() {
        let raw = b"Content-Length: 999999999999\r\n\r\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}";
        let mut reader = Cursor::new(&raw[..]);
        let mut output = Vec::new();
        let mut state = McpServerState::new();
        serve_with_max_bytes(&mut reader, &mut output, &mut state, 1024).expect("serve");
        let replies = decode_content_length_frames(&output);
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0]["error"]["code"], json!(ERR_INVALID_REQUEST));
        assert_eq!(FramingMode::ContentLength, FramingMode::ContentLength);
    }

    #[test]
    fn oversized_content_length_frame_drains_preserves_id_and_keeps_session_alive() {
        let mut input = Vec::new();
        input.extend(frame_content_length(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#));

        let big_pad = "x".repeat(2048);
        let oversized_str_id = format!(r#"{{"jsonrpc":"2.0","id":"big-1","method":"ping","params":{{"pad":"{big_pad}"}}}}"#);
        input.extend(frame_content_length(&oversized_str_id));
        input.extend(frame_content_length(r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#));

        let oversized_num_id = format!(r#"{{"jsonrpc":"2.0","id":99,"method":"tools/call","params":{{"blob":"{big_pad}"}}}}"#);
        input.extend(frame_content_length(&oversized_num_id));
        input.extend(frame_content_length(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#));

        let mut reader = Cursor::new(input);
        let mut output = Vec::new();
        let mut state = McpServerState::new();
        serve_with_max_bytes(&mut reader, &mut output, &mut state, 1024).expect("serve");

        let replies = decode_content_length_frames(&output);
        assert_eq!(replies.len(), 5, "all requests before and after oversized frames must be answered");
        assert_eq!(replies[0]["id"], json!(0));
        assert_eq!(replies[1]["id"], json!("big-1"));
        assert_eq!(replies[1]["error"]["code"], json!(ERR_INVALID_REQUEST));
        assert_eq!(replies[2]["id"], json!(2));
        assert_eq!(replies[2]["result"], json!({}));
        assert_eq!(replies[3]["id"], json!(99));
        assert_eq!(replies[3]["error"]["code"], json!(ERR_INVALID_REQUEST));
        assert_eq!(replies[4]["id"], json!(3));
        assert!(replies[4]["result"]["tools"].is_array());
    }

    #[test]
    fn default_serve_drains_17mib_content_length_and_answers_next_request() {
        let mut input = Vec::new();
        input.extend(frame_content_length(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#));
        let pad = "a".repeat(17 * 1024 * 1024 - 128);
        let oversized = format!(r#"{{"jsonrpc":"2.0","id":1,"method":"ping","params":{{"data":"{pad}"}}}}"#);
        assert!(oversized.len() > MAX_REQUEST_BYTES);
        assert!(oversized.len() < MAX_DRAIN_BYTES);
        input.extend(frame_content_length(&oversized));
        input.extend(frame_content_length(r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#));

        let mut reader = Cursor::new(input);
        let mut output = Vec::new();
        let mut state = McpServerState::new();
        serve(&mut reader, &mut output, &mut state).expect("serve");

        let replies = decode_content_length_frames(&output);
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0]["id"], json!(0));
        assert_eq!(replies[1]["id"], json!(1));
        assert_eq!(replies[1]["error"]["code"], json!(ERR_INVALID_REQUEST));
        assert_eq!(replies[2]["id"], json!(2));
        assert_eq!(replies[2]["result"], json!({}));
    }

    #[test]
    fn oversized_newline_frame_drains_preserves_id_and_recovers() {
        let big = "z".repeat(2048);
        let script = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":\"nl-1\",\"method\":\"ping\",\"params\":{{\"x\":\"{big}\"}}}}\n{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\"}}\n"
        );
        let mut reader = Cursor::new(script.into_bytes());
        let mut output = Vec::new();
        let mut state = McpServerState::new();
        serve_with_max_bytes(&mut reader, &mut output, &mut state, 1024).expect("serve");

        let lines: Vec<Value> = String::from_utf8(output).expect("utf8").lines().map(|l| serde_json::from_str(l).expect("json line")).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["id"], json!("nl-1"));
        assert_eq!(lines[0]["error"]["code"], json!(ERR_INVALID_REQUEST));
        assert_eq!(lines[1]["id"], json!(7));
        assert_eq!(lines[1]["result"], json!({}));
    }

    #[test]
    fn rv15_serve_with_max_bytes_clamps_both_framings_and_enforces_first_header_cap() {
        // 1. Oversized FIRST header line (> 8 KiB) must be rejected at MAX_HEADER_LINE_BYTES without buffering 16 MiB.
        for huge_first_hdr in [
            format!("X-First-Huge: {}\r\nContent-Length: 2\r\n\r\n{{}}", "a".repeat(super::MAX_HEADER_LINE_BYTES + 512)),
            format!("Content-Length: {:0>9000}\r\n\r\n{{}}", 2),
        ] {
            let mut reader = Cursor::new(huge_first_hdr.into_bytes());
            let mut output = Vec::new();
            let mut state = McpServerState::new();
            serve(&mut reader, &mut output, &mut state).expect("serve oversized first header");
            let replies = decode_content_length_frames(&output);
            assert_eq!(replies.len(), 1);
            assert_eq!(replies[0]["error"]["code"], json!(ERR_INVALID_REQUEST));
            assert!(replies[0]["error"]["message"].as_str().is_some_and(|m| m.contains(&super::MAX_HEADER_LINE_BYTES.to_string())));
        }

        // 2. Single-line JSON-RPC requests starting with `{` or `[` larger than 8 KiB still succeed up to effective_max.
        let pad_12k = "p".repeat(12 * 1024);
        let jsonl_12k = format!("  {{\"jsonrpc\":\"2.0\",\"id\":\"ok-12k\",\"method\":\"ping\",\"params\":{{\"pad\":\"{pad_12k}\"}}}}\n");
        let mut reader = Cursor::new(jsonl_12k.into_bytes());
        let mut output = Vec::new();
        let mut state = McpServerState::new();
        serve_with_max_bytes(&mut reader, &mut output, &mut state, 32 * 1024).expect("serve 12k jsonl");
        let reply: Value = serde_json::from_slice(&output).expect("valid jsonl reply");
        assert_eq!(reply["id"], json!("ok-12k"));
        assert_eq!(reply["result"], json!({}));

        // 3. Passing max_bytes > MAX_REQUEST_BYTES clamps Content-Length body limit to MAX_REQUEST_BYTES (16 MiB).
        let over_16m_pad = "b".repeat(MAX_REQUEST_BYTES + 2048);
        let over_16m_body = format!(r#"{{"jsonrpc":"2.0","id":"cl-clamp","method":"ping","params":{{"x":"{over_16m_pad}"}}}}"#);
        let mut cl_input = frame_content_length(&over_16m_body);
        cl_input.extend(frame_content_length(r#"{"jsonrpc":"2.0","id":"cl-next","method":"ping"}"#));
        let mut reader = Cursor::new(cl_input);
        let mut output = Vec::new();
        let mut state = McpServerState::new();
        serve_with_max_bytes(&mut reader, &mut output, &mut state, MAX_REQUEST_BYTES * 4).expect("serve clamped CL");
        let cl_replies = decode_content_length_frames(&output);
        assert_eq!(cl_replies.len(), 2);
        assert_eq!(cl_replies[0]["id"], json!("cl-clamp"));
        assert_eq!(cl_replies[0]["error"]["code"], json!(ERR_INVALID_REQUEST));
        assert!(cl_replies[0]["error"]["message"].as_str().is_some_and(|m| m.contains(&MAX_REQUEST_BYTES.to_string())));
        assert_eq!(cl_replies[1]["id"], json!("cl-next"));
        assert_eq!(cl_replies[1]["result"], json!({}));

        // 4. Passing max_bytes = 0 clamps to 1 cleanly for both framings without panicking.
        let mut zero_out = Vec::new();
        let mut state = McpServerState::new();
        serve_with_max_bytes(&mut Cursor::new(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n".to_vec()), &mut zero_out, &mut state, 0)
            .expect("zero max_bytes");
        let zero_reply: Value = serde_json::from_slice(&zero_out).expect("zero jsonl reply");
        assert_eq!(zero_reply["error"]["code"], json!(ERR_INVALID_REQUEST));
    }

    #[test]
    fn rv18_initialize_and_shutdown_lifecycle_guards_preserve_stateless_default() {
        // Strict require_initialize rejects operational calls before initialize, allows ping, then succeeds after initialize.
        let mut strict = McpServerState::new().with_require_initialize(true);
        let mut out = Vec::new();
        let pre_init = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\"}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\"}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/list\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"shutdown\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/list\"}\n",
        );
        serve(&mut Cursor::new(pre_init.as_bytes()), &mut out, &mut strict).expect("serve lifecycle");
        let replies: Vec<Value> = String::from_utf8(out).expect("utf8").lines().map(|l| serde_json::from_str(l).expect("json")).collect();
        assert_eq!(replies.len(), 7);
        assert_eq!(replies[0]["error"]["code"], json!(ERR_INVALID_REQUEST), "pre-init tools/list must fail when require_initialize is set");
        assert_eq!(replies[1]["result"], json!({}), "ping remains available before initialize");
        assert_eq!(replies[2]["result"]["protocolVersion"], json!("2024-11-05"));
        assert_eq!(replies[3]["error"]["code"], json!(ERR_INVALID_REQUEST), "duplicate initialize must fail");
        assert!(replies[4]["result"]["tools"].is_array(), "tools/list succeeds once initialized");
        assert_eq!(replies[5]["result"], json!({}), "shutdown succeeds");
        assert_eq!(replies[6]["error"]["code"], json!(ERR_INVALID_REQUEST), "post-shutdown tools/list must fail");
    }
}
