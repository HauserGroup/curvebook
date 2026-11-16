//! JSON-RPC 2.0 transport and MCP message framing over arbitrary `BufRead + Write` streams.
//!
//! Supports both LSP-style `Content-Length: N\r\n\r\n` framing and newline-delimited JSON-RPC,
//! enforcing strict size caps and zero-panic error reporting on malformed or adversarial inputs.

use std::io::{self, BufRead, BufReader, BufWriter, Write};

use lumencraft_doc::ErrorCode;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::SERVER_NAME;
use crate::errors::{self, StructuredErrorPayload};
use crate::prompts::{get_prompt_for_mode, list_prompts_for_mode};
#[cfg(not(target_arch = "wasm32"))]
use crate::resources::{ResourceContent, ResourceDescriptor};
use crate::resources::{ResourceError, list_resources, read_resource};
#[cfg(not(target_arch = "wasm32"))]
use crate::tools::decode_base64;
use crate::tools::{McpServerState, ToolCallError, call_tool, list_tools_for_state};

/// MCP protocol revision advertised by this server during `initialize`.
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Guidance string advertised to MCP clients during `initialize`.
pub const SERVER_INSTRUCTIONS: &str = "\
LumenCraft MCP server for scientific tables, statistical analysis, curve fitting, and publication figures. \
Use `doc_inspect` or MCP resources (`resources/list`, `resources/read` with `sheet://<id>` or `result://<id>`) to inspect active sheets and results. \
Core tools are available immediately; call `list_toolsets` and `enable_toolset` to dynamically load per-command CommandGroup tools (which emits `notifications/tools/list_changed`). \
Inspect each tool's `annotations` (`readOnlyHint`, `destructiveHint`, `idempotentHint`, `openWorldHint`) before executing mutating or filesystem commands, and use `prompts/list` + `prompts/get` for guided analysis workflows.";

/// Guidance string advertised during `initialize` when strict `--gui-only` policy is active.
pub const GUI_ONLY_SERVER_INSTRUCTIONS: &str = "\
LumenCraft MCP server running under strict `--gui-only` policy. \
Only the 8 human-equivalent GUI tools are exposed (`ui_inspect`, `ui_screenshot`, `ui_pointer_gesture`, `ui_type_text`, `ui_press_key`, `ui_invoke_menu`, `ui_switch_tab`, `ui_answer_dialog`). \
Direct engine commands (`command_run`, `command_batch`, `enable_toolset`), `doc_inspect`, file shortcuts (`open_file`, `save_file`, `export`, `render_preview`), and `resources/read` are blocked with `gui_only_violation`.";

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
    TooLong { id: Value },
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
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());
    serve(&mut reader, &mut writer, state)
}

/// Run the MCP server on process stdin/stdout attached to a live desktop `--control <port>` loop.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_stdio_with_control_port(port: u16) -> Result<(), String> {
    run_stdio_with_control_port_gui_only(port, false)
}

/// Run the MCP server on process stdin/stdout attached to a live desktop `--control <port>` loop
/// with explicit `--gui-only` policy state.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_stdio_with_control_port_gui_only(port: u16, gui_only: bool) -> Result<(), String> {
    run_stdio_with_control_port_options(port, gui_only, None)
}

/// Run the MCP server on process stdin/stdout attached to a live desktop `--control <port>` loop
/// with explicit `--gui-only` policy state and optional `--control-token`.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_stdio_with_control_port_options(port: u16, gui_only: bool, control_token: Option<&str>) -> Result<(), String> {
    let mut state = McpServerState::with_control_port_and_gui_only(port, gui_only)?;
    if let Some(token) = control_token {
        state.set_control_token(Some(token))?;
    }
    run_stdio_with_state(&mut state).map_err(|err| format!("stdio transport error: {err}"))
}

/// Serve MCP JSON-RPC 2.0 requests until EOF using [`MAX_REQUEST_BYTES`].
pub fn serve<R: BufRead, W: Write>(reader: &mut R, writer: &mut W, state: &mut McpServerState) -> io::Result<()> {
    serve_with_max_bytes(reader, writer, state, MAX_REQUEST_BYTES)
}

/// Serve MCP JSON-RPC 2.0 requests until EOF with an explicit per-request byte ceiling.
pub fn serve_with_max_bytes<R: BufRead, W: Write>(reader: &mut R, writer: &mut W, state: &mut McpServerState, max_bytes: usize) -> io::Result<()> {
    let mut last_mode = FramingMode::Newline;
    let effective_max = max_bytes.max(1);
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
        if method == "notifications/initialized" || method == "initialized" {
            state.mark_initialized();
        }
        return req_id.map(|id| JsonRpcResponse::ok(id, json!({})));
    }

    let id = req_id?;
    match method {
        "initialize" => Some(handle_initialize(id, &params, state)),
        "ping" => Some(JsonRpcResponse::ok(id, json!({}))),
        "tools/list" => Some(handle_tools_list(id, &params, state)),
        "tools/call" => Some(handle_tools_call(id, params, state)),
        "resources/list" => Some(handle_resources_list(id, &params, state)),
        "resources/read" => Some(handle_resources_read(id, &params, state)),
        "prompts/list" => Some(handle_prompts_list(id, &params, state)),
        "prompts/get" => Some(handle_prompts_get(id, &params, state)),
        _ => Some(JsonRpcResponse::err(id, ERR_METHOD_NOT_FOUND, format!("method not found: {method}"), None)),
    }
}

fn is_notification_method(method: &str, is_notification: bool) -> bool {
    is_notification || matches!(method, "notifications/initialized" | "initialized" | "notifications/cancelled")
}

fn rpc_error_from_payload(id: Value, payload: StructuredErrorPayload) -> JsonRpcResponse {
    let code = if payload.code == ErrorCode::GuiOnlyViolation { ERR_INVALID_REQUEST } else { ERR_INTERNAL_ERROR };
    JsonRpcResponse::err(id, code, payload.message.clone(), Some(payload.to_value()))
}

fn run_guarded_rpc<F>(id: Value, op: &str, f: F) -> JsonRpcResponse
where
    F: FnOnce(&Value) -> Result<JsonRpcResponse, lumencraft_engine::EngineError>,
{
    match lumencraft_engine::guard::run_guarded(op, || f(&id)) {
        Ok(resp) => resp,
        Err(err) => rpc_error_from_payload(id, StructuredErrorPayload::from_engine_error(&err)),
    }
}

fn handle_initialize(id: Value, params: &Value, state: &mut McpServerState) -> JsonRpcResponse {
    if !params.is_object() {
        return JsonRpcResponse::err(id, ERR_INVALID_PARAMS, "initialize params must be an object", None);
    }
    if let Err(payload) = state.sync_remote_control_policy() {
        return rpc_error_from_payload(id, payload);
    }
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let negotiated = negotiate_protocol_version(requested);
    let instructions = if state.is_gui_only() { GUI_ONLY_SERVER_INSTRUCTIONS } else { SERVER_INSTRUCTIONS };
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
            "instructions": instructions
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
    if let Err(payload) = state.sync_remote_control_policy() {
        return rpc_error_from_payload(id, payload);
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
        Err(ToolCallError::UnknownTool(unknown)) => JsonRpcResponse::err(id, ERR_INVALID_PARAMS, format!("unknown tool: {unknown}"), None),
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
        if let Err(payload) = state.sync_remote_control_policy() {
            return Ok(rpc_error_from_payload(req_id.clone(), payload));
        }
        if state.is_gui_only() {
            return Ok(JsonRpcResponse::ok(req_id.clone(), json!({ "resources": [] })));
        }
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

fn handle_resources_read(id: Value, params: &Value, state: &mut McpServerState) -> JsonRpcResponse {
    let Some(map) = params.as_object() else {
        return JsonRpcResponse::err(id, ERR_INVALID_PARAMS, "resources/read params must be an object", None);
    };
    let Some(uri) = map.get("uri").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) else {
        return JsonRpcResponse::err(id, ERR_INVALID_PARAMS, "resources/read requires a non-empty string \"uri\"", None);
    };
    run_guarded_rpc(id, "mcp.resources_read", |req_id| {
        if let Err(payload) = state.sync_remote_control_policy() {
            return Ok(rpc_error_from_payload(req_id.clone(), payload));
        }
        if state.is_gui_only() {
            let payload = StructuredErrorPayload::new(ErrorCode::GuiOnlyViolation, "resources/read is forbidden under --gui-only policy")
                .with_remediation("Inspect visible sheets via `ui_inspect` or `ui_screenshot` instead.")
                .with_details(json!({ "uri": uri, "policy": "gui_only" }));
            return Ok(JsonRpcResponse::err(req_id.clone(), ERR_INVALID_REQUEST, payload.message.clone(), Some(payload.to_value())));
        }

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
fn read_live_resource(state: &mut McpServerState, uri: &str) -> Result<Vec<ResourceContent>, ResourceError> {
    let trimmed = uri.trim();
    if let Some(sheet_id_raw) = trimmed.strip_prefix("sheet://") {
        let id_str = sheet_id_raw.trim();
        if id_str.is_empty() {
            return Err(ResourceError::InvalidUri(String::from("sheet:// URI requires a non-empty sheet id")));
        }
        let mut val = state
            .send_live_control("doc.inspect", json!({ "sheet_id": id_str, "include_cells": true }))
            .map_err(|err| map_live_resource_error(uri, &err))?;
        if let Some(obj) = val.as_object_mut() {
            obj.insert(String::from("uri"), Value::String(trimmed.to_owned()));
            if let Some(sec) = obj.get("section").and_then(Value::as_str) {
                obj.insert(String::from("section"), Value::String(sec.to_ascii_lowercase()));
            }
        }
        let text = serde_json::to_string_pretty(&val).map_err(|err| ResourceError::Internal(format!("resource JSON serialize failed: {err}")))?;
        return Ok(vec![ResourceContent { uri: trimmed.to_owned(), mime_type: String::from("application/json"), text }]);
    }
    if let Some(result_id_raw) = trimmed.strip_prefix("result://") {
        let id_str = result_id_raw.trim();
        if id_str.is_empty() {
            return Err(ResourceError::InvalidUri(String::from("result:// URI requires a non-empty result sheet id")));
        }
        let inspect = state.send_live_control("doc.inspect", json!({ "sheet_id": id_str })).map_err(|err| map_live_resource_error(uri, &err))?;
        let is_result = inspect.get("section").and_then(Value::as_str).is_some_and(|s| s.eq_ignore_ascii_case("results"));
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
        let text = String::from_utf8(bytes).map_err(|err| ResourceError::Internal(format!("UTF-8 decode failed: {err}")))?;
        return Ok(vec![ResourceContent { uri: trimmed.to_owned(), mime_type: String::from("application/json"), text }]);
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

fn handle_prompts_list(id: Value, params: &Value, state: &mut McpServerState) -> JsonRpcResponse {
    if !params.is_object() {
        return JsonRpcResponse::err(id, ERR_INVALID_PARAMS, "prompts/list params must be an object", None);
    }
    if let Err(payload) = state.sync_remote_control_policy() {
        return rpc_error_from_payload(id, payload);
    }
    JsonRpcResponse::ok(id, json!({ "prompts": list_prompts_for_mode(state.is_gui_only()) }))
}

fn handle_prompts_get(id: Value, params: &Value, state: &mut McpServerState) -> JsonRpcResponse {
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
    if let Err(payload) = state.sync_remote_control_policy() {
        return rpc_error_from_payload(id, payload);
    }

    match get_prompt_for_mode(name, &arguments, state.is_gui_only()) {
        Ok(result) => match serde_json::to_value(result) {
            Ok(val) => JsonRpcResponse::ok(id, val),
            Err(err) => JsonRpcResponse::err(id, ERR_INTERNAL_ERROR, format!("failed to encode prompt result: {err}"), None),
        },
        Err(msg) => JsonRpcResponse::err(id, ERR_INVALID_PARAMS, msg, None),
    }
}

fn read_next_frame<R: BufRead>(reader: &mut R, max_bytes: usize, fallback_mode: FramingMode) -> io::Result<NextFrame> {
    let line_cap = max_bytes.min(MAX_REQUEST_BYTES);
    let Some(first_line) = read_next_non_empty_line(reader, line_cap)? else {
        return Ok(NextFrame::Eof);
    };

    let first_line = match first_line {
        Ok(bytes) => bytes,
        Err(err_frame) => return Ok(err_frame),
    };

    if starts_json_payload(&first_line) {
        return Ok(NextFrame::Payload { bytes: first_line, mode: FramingMode::Newline });
    }

    if looks_like_header_line(&first_line) {
        return read_content_length_frame(reader, first_line, max_bytes);
    }

    Ok(NextFrame::Payload { bytes: first_line, mode: fallback_or_newline(fallback_mode) })
}

fn fallback_or_newline(last_mode: FramingMode) -> FramingMode {
    let _ = last_mode;
    FramingMode::Newline
}

fn read_next_non_empty_line<R: BufRead>(reader: &mut R, max_bytes: usize) -> io::Result<Option<Result<Vec<u8>, NextFrame>>> {
    loop {
        match read_line_bytes(reader, max_bytes)? {
            ReadLineOutcome::Eof(bytes) => {
                if is_ascii_blank(&bytes) {
                    return Ok(None);
                }
                return Ok(Some(Ok(bytes)));
            }
            ReadLineOutcome::TooLong { id } => {
                return Ok(Some(Err(NextFrame::RecoverableError {
                    id,
                    mode: FramingMode::Newline,
                    code: ERR_INVALID_REQUEST,
                    message: format!("line exceeds maximum request size of {max_bytes} bytes"),
                })));
            }
            ReadLineOutcome::DrainLimitExceeded => {
                return Ok(Some(Err(NextFrame::FatalError {
                    id: Value::Null,
                    mode: FramingMode::Newline,
                    code: ERR_INVALID_REQUEST,
                    message: format!("oversized newline frame exceeds maximum drain ceiling of {MAX_DRAIN_BYTES} bytes"),
                })));
            }
            ReadLineOutcome::Line(bytes) => {
                if !is_ascii_blank(&bytes) {
                    return Ok(Some(Ok(bytes)));
                }
            }
        }
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
        body.extend_from_slice(&avail[..take]);
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
            sniff_prefix.extend_from_slice(&avail[..keep]);
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
                    buf.extend_from_slice(&avail[..keep]);
                }
                reader.consume(take);
                return Ok(ReadLineOutcome::TooLong { id: extract_jsonrpc_id_prefix(&buf) });
            }
            buf.extend_from_slice(&avail[..take]);
            reader.consume(take);
            return Ok(ReadLineOutcome::Line(buf));
        }

        if buf.len().saturating_add(avail.len()) > max_bytes {
            let avail_len = avail.len();
            if buf.len() < sniff_cap {
                let keep = avail_len.min(sniff_cap - buf.len());
                buf.extend_from_slice(&avail[..keep]);
            }
            reader.consume(avail_len);
            if !drain_until_newline_bounded(reader, MAX_DRAIN_BYTES)? {
                return Ok(ReadLineOutcome::DrainLimitExceeded);
            }
            return Ok(ReadLineOutcome::TooLong { id: extract_jsonrpc_id_prefix(&buf) });
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
            match serde_json::from_slice::<Value>(&bytes[start..end]) {
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
    let s: String = serde_json::from_slice(&bytes[start..end]).ok()?;
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

fn starts_json_payload(bytes: &[u8]) -> bool {
    bytes.iter().copied().find(|b| !b.is_ascii_whitespace()).is_some_and(|b| b == b'{' || b == b'[')
}

fn looks_like_header_line(bytes: &[u8]) -> bool {
    let trimmed = trim_ascii_whitespace(bytes);
    let Some(colon_pos) = trimmed.iter().position(|&b| b == b':') else {
        return false;
    };
    if colon_pos == 0 {
        return false;
    }
    let name = &trimmed[..colon_pos];
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

fn trim_ascii_whitespace(bytes: &[u8]) -> &[u8] {
    let start = bytes.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(bytes.len());
    let end = bytes.iter().rposition(|b| !b.is_ascii_whitespace()).map_or(start, |idx| idx + 1);
    &bytes[start..end]
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
        assert_eq!(replies[0]["result"]["serverInfo"]["name"], json!("lumencraft"));
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
}
