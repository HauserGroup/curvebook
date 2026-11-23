//! Unified `FormatRegistry` adapters, `WorkdirGuard` path containment, atomic disk persistence,
//! figure rasterization/vector export, and `open_file`/`save_file`/`export`/`render_preview` handlers.
//!
//! # Filesystem Trust Boundary (`--workdir` vs. Unconstrained Mode)
//!
//! MCP file operations (`open_file`, `save_file`, `export`, engine command path parameters via
//! `sanitize_command_params`, and stored external data-link refreshes) operate in one of two
//! explicit path-policy modes configured on [`McpServerState`]:
//!
//! 1. **Unconstrained local-user mode (default when `--workdir` is omitted):** Intended for trusted
//!    local automation running with the invoking OS user's full privileges (including live-attached
//!    `--control-port` sessions). Relative and absolute host paths anywhere readable/writable by
//!    that OS user are accepted (still enforcing non-empty paths, NUL-byte rejection, regular-file
//!    checks, input size caps, refuse-newer archive guards, and crash-safe atomic writes via
//!    `curvebook_platform::fs::write_atomic`). Because an MCP client in unconstrained mode can
//!    atomically overwrite any user-writable destination file via `save_file` or `export`, both
//!    tools report `destructiveHint: true` and **only attach unconstrained MCP servers to trusted
//!    local agents**.
//! 2. **Sandboxed workdir mode (`--workdir <dir>` / [`McpServerState::with_workdir`]):** Applies
//!    strictly to headless and in-process `GuiSessionHost` modes (cannot be combined with
//!    `--control-port`, because `--workdir` cannot confine a live-attached desktop host). Enforces
//!    a hard containment boundary via `curvebook_platform::WorkdirGuard`. Every input, output,
//!    command parameter, and linked-file path must canonicalize strictly within `<dir>`; absolute
//!    paths outside `<dir>`, `..` parent-directory traversals, and symlinks resolving outside
//!    `<dir>` are rejected with `ErrorCode::PathEscape`. Untrusted or semi-trusted MCP clients
//!    must always be launched with `--workdir <dir>` (or `--gui-only`).

use std::path::{Path, PathBuf};

use curvebook_doc::{DataSheet, ErrorCode, ProjectDocument, SheetId, SubcolumnMode, TableKind};
use curvebook_engine::services::{
    ExportArtifactFormat, ExportArtifactRequest, ExportTarget, RasterMetadata, RasterSizingPolicy, compile_visual_scene_for_sheet,
    export_document_artifact,
};
use curvebook_engine::{EngineError, Session};
use curvebook_format::{
    CsvOptions, ExportOptions, ExportSource, Exporter, FormatRegistry, ImportOptions, ImportPayload, Importer, is_unpacked_project_dir,
    strip_project_data_for_template,
};
use curvebook_pdf::{PdfColorSpace, PdfExportOptions, PdfVersion};
use curvebook_render::{MAX_CAPPED_RASTER_DIM_PX, TiffCompression};
use curvebook_svg::SvgExportOptions;
use serde_json::{Map, Value, json};

#[cfg(not(target_arch = "wasm32"))]
use crate::errors::StructuredErrorPayload;
use crate::errors::{self};
use crate::gui_tools::reject_unknown_args;
#[cfg(not(target_arch = "wasm32"))]
use crate::tools::decode_base64;
use crate::tools::{
    ContentBlock, McpServerState, ToolCallError, ToolCallResult, encode_base64, optional_bool, optional_non_empty_str, optional_u32_bounded,
    required_non_empty_str,
};

const DEFAULT_PREVIEW_DPI: u16 = 144;
const MAX_DPI: u16 = 2400;
const MIN_DPI: u16 = 18;
#[cfg(not(target_arch = "wasm32"))]
const MAX_LIVE_INLINE_ARCHIVE_BYTES: usize = ((crate::tools::MAX_CONTROL_RESPONSE_BYTES - 65_536) / 4) * 3;
#[cfg(not(target_arch = "wasm32"))]
const MAX_LIVE_INLINE_CSV_BYTES: usize = 8 * 1024 * 1024;
#[cfg(not(target_arch = "wasm32"))]
const MAX_FONT_FILE_BYTES: usize = 64 * 1024 * 1024;

/// Resolve `raw_path` against the active filesystem trust boundary on `state`.
///
/// - When [`McpServerState::workdir_guard`] is configured (`--workdir <dir>`), `raw_path` is
///   confined strictly inside that canonical root via `WorkdirGuard::resolve`, returning
///   [`ErrorCode::PathEscape`] if `raw_path`, `..` components, or symlinks escape `<dir>`.
/// - When no `WorkdirGuard` is configured (including live `--control-port` sessions, which
///   cannot be combined with `--workdir`), `raw_path` may be any non-empty, NUL-free relative or
///   absolute path accessible to the current OS user; callers writing to the returned path (such as
///   `save_file` and `export`) atomically overwrite any user-writable destination file.
pub(crate) fn resolve_mcp_path(state: &McpServerState, raw_path: &str) -> Result<PathBuf, EngineError> {
    if raw_path.trim().is_empty() {
        return Err(EngineError::with_code(ErrorCode::BadInput, "path must not be empty"));
    }
    if raw_path.contains('\0') {
        return Err(EngineError::with_code(ErrorCode::BadInput, "path must not contain NUL bytes"));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(guard) = state.workdir_guard() {
            return guard.resolve(raw_path).map_err(EngineError::from);
        }
        Ok(PathBuf::from(raw_path))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = state;
        Err(EngineError::with_code(ErrorCode::IoError, "filesystem operations are not supported on wasm32"))
    }
}

/// Sanitize recursive path keys inside engine command JSON payloads when a `WorkdirGuard` is active.
///
/// Applies to headless and in-process `GuiSessionHost` modes (`--workdir` cannot be combined with
/// `--control-port`). Without `--workdir`, command path parameters remain unconstrained and execute
/// with the local user's filesystem permissions.
pub(crate) fn sanitize_command_params(state: &McpServerState, params: &mut Value) -> Result<(), EngineError> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(guard) = state.workdir_guard() {
        curvebook_platform::PathResolver::Sandboxed(guard.clone()).sanitize_params(params).map_err(EngineError::from)?;
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (state, params);
    Ok(())
}

/// Handle `open_file` dispatched via [`FormatRegistry`] with optional `WorkdirGuard` enforcement.
pub(crate) fn handle_open_file(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    reject_unknown_args("open_file", args, &["path", "format", "target_kind", "subcolumn_mode", "delimiter", "sheet_id", "title", "append"])?;
    let raw_path = required_non_empty_str(args, "path")?;
    let explicit_format = optional_non_empty_str(args, "format")?.map(|s| s.to_ascii_lowercase());
    let title_override = optional_non_empty_str(args, "title")?;
    let append = optional_bool(args, "append")?.unwrap_or(false);

    let resolved_path = match resolve_mcp_path(state, &raw_path) {
        Ok(p) => p,
        Err(err) => return Ok(errors::result_from_engine_error(&err)),
    };

    let reg = FormatRegistry::default_registry();
    let importer = resolve_importer(&reg, explicit_format.as_deref(), &resolved_path)?;
    let import_opts = build_import_options(&resolved_path, args)?;

    let format_id = importer.descriptor().id;
    if format_id == "curvebook.d" {
        let payload = match importer.import_path(&resolved_path, &import_opts) {
            Ok(p) => p,
            Err(err) => return Ok(errors::result_from_engine_error(&EngineError::from(err))),
        };
        return apply_imported_payload(state, &resolved_path, format_id, payload, None, title_override.as_deref(), &import_opts, append);
    }

    #[cfg(not(target_arch = "wasm32"))]
    let raw_bytes = match curvebook_platform::fs::read_regular_file_bytes(&resolved_path, importer.max_input_bytes()) {
        Ok(bytes) => bytes,
        Err(curvebook_platform::PlatformError::InvalidPath { reason }) => {
            return Ok(errors::result_from_engine_error(&EngineError::with_code(
                ErrorCode::BadInput,
                format!("invalid file `{}`: {reason}", resolved_path.display()),
            )));
        }
        Err(err) => return Ok(errors::result_from_engine_error(&EngineError::from(err))),
    };
    #[cfg(target_arch = "wasm32")]
    let raw_bytes: Vec<u8> = {
        return Ok(errors::result_from_code(ErrorCode::IoError, "filesystem operations are not supported on wasm32"));
    };

    let payload = match importer.import_bytes(&raw_bytes, &import_opts) {
        Ok(p) => p,
        Err(err) => return Ok(errors::result_from_engine_error(&EngineError::from(err))),
    };

    apply_imported_payload(state, &resolved_path, format_id, payload, Some(raw_bytes), title_override.as_deref(), &import_opts, append)
}

#[cfg(not(target_arch = "wasm32"))]
fn live_attach_import_error(payload: StructuredErrorPayload) -> ToolCallResult {
    errors::result_from_payload(&payload)
}

fn build_csv_import_params(
    resolved_path: &Path,
    format_id: &str,
    raw_bytes: Vec<u8>,
    import_opts: &ImportOptions,
    prefer_path: bool,
) -> Result<Value, ToolCallResult> {
    let csv_text = match String::from_utf8(raw_bytes) {
        Ok(text) => text,
        Err(err) => {
            return Err(errors::result_from_code(ErrorCode::IoError, format!("failed to decode '{}' as UTF-8: {err}", resolved_path.display())));
        }
    };
    let csv_opts = import_opts.csv.clone().unwrap_or_else(|| if format_id == "tsv" { CsvOptions::tsv() } else { CsvOptions::detect(&csv_text) });
    let subcolumn_mode = match serde_json::to_value(import_opts.subcolumn_mode) {
        Ok(v) => v,
        Err(err) => {
            return Err(errors::result_from_code(ErrorCode::InternalError, format!("failed to serialize subcolumn mode: {err}")));
        }
    };
    let mut map = Map::new();
    map.insert(String::from("sheet_id"), json!(import_opts.sheet_id.as_str()));
    if prefer_path {
        map.insert(String::from("path"), json!(resolved_path.display().to_string()));
    } else {
        map.insert(String::from("csv"), json!(csv_text));
    }
    map.insert(String::from("title"), json!(import_opts.title));
    map.insert(String::from("archetype"), json!(import_opts.table_kind.label()));
    map.insert(String::from("subcolumn_mode"), subcolumn_mode);
    map.insert(String::from("delimiter"), json!(csv_opts.delimiter.to_string()));
    map.insert(String::from("decimal_separator"), json!(csv_opts.decimal_separator.to_string()));
    Ok(Value::Object(map))
}

fn resolve_importer<'a>(reg: &'a FormatRegistry, explicit: Option<&str>, path: &Path) -> Result<&'a dyn Importer, ToolCallError> {
    if let Some(fmt) = explicit {
        return reg.importer_by_format(fmt).ok_or_else(|| ToolCallError::InvalidParams(format!("unsupported open format '{fmt}'")));
    }
    if let Some(found) = reg.importer_for_path(path) {
        return Ok(found);
    }
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("txt") => reg.importer_by_format("csv").ok_or_else(|| ToolCallError::InvalidParams(String::from("CSV importer unavailable"))),
        None | Some("") => Err(ToolCallError::InvalidParams(String::from("cannot infer file format without extension; specify \"format\""))),
        Some(other) => Err(ToolCallError::InvalidParams(format!("unsupported file extension '.{other}'"))),
    }
}

fn build_import_options(path: &Path, args: &Map<String, Value>) -> Result<ImportOptions, ToolCallError> {
    let target_kind = match optional_non_empty_str(args, "target_kind")? {
        Some(raw) => parse_table_kind(&raw)?,
        None => TableKind::Xy,
    };
    let subcolumn_mode = match optional_non_empty_str(args, "subcolumn_mode")? {
        Some(raw) => parse_subcolumn_mode(&raw)?,
        None => SubcolumnMode::Single,
    };

    let csv_override = if let Some(delim) = optional_non_empty_str(args, "delimiter")? {
        let mut chars = delim.chars();
        let Some(ch) = chars.next() else {
            return Err(ToolCallError::InvalidParams(String::from("delimiter must not be empty")));
        };
        if chars.next().is_some() {
            return Err(ToolCallError::InvalidParams(String::from("delimiter must be a single character")));
        }
        let mut opts = CsvOptions::csv();
        opts.delimiter = ch;
        Some(opts)
    } else {
        None
    };

    let stem = path.file_stem().and_then(|s| s.to_str()).filter(|s| !s.trim().is_empty()).unwrap_or("Imported Data");
    let sheet_title = optional_non_empty_str(args, "title")?.unwrap_or_else(|| stem.to_owned());
    let sheet_id_str = optional_non_empty_str(args, "sheet_id")?.unwrap_or_else(|| sanitize_sheet_id(stem, "data-1"));

    Ok(ImportOptions { sheet_id: SheetId::new(sheet_id_str), title: sheet_title, table_kind: target_kind, subcolumn_mode, csv: csv_override })
}

#[allow(clippy::too_many_arguments)]
fn apply_imported_payload(
    state: &mut McpServerState,
    resolved_path: &Path,
    format_id: &str,
    payload: ImportPayload,
    raw_bytes: Option<Vec<u8>>,
    title_override: Option<&str>,
    import_opts: &ImportOptions,
    append: bool,
) -> Result<ToolCallResult, ToolCallError> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(port) = state.control_port() {
        return apply_imported_payload_live(state, port, resolved_path, format_id, payload, raw_bytes, title_override, import_opts, append);
    }

    match (format_id, payload) {
        ("curvebook" | "curvebook.d", ImportPayload::Document(doc)) => {
            let sheets = doc.total_sheet_count();
            let title = doc.title.clone();
            state.install_session(Session::from_document(doc));
            state.session_mut().set_file_path(Some(resolved_path.to_path_buf()), true);
            ToolCallResult::ok_json(&json!({
                "opened": true,
                "format": format_id,
                "path": resolved_path.display().to_string(),
                "title": title,
                "total_sheet_count": sheets,
            }))
        }
        ("cbtemplate", ImportPayload::Document(raw_doc)) => {
            let mut doc = match strip_project_data_for_template(&raw_doc) {
                Ok(d) => d,
                Err(err) => return Ok(errors::result_from_engine_error(&EngineError::from(err))),
            };
            if let Some(new_title) = title_override {
                if let Err(err) = ProjectDocument::new(new_title.to_owned()) {
                    return Ok(errors::result_from_engine_error(&EngineError::from(err)));
                }
                doc.title = new_title.to_owned();
            }
            let sheets = doc.total_sheet_count();
            let title = doc.title.clone();
            state.install_session(Session::from_document(doc));
            state.session_mut().set_file_path(None, true);
            ToolCallResult::ok_json(&json!({
                "opened": true,
                "format": "cbtemplate",
                "path": resolved_path.display().to_string(),
                "title": title,
                "total_sheet_count": sheets,
            }))
        }
        ("prism" | "pzfx", ImportPayload::Report(report)) => {
            let Some(bytes) = raw_bytes else {
                return Ok(errors::result_from_code(ErrorCode::InternalError, format!("missing buffered bytes for '{format_id}'")));
            };
            let warnings = match serde_json::to_value(&report.warnings) {
                Ok(v) => v,
                Err(err) => {
                    return Ok(errors::result_from_code(ErrorCode::InternalError, format!("failed to serialize import warnings: {err}")));
                }
            };
            apply_report_import(state, resolved_path, format_id, &bytes, warnings, append)
        }
        ("csv" | "tsv", ImportPayload::DataSheet(_)) => {
            let Some(bytes) = raw_bytes else {
                return Ok(errors::result_from_code(ErrorCode::InternalError, format!("missing buffered bytes for '{format_id}'")));
            };
            apply_csv_import(state, resolved_path, format_id, bytes, import_opts, append)
        }
        _ => Ok(errors::result_from_code(ErrorCode::UnsupportedFormat, format!("unexpected payload returned by importer '{format_id}'"))),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn build_live_report_import_params(resolved_path: Option<&Path>, raw_bytes: Option<&[u8]>, append: bool) -> Result<Value, StructuredErrorPayload> {
    if let Some(path) = resolved_path {
        let path_str = path.display().to_string();
        if !path_str.trim().is_empty() {
            return Ok(json!({ "path": path_str, "merge": append }));
        }
    }
    let Some(bytes) = raw_bytes else {
        return Err(StructuredErrorPayload::new(ErrorCode::InternalError, "missing buffered bytes for live archive import"));
    };
    if bytes.len() > MAX_LIVE_INLINE_ARCHIVE_BYTES {
        return Err(StructuredErrorPayload::new(
            ErrorCode::ResourceLimit,
            format!(
                "inline live-control payload ({} bytes) exceeds maximum {MAX_LIVE_INLINE_ARCHIVE_BYTES} bytes before base64 encoding",
                bytes.len()
            ),
        )
        .with_remediation("Open the archive from a filesystem path accessible to the desktop host."));
    }
    Ok(json!({ "bytes_base64": encode_base64(bytes), "merge": append }))
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
fn apply_imported_payload_live(
    state: &mut McpServerState,
    port: u16,
    resolved_path: &Path,
    format_id: &str,
    payload: ImportPayload,
    raw_bytes: Option<Vec<u8>>,
    title_override: Option<&str>,
    import_opts: &ImportOptions,
    append: bool,
) -> Result<ToolCallResult, ToolCallError> {
    match (format_id, payload) {
        ("curvebook" | "curvebook.d", ImportPayload::Document(_)) => {
            let params = json!({
                "path": resolved_path.display().to_string(),
                "unpacked": format_id == "curvebook.d",
            });
            let res = match state.send_live_control("engine.execute", json!({ "command_id": "project.open", "params": params })) {
                Ok(v) => v,
                Err(err) => return Ok(live_attach_import_error(err)),
            };
            let title = res.pointer("/payload/title").and_then(Value::as_str).unwrap_or("");
            let sheet_count = res.pointer("/payload/sheet_count").and_then(Value::as_u64).unwrap_or(0);
            ToolCallResult::ok_json(&json!({
                "opened": true,
                "mode": "live_attach",
                "control_port": port,
                "format": format_id,
                "path": resolved_path.display().to_string(),
                "title": title,
                "total_sheet_count": sheet_count,
            }))
        }
        ("cbtemplate", ImportPayload::Document(_)) => {
            let mut params = Map::new();
            params.insert(String::from("path"), json!(resolved_path.display().to_string()));
            if let Some(title) = title_override {
                params.insert(String::from("title"), json!(title));
            }
            let res = match state
                .send_live_control("engine.execute", json!({ "command_id": "project.instantiateTemplate", "params": Value::Object(params) }))
            {
                Ok(v) => v,
                Err(err) => return Ok(live_attach_import_error(err)),
            };
            let title = res.pointer("/payload/title").and_then(Value::as_str).unwrap_or("");
            let sheet_count = res.pointer("/payload/sheet_count").and_then(Value::as_u64).unwrap_or(0);
            ToolCallResult::ok_json(&json!({
                "opened": true,
                "mode": "live_attach",
                "control_port": port,
                "format": "cbtemplate",
                "path": resolved_path.display().to_string(),
                "title": title,
                "total_sheet_count": sheet_count,
            }))
        }
        ("prism" | "pzfx", ImportPayload::Report(report)) => {
            let warnings = match serde_json::to_value(&report.warnings) {
                Ok(v) => v,
                Err(err) => {
                    return Ok(errors::result_from_code(ErrorCode::InternalError, format!("failed to serialize import warnings: {err}")));
                }
            };
            let cmd_id = if format_id == "prism" { "table.importPrism" } else { "table.importPzfx" };
            let import_params = match build_live_report_import_params(Some(resolved_path), raw_bytes.as_deref(), append) {
                Ok(p) => p,
                Err(err) => return Ok(live_attach_import_error(err)),
            };
            let send_res = if append {
                state.send_live_control("engine.execute", json!({ "command_id": cmd_id, "params": import_params }))
            } else {
                state.send_live_control(
                    "command.batch",
                    json!({
                        "atomic": true,
                        "commands": [
                            { "command": "file.new", "params": {} },
                            { "command": cmd_id, "params": import_params },
                        ]
                    }),
                )
            };
            if let Err(err) = send_res {
                return Ok(live_attach_import_error(err));
            }
            let snap = state.send_live_control("doc.inspect", json!({})).ok();
            let title = snap.as_ref().and_then(|v| v.get("title")).and_then(Value::as_str).unwrap_or("");
            let sheet_count = snap.as_ref().and_then(|v| v.pointer("/counts/total")).and_then(Value::as_u64).unwrap_or(0);
            ToolCallResult::ok_json(&json!({
                "opened": true,
                "mode": "live_attach",
                "control_port": port,
                "format": format_id,
                "path": resolved_path.display().to_string(),
                "title": title,
                "total_sheet_count": sheet_count,
                "warnings": warnings,
            }))
        }
        ("csv" | "tsv", ImportPayload::DataSheet(_)) => {
            let Some(bytes) = raw_bytes else {
                return Ok(errors::result_from_code(ErrorCode::InternalError, format!("missing buffered bytes for '{format_id}'")));
            };
            let prefer_path = bytes.len() > MAX_LIVE_INLINE_CSV_BYTES;
            let params = match build_csv_import_params(resolved_path, format_id, bytes, import_opts, prefer_path) {
                Ok(p) => p,
                Err(err_res) => return Ok(err_res),
            };
            let send_res = if append {
                state.send_live_control("engine.execute", json!({ "command_id": "table.importCsv", "params": params }))
            } else {
                state.send_live_control(
                    "command.batch",
                    json!({
                        "atomic": true,
                        "commands": [
                            { "command": "file.new", "params": { "title": import_opts.title } },
                            { "command": "table.importCsv", "params": params },
                        ]
                    }),
                )
            };
            if let Err(err) = send_res {
                return Ok(live_attach_import_error(err));
            }
            let inspect = match state.send_live_control("doc.inspect", json!({ "sheet_id": import_opts.sheet_id.as_str() })) {
                Ok(v) => v,
                Err(err) => return Ok(live_attach_import_error(err)),
            };
            ToolCallResult::ok_json(&json!({
                "opened": true,
                "mode": "live_attach",
                "control_port": port,
                "format": format_id,
                "path": resolved_path.display().to_string(),
                "sheet_id": import_opts.sheet_id.as_str(),
                "title": inspect.get("title").and_then(Value::as_str).unwrap_or(&import_opts.title),
                "table_kind": inspect.get("kind").and_then(Value::as_str).unwrap_or(import_opts.table_kind.label()),
                "rows": inspect.get("rows").and_then(Value::as_u64).unwrap_or(0),
                "columns": inspect.get("cols").and_then(Value::as_u64).unwrap_or(0),
            }))
        }
        _ => Ok(errors::result_from_code(ErrorCode::UnsupportedFormat, format!("unexpected payload returned by importer '{format_id}'"))),
    }
}

fn apply_report_import(
    state: &mut McpServerState,
    resolved_path: &Path,
    format_id: &str,
    bytes: &[u8],
    warnings: Value,
    append: bool,
) -> Result<ToolCallResult, ToolCallError> {
    let command_id = if format_id == "prism" { "table.importPrism" } else { "table.importPzfx" };
    let params = json!({
        "bytes_base64": encode_base64(bytes),
        "merge": append,
    });

    let cancel = state.current_cancel_token();
    if append {
        return state.mutate_active_session(|session| {
            if let Err(err) = session.execute_with_caller(command_id, params, curvebook_engine::Caller::Mcp, &cancel) {
                return Ok(errors::result_from_engine_error(&err));
            }
            let sheets = session.document().total_sheet_count();
            let title = session.document().title.clone();
            ToolCallResult::ok_json(&json!({
                "opened": true,
                "format": format_id,
                "path": resolved_path.display().to_string(),
                "title": title,
                "total_sheet_count": sheets,
                "warnings": warnings,
            }))
        });
    }

    let mut next_session = Session::new();
    if let Err(err) = next_session.execute_with_caller(command_id, params, curvebook_engine::Caller::Mcp, &cancel) {
        return Ok(errors::result_from_engine_error(&err));
    }

    let sheets = next_session.document().total_sheet_count();
    let title = next_session.document().title.clone();
    state.install_session(next_session);
    ToolCallResult::ok_json(&json!({
        "opened": true,
        "format": format_id,
        "path": resolved_path.display().to_string(),
        "title": title,
        "total_sheet_count": sheets,
        "warnings": warnings,
    }))
}

fn apply_csv_import(
    state: &mut McpServerState,
    resolved_path: &Path,
    format_id: &str,
    raw_bytes: Vec<u8>,
    import_opts: &ImportOptions,
    append: bool,
) -> Result<ToolCallResult, ToolCallError> {
    if append && state.session().document().sheet_section(&import_opts.sheet_id).is_some() {
        return Ok(errors::result_from_engine_error(&EngineError::from(curvebook_doc::DocError::DuplicateSheetId {
            sheet_id: import_opts.sheet_id.clone(),
        })));
    }

    let params = match build_csv_import_params(resolved_path, format_id, raw_bytes, import_opts, false) {
        Ok(p) => p,
        Err(err_res) => return Ok(err_res),
    };

    let cancel = state.current_cancel_token();
    if append {
        return state.mutate_active_session(|session| {
            execute_csv_import_on_session(session, format_id, resolved_path, &import_opts.sheet_id, params, &cancel)
        });
    }

    let doc = match ProjectDocument::new(import_opts.title.clone()) {
        Ok(d) => d,
        Err(err) => return Ok(errors::result_from_engine_error(&EngineError::from(err))),
    };
    let mut next_session = Session::from_document(doc);
    let result = execute_csv_import_on_session(&mut next_session, format_id, resolved_path, &import_opts.sheet_id, params, &cancel)?;
    if !result.is_error {
        state.install_session(next_session);
    }
    Ok(result)
}

fn execute_csv_import_on_session(
    session: &mut Session,
    format_id: &str,
    resolved_path: &Path,
    sheet_id: &SheetId,
    params: Value,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<ToolCallResult, ToolCallError> {
    if let Err(err) = session.execute_with_caller("table.importCsv", params, curvebook_engine::Caller::Mcp, cancel) {
        return Ok(errors::result_from_engine_error(&err));
    }
    let Some(ds) = session.document().data_sheet(sheet_id) else {
        return Ok(errors::result_from_code(ErrorCode::InternalError, format!("imported sheet '{}' missing after import", sheet_id.as_str())));
    };
    sheet_summary_json(format_id, resolved_path, ds)
}

fn sheet_summary_json(format_id: &str, resolved_path: &Path, ds: &DataSheet) -> Result<ToolCallResult, ToolCallError> {
    ToolCallResult::ok_json(&json!({
        "opened": true,
        "format": format_id,
        "path": resolved_path.display().to_string(),
        "sheet_id": ds.id.as_str(),
        "title": ds.title,
        "table_kind": ds.kind().label(),
        "rows": ds.archetype.row_count(),
        "columns": ds.archetype.column_count(),
    }))
}

/// Handle `save_file` dispatched via [`FormatRegistry`] (which persists atomically via `write_atomic`).
pub(crate) fn handle_save_file(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    reject_unknown_args("save_file", args, &["path", "format"])?;
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(port) = state.control_port() {
        let raw_path = match optional_non_empty_str(args, "path")? {
            Some(p) => p,
            None => {
                let snap = match state.send_live_control("doc.inspect", json!({})) {
                    Ok(v) => v,
                    Err(err) => return Ok(errors::result_from_payload(&err)),
                };
                match snap.get("file_path").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
                    Some(p) => p.to_owned(),
                    None => {
                        return Err(ToolCallError::InvalidParams(String::from(
                            "save_file requires \"path\" when live session has no associated file_path",
                        )));
                    }
                }
            }
        };
        let resolved_path = match resolve_mcp_path(state, &raw_path) {
            Ok(p) => p,
            Err(err) => return Ok(errors::result_from_engine_error(&err)),
        };
        let explicit_format = optional_non_empty_str(args, "format")?.map(|s| s.to_ascii_lowercase());
        let reg = FormatRegistry::default_registry();
        let exporter = resolve_save_exporter(&reg, explicit_format.as_deref(), &resolved_path)?;
        let format_id = exporter.descriptor().id;
        let cmd_id = if format_id == "cbtemplate" { "project.saveTemplate" } else { "project.save" };
        let mut cmd_params = json!({
            "path": resolved_path.display().to_string(),
            "return_bytes": false,
        });
        if format_id == "curvebook.d"
            && let Some(obj) = cmd_params.as_object_mut()
        {
            obj.insert(String::from("unpacked"), json!(true));
        }
        if let Err(err) = state.send_live_control("engine.execute", json!({ "command_id": cmd_id, "params": cmd_params })) {
            return Ok(errors::result_from_payload(&err));
        }
        let snap = state.send_live_control("doc.inspect", json!({})).ok();
        let schema_version = snap.as_ref().and_then(|v| v.get("schema_version")).and_then(Value::as_u64).unwrap_or(0);
        let total_sheet_count = snap.as_ref().and_then(|v| v.pointer("/counts/total")).and_then(Value::as_u64).unwrap_or(0);
        return ToolCallResult::ok_json(&json!({
            "saved": true,
            "mode": "live_attach",
            "control_port": port,
            "format": format_id,
            "path": resolved_path.display().to_string(),
            "schema_version": schema_version,
            "total_sheet_count": total_sheet_count,
        }));
    }

    let raw_path = match optional_non_empty_str(args, "path")? {
        Some(p) => p,
        None => match state.session().file_path() {
            Some(existing) => existing.display().to_string(),
            None => {
                return Err(ToolCallError::InvalidParams(String::from("save_file requires \"path\" when session has no associated file_path")));
            }
        },
    };
    let resolved_path = match resolve_mcp_path(state, &raw_path) {
        Ok(p) => p,
        Err(err) => return Ok(errors::result_from_engine_error(&err)),
    };
    let explicit_format = optional_non_empty_str(args, "format")?.map(|s| s.to_ascii_lowercase());
    let reg = FormatRegistry::default_registry();
    let exporter = resolve_save_exporter(&reg, explicit_format.as_deref(), &resolved_path)?;
    let format_id = exporter.descriptor().id;

    let doc = state.session().document().clone();
    if let Err(err) = exporter.export_path(ExportSource::Document(&doc), &resolved_path, &ExportOptions::default()) {
        let engine_err = EngineError::from(err);
        return Ok(errors::result_from_engine_error(&engine_err));
    }

    if matches!(format_id, "curvebook" | "curvebook.d") {
        state.mutate_active_session(|session| session.set_file_path(Some(resolved_path.clone()), true));
    }

    ToolCallResult::ok_json(&json!({
        "saved": true,
        "format": format_id,
        "path": resolved_path.display().to_string(),
        "schema_version": doc.schema_version,
        "total_sheet_count": doc.total_sheet_count(),
    }))
}

fn has_curvebook_unpacked_suffix(path: &Path) -> bool {
    path.file_name().and_then(std::ffi::OsStr::to_str).is_some_and(|name| name.to_ascii_lowercase().ends_with(".curvebook.d"))
}

fn resolve_save_exporter<'a>(reg: &'a FormatRegistry, explicit: Option<&str>, path: &Path) -> Result<&'a dyn Exporter, ToolCallError> {
    if let Some(fmt) = explicit {
        if !matches!(fmt, "curvebook" | "curvebook.d" | "cbtemplate") {
            return Err(ToolCallError::InvalidParams(format!("unsupported save format '{fmt}'")));
        }
        return reg.exporter_by_format(fmt).ok_or_else(|| ToolCallError::InvalidParams(format!("unsupported save format '{fmt}'")));
    }
    if !has_curvebook_unpacked_suffix(path) && !is_unpacked_project_dir(path) && path.is_dir() {
        return Err(ToolCallError::InvalidParams(format!("save_file target `{}` is a directory without a `.curvebook.d` suffix", path.display())));
    }
    if let Some(found) = reg.exporter_for_path(path)
        && matches!(found.descriptor().id, "curvebook" | "curvebook.d" | "cbtemplate")
        && (found.descriptor().id != "curvebook.d" || has_curvebook_unpacked_suffix(path) || is_unpacked_project_dir(path))
    {
        return Ok(found);
    }
    reg.exporter_by_format("curvebook").ok_or_else(|| ToolCallError::InvalidParams(String::from("curvebook exporter unavailable")))
}

/// Handle `render_preview`: compile GraphSheet/LayoutSheet and rasterize PNG preview with metadata.
pub(crate) fn handle_render_preview(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    reject_unknown_args("render_preview", args, &["sheet_id", "width_px", "height_px", "dpi"])?;
    let sheet_id = optional_non_empty_str(args, "sheet_id")?;
    let dpi = parse_dpi(args)?;
    let requested_w = optional_u32_bounded(args, "width_px", 1, MAX_CAPPED_RASTER_DIM_PX)?;
    let requested_h = optional_u32_bounded(args, "height_px", 1, MAX_CAPPED_RASTER_DIM_PX)?;

    #[cfg(not(target_arch = "wasm32"))]
    if let Some(port) = state.control_port() {
        let mut remote = match state.send_live_control(
            "render.preview",
            json!({
                "sheet_id": sheet_id,
                "dpi": dpi,
                "width_px": requested_w,
                "height_px": requested_h,
            }),
        ) {
            Ok(v) => v,
            Err(err) => return Ok(errors::result_from_payload(&err)),
        };
        let Some(b64) = remote.get("data_base64").and_then(Value::as_str).map(str::to_owned) else {
            return Ok(errors::result_from_code(ErrorCode::InternalError, "live render.preview response missing data_base64"));
        };
        if let Some(obj) = remote.as_object_mut() {
            obj.remove("data_base64");
            obj.remove("encoding");
            obj.insert(String::from("mode"), Value::String(String::from("live_attach")));
            obj.insert(String::from("control_port"), json!(port));
        }
        let metadata_text = serde_json::to_string_pretty(&remote).unwrap_or_default();
        return Ok(ToolCallResult {
            content: vec![ContentBlock::Image { data: b64, mime_type: String::from("image/png") }, ContentBlock::Text { text: metadata_text }],
            is_error: false,
            structured_content: Some(remote),
        });
    }

    let doc = state.session().document();
    let (target_id, sheet_kind) = match resolve_preview_sheet_id(doc, sheet_id.as_deref()) {
        Ok(found) => found,
        Err(err) => return Ok(errors::result_from_engine_error(&err)),
    };
    let scene = match compile_visual_scene_for_sheet(doc, &target_id) {
        Ok(scene) => scene,
        Err(err) => return Ok(errors::result_from_engine_error(&err)),
    };
    let cancel = state.current_cancel_token();
    let artifact = match export_document_artifact(
        doc,
        &ExportArtifactRequest {
            target: ExportTarget::Explicit(target_id.clone()),
            format: ExportArtifactFormat::Png {
                width_px: requested_w,
                height_px: requested_h,
                dpi: Some(dpi),
                sizing: RasterSizingPolicy::CapToLimits { default_dpi: DEFAULT_PREVIEW_DPI },
            },
        },
        Some(&cancel),
        "render_preview",
    ) {
        Ok(art) => art,
        Err(err) => return Ok(errors::result_from_engine_error(&err)),
    };
    let raster = artifact.raster.unwrap_or(RasterMetadata { width_px: 0, height_px: 0, dpi });

    let metadata = json!({
        "sheet_id": target_id.as_str(),
        "sheet_kind": sheet_kind,
        "title": scene.title,
        "width_px": raster.width_px,
        "height_px": raster.height_px,
        "dpi": raster.dpi,
        "width_pt": scene.width_pt,
        "height_pt": scene.height_pt,
        "png_bytes": artifact.bytes.len(),
        "primitives_count": scene.primitives.len(),
        "warnings": scene.warnings,
    });
    let metadata_text = serde_json::to_string_pretty(&metadata).unwrap_or_default();

    Ok(ToolCallResult {
        content: vec![
            ContentBlock::Image { data: encode_base64(&artifact.bytes), mime_type: String::from("image/png") },
            ContentBlock::Text { text: metadata_text },
        ],
        is_error: false,
        structured_content: Some(metadata),
    })
}

/// Handle `export` delegated to [`export_document_artifact`] with `WorkdirGuard` path checks
/// and atomic writes (`write_atomic`).
pub(crate) fn handle_export(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    reject_unknown_args(
        "export",
        args,
        &[
            "format",
            "sheet_id",
            "output_path",
            "path",
            "width_px",
            "height_px",
            "dpi",
            "tiff_compression",
            "pdf_color_space",
            "pdf_version",
            "ttf_font",
            "all_graphs",
            "svg_precision",
            "pretty",
            "tidy",
            "tab_id",
            "csv_delimiter",
        ],
    )?;
    let format = required_non_empty_str(args, "format")?.to_ascii_lowercase();
    if !matches!(format.as_str(), "svg" | "pdf" | "emf" | "png" | "tiff" | "csv" | "tsv" | "json") {
        return Err(ToolCallError::InvalidParams(format!("unsupported export format '{format}'")));
    }

    let sheet_id = optional_non_empty_str(args, "sheet_id")?;
    let output_path = optional_non_empty_str(args, "output_path")?.or(optional_non_empty_str(args, "path")?);
    let cfg = ExportArtifactConfig::parse(args, sheet_id)?;

    let resolved_path = match output_path.as_deref() {
        Some(raw) => match resolve_mcp_path(state, raw) {
            Ok(p) => Some(p),
            Err(err) => return Ok(errors::result_from_engine_error(&err)),
        },
        None => None,
    };

    let resolved_ttf_font = match cfg.ttf_font.as_deref() {
        Some(raw) => match resolve_mcp_path(state, raw) {
            Ok(p) => Some(p),
            Err(err) => return Ok(errors::result_from_engine_error(&err)),
        },
        None => None,
    };

    #[cfg(not(target_arch = "wasm32"))]
    if let Some(port) = state.control_port() {
        let pdf_cs = match cfg.pdf_color_space {
            PdfColorSpace::Srgb => "srgb",
            PdfColorSpace::Cmyk => "cmyk",
            PdfColorSpace::Grayscale => "grayscale",
        };
        let pdf_ver = pdf_version_str(cfg.pdf_version);
        let tiff_comp = match cfg.tiff_compression {
            TiffCompression::None => "none",
            TiffCompression::Deflate => "deflate",
            TiffCompression::Lzw => "lzw",
        };
        let mut live_params = json!({
            "format": format,
            "sheet_id": cfg.sheet_id,
            "pretty": cfg.pretty,
            "tidy": cfg.tidy,
            "tab_id": cfg.tab_id,
            "csv_delimiter": cfg.csv_opts.as_ref().map(|c| c.delimiter.to_string()),
            "svg_precision": cfg.svg_precision,
            "pdf_color_space": pdf_cs,
            "pdf_version": pdf_ver,
            "ttf_font": resolved_ttf_font.as_ref().map(|p| p.display().to_string()),
            "all_graphs": cfg.all_graphs,
            "dpi": cfg.dpi,
            "width_px": cfg.width_px,
            "height_px": cfg.height_px,
            "tiff_compression": tiff_comp,
        });
        if let Some(dest) = resolved_path.as_ref()
            && let Some(obj) = live_params.as_object_mut()
        {
            obj.insert(String::from("path"), json!(dest.display().to_string()));
        }
        let remote = match state.send_live_control("doc.export", live_params) {
            Ok(v) => v,
            Err(err) => return Ok(errors::result_from_payload(&err)),
        };
        let target_id = remote.get("sheet_id").and_then(Value::as_str).unwrap_or("project").to_owned();
        if let Some(dest) = resolved_path {
            if remote.get("exported").and_then(Value::as_bool) == Some(true) {
                let written_path = remote.get("path").and_then(Value::as_str).map(ToOwned::to_owned).unwrap_or_else(|| dest.display().to_string());
                let bytes_written = remote.get("bytes_written").or_else(|| remote.get("bytes_len")).and_then(Value::as_u64).unwrap_or(0);
                return ToolCallResult::ok_json(&json!({
                    "exported": true,
                    "mode": "live_attach",
                    "control_port": port,
                    "format": format,
                    "sheet_id": target_id,
                    "path": written_path,
                    "bytes_written": bytes_written,
                }));
            }
            let Some(b64) = remote.get("data_base64").and_then(Value::as_str) else {
                return Ok(errors::result_from_code(
                    ErrorCode::InternalError,
                    "live doc.export response missing exported confirmation or data_base64",
                ));
            };
            let bytes = match decode_base64(b64) {
                Ok(b) => b,
                Err(err) => return Ok(errors::result_from_code(ErrorCode::InternalError, format!("invalid base64 from live doc.export: {err}"))),
            };
            return write_atomic_export(&dest, &format, &target_id, &bytes);
        }
        let Some(b64) = remote.get("data_base64").and_then(Value::as_str) else {
            return Ok(errors::result_from_code(ErrorCode::InternalError, "live doc.export response missing data_base64"));
        };
        let bytes = match decode_base64(b64) {
            Ok(b) => b,
            Err(err) => return Ok(errors::result_from_code(ErrorCode::InternalError, format!("invalid base64 from live doc.export: {err}"))),
        };
        let raster_meta = match (remote.get("width_px").and_then(Value::as_u64), remote.get("height_px").and_then(Value::as_u64)) {
            (Some(w), Some(h)) => {
                let w32 = u32::try_from(w).unwrap_or(0);
                let h32 = u32::try_from(h).unwrap_or(0);
                let dpi_u16 = remote.get("dpi").and_then(Value::as_u64).and_then(|d| u16::try_from(d).ok()).unwrap_or(cfg.dpi);
                Some(RasterMetadata { width_px: w32, height_px: h32, dpi: dpi_u16 })
            }
            _ => None,
        };
        return format_inline_export_result(&format, &target_id, &bytes, raster_meta);
    }

    let embedded_ttf_font = if format == "pdf" {
        match resolved_ttf_font.as_deref() {
            Some(font_path) => match load_validated_ttf_font_bytes(font_path) {
                Ok(bytes) => Some(bytes),
                Err(err) => return Ok(errors::result_from_engine_error(&err)),
            },
            None => None,
        }
    } else {
        None
    };

    let doc = state.session().document();
    let req = match build_export_artifact_request(doc, &format, &cfg, embedded_ttf_font) {
        Ok(req) => req,
        Err(err) => return Ok(errors::result_from_engine_error(&err)),
    };
    let cancel = state.current_cancel_token();
    let artifact = match export_document_artifact(doc, &req, Some(&cancel), "export") {
        Ok(art) => art,
        Err(err) => return Ok(errors::result_from_engine_error(&err)),
    };

    if let Some(dest) = resolved_path {
        return write_atomic_export(&dest, &format, &artifact.target_id, &artifact.bytes);
    }

    format_inline_export_result(&format, &artifact.target_id, &artifact.bytes, artifact.raster)
}

fn write_atomic_export(dest: &Path, format: &str, target_id: &str, bytes: &[u8]) -> Result<ToolCallResult, ToolCallError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(parent) = dest.parent()
            && !parent.as_os_str().is_empty()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            return Ok(errors::result_from_payload(
                &StructuredErrorPayload::new(ErrorCode::IoError, format!("failed to prepare directory for '{}': {err}", dest.display()))
                    .with_retryable(true),
            ));
        }
        if let Err(err) = curvebook_platform::fs::write_atomic(dest, bytes) {
            return Ok(errors::result_from_engine_error(&EngineError::from(err)));
        }
        ToolCallResult::ok_json(&json!({
            "exported": true,
            "format": format,
            "sheet_id": target_id,
            "path": dest.display().to_string(),
            "bytes_written": bytes.len(),
        }))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (dest, format, target_id, bytes);
        Ok(errors::result_from_code(ErrorCode::IoError, "filesystem export is not supported on wasm32"))
    }
}

fn format_inline_export_result(
    format: &str,
    target_id: &str,
    bytes: &[u8],
    raster_meta: Option<RasterMetadata>,
) -> Result<ToolCallResult, ToolCallError> {
    match format {
        "svg" | "csv" | "tsv" => {
            let text = String::from_utf8(bytes.to_vec()).map_err(|err| ToolCallError::Execution(format!("UTF-8 export decode failed: {err}")))?;
            Ok(ToolCallResult::ok_text(text))
        }
        "json" => {
            let text = String::from_utf8(bytes.to_vec()).map_err(|err| ToolCallError::Execution(format!("UTF-8 export decode failed: {err}")))?;
            let structured = serde_json::from_str::<Value>(&text).ok();
            Ok(ToolCallResult { content: vec![ContentBlock::Text { text }], is_error: false, structured_content: structured })
        }
        "png" => {
            let raster = raster_meta.unwrap_or(RasterMetadata { width_px: 0, height_px: 0, dpi: DEFAULT_PREVIEW_DPI });
            let meta = json!({
                "format": "png",
                "sheet_id": target_id,
                "width_px": raster.width_px,
                "height_px": raster.height_px,
                "dpi": raster.dpi,
                "bytes_len": bytes.len(),
            });
            let text = serde_json::to_string_pretty(&meta).unwrap_or_default();
            Ok(ToolCallResult {
                content: vec![ContentBlock::Image { data: encode_base64(bytes), mime_type: String::from("image/png") }, ContentBlock::Text { text }],
                is_error: false,
                structured_content: Some(meta),
            })
        }
        "pdf" | "emf" => ToolCallResult::ok_json(&json!({
            "format": format,
            "sheet_id": target_id,
            "encoding": "base64",
            "bytes_len": bytes.len(),
            "data": encode_base64(bytes),
        })),
        "tiff" => {
            let raster = raster_meta.unwrap_or(RasterMetadata { width_px: 0, height_px: 0, dpi: DEFAULT_PREVIEW_DPI });
            ToolCallResult::ok_json(&json!({
                "format": "tiff",
                "sheet_id": target_id,
                "encoding": "base64",
                "width_px": raster.width_px,
                "height_px": raster.height_px,
                "dpi": raster.dpi,
                "bytes_len": bytes.len(),
                "data": encode_base64(bytes),
            }))
        }
        _ => Err(ToolCallError::InvalidParams(format!("unsupported export format '{format}'"))),
    }
}

#[derive(Clone)]
struct ExportArtifactConfig {
    sheet_id: Option<String>,
    pretty: bool,
    tidy: bool,
    tab_id: Option<String>,
    csv_opts: Option<CsvOptions>,
    svg_precision: Option<u8>,
    pdf_color_space: PdfColorSpace,
    pdf_version: PdfVersion,
    ttf_font: Option<String>,
    all_graphs: bool,
    width_px: Option<u32>,
    height_px: Option<u32>,
    dpi: u16,
    tiff_compression: TiffCompression,
}

impl ExportArtifactConfig {
    fn parse(args: &Map<String, Value>, sheet_id: Option<String>) -> Result<Self, ToolCallError> {
        let pretty = optional_bool(args, "pretty")?.unwrap_or(true);
        let explicit_tidy = optional_bool(args, "tidy")?;
        let tab_id = optional_non_empty_str(args, "tab_id")?;
        if explicit_tidy == Some(true) && tab_id.is_some() {
            return Err(ToolCallError::InvalidParams(String::from(
                "export `tidy: true` cannot be combined with `tab_id`; omit `tidy` or pass `tidy: false` to export a specific tab",
            )));
        }
        let tidy = explicit_tidy.unwrap_or(tab_id.is_none());
        let csv_opts = match optional_non_empty_str(args, "csv_delimiter")? {
            Some(delim) => {
                let mut chars = delim.chars();
                let Some(ch) = chars.next() else {
                    return Err(ToolCallError::InvalidParams(String::from("csv_delimiter must not be empty")));
                };
                if chars.next().is_some() {
                    return Err(ToolCallError::InvalidParams(String::from("csv_delimiter must be a single character")));
                }
                let mut opts = CsvOptions::csv();
                opts.delimiter = ch;
                Some(opts)
            }
            None => None,
        };
        let svg_precision = match optional_u32_bounded(args, "svg_precision", 1, 8)? {
            Some(p) => Some(u8::try_from(p).map_err(|_| ToolCallError::InvalidParams(String::from("svg_precision out of bounds")))?),
            None => None,
        };
        let pdf_color_space = match optional_non_empty_str(args, "pdf_color_space")? {
            Some(raw) => parse_pdf_color_space(&raw)?,
            None => PdfColorSpace::Srgb,
        };
        let pdf_version = match optional_non_empty_str(args, "pdf_version")? {
            Some(raw) => parse_pdf_version(&raw)?,
            None => PdfVersion::Pdf17,
        };
        let ttf_font = optional_non_empty_str(args, "ttf_font")?;
        let all_graphs = optional_bool(args, "all_graphs")?.unwrap_or(false);
        let dpi = parse_dpi(args)?;
        let width_px = optional_u32_bounded(args, "width_px", 1, MAX_CAPPED_RASTER_DIM_PX)?;
        let height_px = optional_u32_bounded(args, "height_px", 1, MAX_CAPPED_RASTER_DIM_PX)?;
        let tiff_compression = match optional_non_empty_str(args, "tiff_compression")? {
            Some(raw) => parse_tiff_compression(&raw)?,
            None => TiffCompression::Lzw,
        };

        Ok(Self {
            sheet_id,
            pretty,
            tidy,
            tab_id,
            csv_opts,
            svg_precision,
            pdf_color_space,
            pdf_version,
            ttf_font,
            all_graphs,
            width_px,
            height_px,
            dpi,
            tiff_compression,
        })
    }
}

fn resolve_preview_sheet_id(doc: &ProjectDocument, sheet_id: Option<&str>) -> Result<(SheetId, &'static str), EngineError> {
    if let Some(id_str) = sheet_id {
        let id = SheetId::new(id_str);
        if doc.graph_sheet(&id).is_some() {
            return Ok((id, "graph"));
        }
        if doc.layout_sheet(&id).is_some() {
            return Ok((id, "layout"));
        }
        return Err(EngineError::with_code(ErrorCode::NotFound, format!("graph or layout sheet '{id_str}' not found")));
    }
    if let Some(active) = &doc.active_sheet_id {
        if doc.graph_sheet(active).is_some() {
            return Ok((active.clone(), "graph"));
        }
        if doc.layout_sheet(active).is_some() {
            return Ok((active.clone(), "layout"));
        }
    }
    if let Some(gs) = doc.graph_sheets.first() {
        return Ok((gs.id.clone(), "graph"));
    }
    if let Some(ls) = doc.layout_sheets.first() {
        return Ok((ls.id.clone(), "layout"));
    }
    Err(EngineError::with_code(ErrorCode::NotFound, "no GraphSheet or LayoutSheet exists in the current document"))
}

fn resolve_mcp_csv_target(doc: &ProjectDocument, sheet_id: Option<&str>, tab_id: Option<&str>) -> Result<ExportTarget, EngineError> {
    if let Some(id_str) = sheet_id {
        return Ok(ExportTarget::Explicit(SheetId::new(id_str)));
    }
    if let Some(tid) = tab_id {
        if let Some(active) = &doc.active_sheet_id
            && let Some(rs) = doc.result_sheet(active)
            && rs.tabs.iter().any(|t| t.id == tid)
        {
            return Ok(ExportTarget::Explicit(rs.id.clone()));
        }
        if let Some(rs) = doc.result_sheets.iter().find(|rs| rs.tabs.iter().any(|t| t.id == tid)) {
            return Ok(ExportTarget::Explicit(rs.id.clone()));
        }
        return Err(EngineError::with_code(ErrorCode::NotFound, format!("tab '{tid}' not found on any result sheet")));
    }
    if let Some(active) = &doc.active_sheet_id
        && (doc.result_sheet(active).is_some() || doc.data_sheet(active).is_some())
    {
        return Ok(ExportTarget::Explicit(active.clone()));
    }
    if let Some(rs) = doc.result_sheets.first() {
        return Ok(ExportTarget::Explicit(rs.id.clone()));
    }
    Ok(ExportTarget::AutoOrExplicit(None))
}

fn build_export_artifact_request(
    doc: &ProjectDocument,
    format: &str,
    cfg: &ExportArtifactConfig,
    embedded_ttf_font: Option<Vec<u8>>,
) -> Result<ExportArtifactRequest, EngineError> {
    let default_target = ExportTarget::AutoOrExplicit(cfg.sheet_id.as_ref().map(SheetId::new));
    let (target, artifact_format) = match format {
        "svg" => {
            let mut options = SvgExportOptions { pretty: cfg.pretty, ..SvgExportOptions::default() };
            if let Some(p) = cfg.svg_precision {
                options.precision = p;
            }
            (default_target, ExportArtifactFormat::Svg { options })
        }
        "pdf" => (
            default_target,
            ExportArtifactFormat::Pdf {
                options: PdfExportOptions {
                    color_space: cfg.pdf_color_space,
                    version: cfg.pdf_version,
                    embedded_truetype_font: embedded_ttf_font,
                    ..PdfExportOptions::default()
                },
                all_graphs: cfg.all_graphs,
                sheet_ids: None,
            },
        ),
        "emf" => (default_target, ExportArtifactFormat::Emf { require_kind: None }),
        "png" => (
            default_target,
            ExportArtifactFormat::Png {
                width_px: cfg.width_px,
                height_px: cfg.height_px,
                dpi: Some(cfg.dpi),
                sizing: RasterSizingPolicy::CapToLimits { default_dpi: DEFAULT_PREVIEW_DPI },
            },
        ),
        "tiff" => (
            default_target,
            ExportArtifactFormat::Tiff {
                width_px: cfg.width_px,
                height_px: cfg.height_px,
                dpi: Some(cfg.dpi),
                compression: cfg.tiff_compression,
                sizing: RasterSizingPolicy::CapToLimits { default_dpi: DEFAULT_PREVIEW_DPI },
            },
        ),
        "csv" => (
            resolve_mcp_csv_target(doc, cfg.sheet_id.as_deref(), cfg.tab_id.as_deref())?,
            ExportArtifactFormat::Csv {
                options: cfg.csv_opts.clone().unwrap_or_else(CsvOptions::csv),
                tab_id: cfg.tab_id.clone(),
                tab_index: None,
                tidy: cfg.tidy,
            },
        ),
        "tsv" => (
            resolve_mcp_csv_target(doc, cfg.sheet_id.as_deref(), cfg.tab_id.as_deref())?,
            ExportArtifactFormat::Csv {
                options: cfg.csv_opts.clone().unwrap_or_else(CsvOptions::tsv),
                tab_id: cfg.tab_id.clone(),
                tab_index: None,
                tidy: cfg.tidy,
            },
        ),
        "json" => (default_target, ExportArtifactFormat::Json { pretty: cfg.pretty }),
        _ => return Err(EngineError::with_code(ErrorCode::BadInput, format!("unsupported export format '{format}'"))),
    };
    Ok(ExportArtifactRequest { target, format: artifact_format })
}

fn load_validated_ttf_font_bytes(path: &Path) -> Result<Vec<u8>, EngineError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let bytes = curvebook_platform::fs::read_regular_file_bytes(path, MAX_FONT_FILE_BYTES).map_err(|err| match err {
            curvebook_platform::PlatformError::InvalidPath { reason } => {
                EngineError::with_code(ErrorCode::BadInput, format!("invalid TrueType font `{}`: {reason}", path.display()))
            }
            other => EngineError::from(other),
        })?;
        if !matches!(bytes.get(0..4), Some([0x00, 0x01, 0x00, 0x00] | b"true")) {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                format!("unsupported SFNT font `{}`: expected TrueType outlines (`0x00010000` or `true`), not CFF/OTTO", path.display()),
            ));
        }
        let Some(num_tables) = bytes.get(4..6).and_then(|s| <[u8; 2]>::try_from(s).ok()).map(u16::from_be_bytes) else {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("truncated TrueType font `{}`", path.display())));
        };
        let min_header_len = 12_usize.saturating_add(usize::from(num_tables).saturating_mul(16));
        if num_tables == 0 || bytes.len() < min_header_len {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid TrueType SFNT table directory in `{}`", path.display())));
        }
        Ok(bytes)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        Err(EngineError::with_code(ErrorCode::IoError, "reading TrueType font files is not supported on wasm32"))
    }
}

fn parse_pdf_version(raw: &str) -> Result<PdfVersion, ToolCallError> {
    match raw.trim() {
        "1.4" => Ok(PdfVersion::Pdf14),
        "1.7" => Ok(PdfVersion::Pdf17),
        other => Err(ToolCallError::InvalidParams(format!("unsupported pdf_version '{other}' (expected '1.4' or '1.7')"))),
    }
}

#[cfg(not(target_arch = "wasm32"))]
const fn pdf_version_str(version: PdfVersion) -> &'static str {
    match version {
        PdfVersion::Pdf14 => "1.4",
        PdfVersion::Pdf17 => "1.7",
    }
}

fn parse_dpi(args: &Map<String, Value>) -> Result<u16, ToolCallError> {
    match optional_u32_bounded(args, "dpi", u32::from(MIN_DPI), u32::from(MAX_DPI))? {
        Some(v) => u16::try_from(v).map_err(|_| ToolCallError::InvalidParams(String::from("parameter \"dpi\" out of range"))),
        None => Ok(DEFAULT_PREVIEW_DPI),
    }
}

fn parse_table_kind(raw: &str) -> Result<TableKind, ToolCallError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "xy" => Ok(TableKind::Xy),
        "column" => Ok(TableKind::Column),
        "grouped" => Ok(TableKind::Grouped),
        "contingency" => Ok(TableKind::Contingency),
        "survival" => Ok(TableKind::Survival),
        "parts_of_whole" | "partsofwhole" => Ok(TableKind::PartsOfWhole),
        "multiple_variables" | "multiplevariables" => Ok(TableKind::MultipleVariables),
        "nested" => Ok(TableKind::Nested),
        _ => Err(ToolCallError::InvalidParams(format!("unknown table kind '{raw}'"))),
    }
}

fn parse_subcolumn_mode(raw: &str) -> Result<SubcolumnMode, ToolCallError> {
    let normalized = raw.trim().to_ascii_lowercase();
    if let Some(rest) = normalized.strip_prefix("replicates:") {
        let count = rest
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|&n| (1..=256).contains(&n))
            .ok_or_else(|| ToolCallError::InvalidParams(format!("invalid replicate subcolumn count in '{raw}'")))?;
        return Ok(SubcolumnMode::RawReplicates(count));
    }
    match normalized.as_str() {
        "single" => Ok(SubcolumnMode::Single),
        "mean_sd_n" => Ok(SubcolumnMode::MeanSdN),
        "mean_sem_n" => Ok(SubcolumnMode::MeanSemN),
        "mean_cv_n" => Ok(SubcolumnMode::MeanCvN),
        "mean_low_high" => Ok(SubcolumnMode::MeanLowHigh),
        "upper_lower_limits" => Ok(SubcolumnMode::UpperLowerLimits),
        "median_iqr" => Ok(SubcolumnMode::MedianIqr),
        _ => Err(ToolCallError::InvalidParams(format!("unknown subcolumn_mode '{raw}'"))),
    }
}

fn parse_pdf_color_space(raw: &str) -> Result<PdfColorSpace, ToolCallError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "srgb" | "rgb" => Ok(PdfColorSpace::Srgb),
        "cmyk" => Ok(PdfColorSpace::Cmyk),
        "grayscale" | "gray" => Ok(PdfColorSpace::Grayscale),
        _ => Err(ToolCallError::InvalidParams(format!("unsupported pdf_color_space '{raw}'"))),
    }
}

fn parse_tiff_compression(raw: &str) -> Result<TiffCompression, ToolCallError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "none" => Ok(TiffCompression::None),
        "deflate" => Ok(TiffCompression::Deflate),
        "lzw" => Ok(TiffCompression::Lzw),
        _ => Err(ToolCallError::InvalidParams(format!("unsupported tiff_compression '{raw}'"))),
    }
}

fn sanitize_sheet_id(stem: &str, fallback: &str) -> String {
    let cleaned: String = stem.chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '-' }).collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() { fallback.to_owned() } else { trimmed.to_owned() }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::path::Path;

    use curvebook_doc::ErrorCode;
    use serde_json::json;

    use super::{MAX_LIVE_INLINE_ARCHIVE_BYTES, build_live_report_import_params};

    #[test]
    fn live_report_import_prefers_path_and_rejects_oversized_inline_fallback_before_base64() {
        let by_path = build_live_report_import_params(Some(Path::new("/study/big.pzfx")), Some(b"ignored"), true).expect("path-based import params");
        assert_eq!(by_path, json!({ "path": "/study/big.pzfx", "merge": true }));

        let small_inline = build_live_report_import_params(None, Some(b"PK\x03\x04"), false).expect("small inline fallback");
        assert_eq!(small_inline, json!({ "bytes_base64": "UEsDBA==", "merge": false }));

        let oversized = vec![0x55_u8; MAX_LIVE_INLINE_ARCHIVE_BYTES + 1];
        let err = build_live_report_import_params(None, Some(&oversized), false).expect_err("oversized inline fallback must fail");
        assert_eq!(err.code, ErrorCode::ResourceLimit);
    }
}
