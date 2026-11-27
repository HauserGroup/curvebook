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
//!    must always be launched with `--workdir <dir>`.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use curvebook_doc::{DataSheet, ErrorCode, ProjectDocument, SheetId, SubcolumnMode, TableKind, validate_project_cells};
use curvebook_engine::services::{
    DEFAULT_PREVIEW_DPI, DocumentExportFormat, ExportOptionsSpec, PZFX_EXPORT_DATA_ONLY_NOTICE, PZFX_EXPORT_STRUCTURE_NOTICE, RasterMetadata,
    compile_visual_scene_for_sheet, export_document_artifact, import_registered_path, load_validated_ttf_font_bytes, parse_csv_delimiter_opt,
    parse_dpi, parse_pdf_color_space, parse_pdf_version, parse_tiff_compression, resolve_open_importer, resolve_preview_sheet_id,
    resolve_project_save_exporter, resolve_tidy_flag,
};
#[cfg(not(target_arch = "wasm32"))]
use curvebook_engine::services::{pdf_color_space_str, pdf_version_str, tiff_compression_str};
use curvebook_engine::{EngineError, ExternalPathPolicy, Session};
#[cfg(not(target_arch = "wasm32"))]
use curvebook_format::list_xlsx_sheets;
use curvebook_format::registry::DESC_PRISM;
use curvebook_format::{
    ArchiveExportOptions, CsvOptions, ExportOptions, ExportSource, FormatRegistry, ImportOptions, ImportPayload, XlsxImportOptions,
    decode_delimited_text_bytes, export_prism_bytes_with_options, export_pzfx_bytes, strip_project_data_for_template,
};
use curvebook_pdf::{PdfColorSpace, PdfVersion};
use curvebook_render::{MAX_CAPPED_RASTER_DIM_PX, TiffCompression};
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

#[cfg(not(target_arch = "wasm32"))]
const MAX_LIVE_INLINE_ARCHIVE_BYTES: usize = ((crate::tools::MAX_CONTROL_RESPONSE_BYTES - 65_536) / 4) * 3;
#[cfg(not(target_arch = "wasm32"))]
const MAX_LIVE_INLINE_CSV_BYTES: usize = 8 * 1024 * 1024;

/// Resolve `raw_path` against the active filesystem trust boundary on `state`.
///
/// - When [`McpServerState::workdir_guard`] is configured (`--workdir <dir>`), `raw_path` is
///   confined strictly inside that canonical root via `WorkdirGuard::resolve`, returning
///   [`ErrorCode::PathEscape`] if `raw_path`, `..` components, or symlinks escape `<dir>`.
/// - When no `WorkdirGuard` is configured (including live `--control-port` sessions, which
///   cannot be combined with `--workdir`), `raw_path` may be any non-empty, NUL-free relative or
///   absolute path accessible to the current OS user; relative paths are resolved against
///   [`std::env::current_dir`] so live-attached desktop processes receive the exact same absolute
///   path as the MCP server regardless of CWD differences. Callers writing to the returned path
///   (such as `save_file` and `export`) atomically overwrite any user-writable destination file.
const EXTRA_PATH_PARAM_KEYS: &[&str] =
    &["dir", "directory", "destination_path", "destinationPath", "unpacked_dir", "unpackedDir", "output_path", "outputPath"];

fn is_mcp_path_param_key(key: &str) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    if curvebook_platform::path_guard::PATH_PARAM_KEYS.contains(&key) {
        return true;
    }
    #[cfg(target_arch = "wasm32")]
    if matches!(
        key,
        "path"
            | "file_path"
            | "filePath"
            | "source_path"
            | "sourcePath"
            | "template_path"
            | "templatePath"
            | "ttf_font"
            | "ttfFont"
            | "font_path"
            | "fontPath"
    ) {
        return true;
    }
    EXTRA_PATH_PARAM_KEYS.contains(&key)
}

fn reject_external_err(raw_path: &str) -> EngineError {
    EngineError::with_code(
        ErrorCode::PathEscape,
        format!("external filesystem access to `{raw_path}` is disabled by ExternalPathPolicy::RejectExternal"),
    )
    .with_remediation("Supply inline payload bytes or remove RejectExternal policy.")
}

fn enforce_path_keys_recursive(policy: &ExternalPathPolicy, value: &mut Value) -> Result<(), EngineError> {
    match value {
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if is_mcp_path_param_key(k) {
                    if let Some(raw) = v.as_str() {
                        if !raw.trim().is_empty() {
                            if raw.contains('\0') {
                                return Err(EngineError::with_code(ErrorCode::BadInput, "path must not contain NUL bytes"));
                            }
                            match policy {
                                ExternalPathPolicy::RejectExternal => return Err(reject_external_err(raw)),
                                #[cfg(not(target_arch = "wasm32"))]
                                ExternalPathPolicy::Workdir(guard) => {
                                    let resolved = guard.resolve(raw).map_err(EngineError::from)?;
                                    *v = Value::String(resolved.to_string_lossy().into_owned());
                                }
                                ExternalPathPolicy::Unconstrained => {}
                            }
                        }
                    } else if !v.is_null() {
                        enforce_path_keys_recursive(policy, v)?;
                    }
                } else {
                    enforce_path_keys_recursive(policy, v)?;
                }
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                enforce_path_keys_recursive(policy, item)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn resolve_mcp_path(state: &McpServerState, raw_path: &str) -> Result<PathBuf, EngineError> {
    if raw_path.trim().is_empty() {
        return Err(EngineError::with_code(ErrorCode::BadInput, "path must not be empty"));
    }
    if raw_path.contains('\0') {
        return Err(EngineError::with_code(ErrorCode::BadInput, "path must not contain NUL bytes"));
    }
    let policy = state.effective_path_policy();
    if policy == ExternalPathPolicy::RejectExternal {
        return Err(reject_external_err(raw_path));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let ExternalPathPolicy::Workdir(guard) = policy {
            return guard.resolve(raw_path).map_err(EngineError::from);
        }
        let path = PathBuf::from(raw_path);
        if path.is_relative() {
            let cwd = std::env::current_dir()
                .map_err(|err| EngineError::with_code(ErrorCode::IoError, format!("failed to determine current working directory: {err}")))?;
            Ok(cwd.join(path))
        } else {
            Ok(path)
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = state;
        Err(EngineError::with_code(ErrorCode::IoError, "filesystem operations are not supported on wasm32"))
    }
}

/// Sanitize recursive path keys inside engine command JSON payloads under active `WorkdirGuard` or
/// `ExternalPathPolicy::RejectExternal` (`AUD-13-03`, `AUD-13-23`).
pub(crate) fn sanitize_command_params(state: &McpServerState, params: &mut Value) -> Result<(), EngineError> {
    let policy = state.effective_path_policy();
    match policy {
        ExternalPathPolicy::Unconstrained => Ok(()),
        ExternalPathPolicy::RejectExternal => enforce_path_keys_recursive(&policy, params),
        #[cfg(not(target_arch = "wasm32"))]
        ExternalPathPolicy::Workdir(ref guard) => {
            curvebook_platform::PathResolver::Sandboxed(guard.clone()).sanitize_params(params).map_err(EngineError::from)?;
            enforce_path_keys_recursive(&policy, params)
        }
    }
}

fn check_not_cancelled(state: &McpServerState, tool_name: &str) -> Option<ToolCallResult> {
    if state.current_cancel_token().load(Ordering::Acquire) {
        Some(errors::result_from_code(ErrorCode::Cancelled, format!("{tool_name} cancelled before execution")))
    } else {
        None
    }
}

/// Handle `open_file` dispatched via [`FormatRegistry`] with `ExternalPathPolicy` / `WorkdirGuard` enforcement.
pub(crate) fn handle_open_file(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    if let Some(cancelled) = check_not_cancelled(state, "open_file") {
        return Ok(cancelled);
    }
    reject_unknown_args(
        "open_file",
        args,
        &[
            "path",
            "format",
            "target_kind",
            "subcolumn_mode",
            "delimiter",
            "sheet_id",
            "title",
            "append",
            "sheets",
            "sheet_indices",
            "list_sheets",
            "has_header",
            "header_rows",
            "has_row_titles",
        ],
    )?;
    let raw_path = required_non_empty_str(args, "path")?;
    let explicit_format = optional_non_empty_str(args, "format")?.map(|s| s.to_ascii_lowercase());
    let title_override = optional_non_empty_str(args, "title")?;
    let append = optional_bool(args, "append")?.unwrap_or(false);
    let list_sheets_only = optional_bool(args, "list_sheets")?.unwrap_or(false);

    let resolved_path = match resolve_mcp_path(state, &raw_path) {
        Ok(p) => p,
        Err(err) => return Ok(errors::result_from_engine_error(&err)),
    };

    let reg = FormatRegistry::default_registry();
    let importer = resolve_open_importer(&reg, explicit_format.as_deref(), &resolved_path, true)
        .map_err(|err| ToolCallError::InvalidParams(err.message().to_owned()))?;
    let format_id = importer.descriptor().id;
    let import_opts = build_import_options(&resolved_path, format_id, args)?;

    if list_sheets_only && format_id != "xlsx" {
        return Err(ToolCallError::InvalidParams(format!("`list_sheets` is only supported for `.xlsx` workbooks (got `{format_id}`)")));
    }
    if format_id == "xlsx" && list_sheets_only {
        #[cfg(target_arch = "wasm32")]
        {
            return Ok(errors::result_from_code(ErrorCode::IoError, "filesystem operations are not supported on wasm32"));
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
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
            let sheets = match list_xlsx_sheets(&raw_bytes) {
                Ok(s) => s,
                Err(err) => return Ok(errors::result_from_engine_error(&EngineError::from(err))),
            };
            return ToolCallResult::ok_json(&json!({
                "opened": false,
                "format": "xlsx",
                "path": resolved_path.display().to_string(),
                "sheet_count": sheets.len(),
                "sheets": sheets,
            }));
        }
    }

    // In standalone append mode for `.prism`/`.pzfx`/`.xlsx`, delegate directly to `table.import*`
    // via `path` so the archive is parsed ONCE with zero base64 amplification (`AUD-13-14`).
    #[cfg(not(target_arch = "wasm32"))]
    let is_live_attached = state.control_port().is_some();
    #[cfg(target_arch = "wasm32")]
    let is_live_attached = false;
    if !is_live_attached && append && matches!(format_id, "curvebook" | "curvebook.d" | "cbtemplate") {
        return apply_project_merge_append_via_path(state, &resolved_path, format_id);
    }
    if !is_live_attached && append && matches!(format_id, "prism" | "pzfx" | "xlsx") {
        return apply_report_import_append_via_path(state, &resolved_path, format_id, &import_opts);
    }

    let (payload, raw_bytes) = match import_registered_path(importer, &resolved_path, &import_opts) {
        Ok(res) => res,
        Err(err) => return Ok(errors::result_from_engine_error(&err)),
    };

    apply_imported_payload(state, &resolved_path, format_id, payload, raw_bytes, title_override.as_deref(), &import_opts, append)
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
    let csv_text = match decode_delimited_text_bytes(&raw_bytes, format_id) {
        Ok(text) => text,
        Err(err) => return Err(errors::result_from_engine_error(&EngineError::from(err))),
    };
    let csv_opts =
        import_opts.csv.clone().unwrap_or_else(|| if format_id == "tsv" { CsvOptions::detect_tsv(&csv_text) } else { CsvOptions::detect(&csv_text) });
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

fn build_import_options(path: &Path, format_id: &str, args: &Map<String, Value>) -> Result<ImportOptions, ToolCallError> {
    let explicit_kind = optional_non_empty_str(args, "target_kind")?.map(|raw| parse_table_kind(&raw)).transpose()?;
    let target_kind = explicit_kind.unwrap_or(TableKind::Xy);
    let subcolumn_mode = match optional_non_empty_str(args, "subcolumn_mode")? {
        Some(raw) => parse_subcolumn_mode(&raw)?,
        None => SubcolumnMode::Single,
    };

    let delim_raw = optional_non_empty_str(args, "delimiter")?;
    let csv_override =
        parse_csv_delimiter_opt(delim_raw.as_deref(), "delimiter").map_err(|err| ToolCallError::InvalidParams(err.message().to_owned()))?;

    let stem = path.file_stem().and_then(|s| s.to_str()).filter(|s| !s.trim().is_empty()).unwrap_or("Imported Data");
    let explicit_title = optional_non_empty_str(args, "title")?;
    let sheet_title = explicit_title.clone().unwrap_or_else(|| stem.to_owned());
    let sheet_id_str = optional_non_empty_str(args, "sheet_id")?.unwrap_or_else(|| sanitize_sheet_id(stem, "data-1"));

    let xlsx_opts = if format_id == "xlsx" {
        let sheet_names = parse_optional_string_array(args, "sheets")?;
        let sheet_indices = parse_optional_usize_array(args, "sheet_indices")?;
        let has_header = optional_bool(args, "has_header")?.unwrap_or(true);
        let header_rows = match args.get("header_rows") {
            None | Some(Value::Null) => usize::from(has_header),
            Some(v) => usize::try_from(
                v.as_u64()
                    .filter(|&n| n <= 16)
                    .ok_or_else(|| ToolCallError::InvalidParams(String::from("`header_rows` must be an integer in 0..=16")))?,
            )
            .map_err(|_| ToolCallError::InvalidParams(String::from("`header_rows` out of range")))?,
        };
        let has_row_titles = optional_bool(args, "has_row_titles")?.unwrap_or(false);
        Some(XlsxImportOptions {
            sheet_names,
            sheet_indices,
            has_header,
            header_rows,
            has_row_titles,
            table_kind: explicit_kind,
            subcolumn_mode,
            include_hidden: false,
            title: Some(explicit_title.unwrap_or_else(|| stem.to_owned())),
            ..XlsxImportOptions::default()
        })
    } else {
        None
    };

    Ok(ImportOptions {
        sheet_id: SheetId::new(sheet_id_str),
        title: sheet_title,
        table_kind: target_kind,
        subcolumn_mode,
        csv: csv_override,
        xlsx: xlsx_opts,
    })
}

fn parse_optional_string_array(args: &Map<String, Value>, key: &str) -> Result<Option<Vec<String>>, ToolCallError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let s = item.as_str().ok_or_else(|| ToolCallError::InvalidParams(format!("`{key}` entries must be strings")))?;
                out.push(s.to_owned());
            }
            Ok(Some(out))
        }
        Some(_) => Err(ToolCallError::InvalidParams(format!("`{key}` must be an array of strings"))),
    }
}

fn parse_optional_usize_array(args: &Map<String, Value>, key: &str) -> Result<Option<Vec<usize>>, ToolCallError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let n = item
                    .as_u64()
                    .and_then(|u| usize::try_from(u).ok())
                    .ok_or_else(|| ToolCallError::InvalidParams(format!("`{key}` entries must be non-negative integers")))?;
                out.push(n);
            }
            Ok(Some(out))
        }
        Some(_) => Err(ToolCallError::InvalidParams(format!("`{key}` must be an array of integers"))),
    }
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
        ("curvebook" | "curvebook.d" | "cbtemplate", ImportPayload::Document(_)) if append => {
            apply_project_merge_append_via_path(state, resolved_path, format_id)
        }
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
        ("prism" | "pzfx" | "xlsx", ImportPayload::Report(report)) => {
            let warnings = match serde_json::to_value(&report.warnings) {
                Ok(v) => v,
                Err(err) => {
                    return Ok(errors::result_from_code(ErrorCode::InternalError, format!("failed to serialize import warnings: {err}")));
                }
            };
            apply_report_import_direct(state, resolved_path, format_id, report.document, warnings)
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
fn build_live_report_import_params(
    resolved_path: Option<&Path>,
    raw_bytes: Option<&[u8]>,
    format_id: &str,
    import_opts: &ImportOptions,
    append: bool,
) -> Result<Value, StructuredErrorPayload> {
    let mut map = Map::new();
    let has_path = if let Some(path) = resolved_path {
        let path_str = path.display().to_string();
        if !path_str.trim().is_empty() {
            map.insert(String::from("path"), json!(path_str));
            true
        } else {
            false
        }
    } else {
        false
    };
    if !has_path {
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
        map.insert(String::from("bytes_base64"), json!(encode_base64(bytes)));
    }
    map.insert(String::from("merge"), json!(append));
    if format_id == "xlsx"
        && let Some(xopts) = import_opts.xlsx.as_ref()
    {
        attach_xlsx_command_params(&mut map, xopts);
    }
    Ok(Value::Object(map))
}

fn attach_xlsx_command_params(map: &mut Map<String, Value>, xopts: &XlsxImportOptions) {
    if let Some(ref names) = xopts.sheet_names {
        map.insert(String::from("sheets"), json!(names));
    }
    if let Some(ref indices) = xopts.sheet_indices {
        map.insert(String::from("sheet_indices"), json!(indices));
    }
    map.insert(String::from("has_header"), json!(xopts.has_header));
    map.insert(String::from("header_rows"), json!(xopts.header_rows));
    map.insert(String::from("has_row_titles"), json!(xopts.has_row_titles));
    if let Some(kind) = xopts.table_kind {
        map.insert(String::from("archetype"), json!(kind.label()));
    }
    if xopts.subcolumn_mode != SubcolumnMode::Single
        && let Ok(val) = serde_json::to_value(xopts.subcolumn_mode)
    {
        map.insert(String::from("subcolumn_mode"), val);
    }
    if let Some(ref cell_range) = xopts.cell_range {
        map.insert(String::from("cell_range"), json!(cell_range));
    }
    if xopts.skip_rows > 0 {
        map.insert(String::from("skip_rows"), json!(xopts.skip_rows));
    }
    if xopts.fill_merged_cells {
        map.insert(String::from("fill_merged_cells"), json!(true));
    }
    if let Some(split_col) = xopts.split_by_column {
        map.insert(String::from("split_by_column"), json!(split_col));
    }
    if let Some(ref rm) = xopts.row_match {
        map.insert(String::from("row_match"), json!({ "column": rm.column, "equals": rm.equals, "drop_column": rm.drop_column }));
    }
    if xopts.extract_preamble_info {
        map.insert(String::from("extract_preamble_info"), json!(true));
    }
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
        ("curvebook" | "curvebook.d" | "cbtemplate", ImportPayload::Document(_)) if append => {
            let mut params = Map::new();
            params.insert(String::from("path"), json!(resolved_path.display().to_string()));
            if format_id == "curvebook.d" {
                params.insert(String::from("unpacked"), json!(true));
            }
            let res = match state.send_live_control("engine.execute", json!({ "command_id": "project.merge", "params": Value::Object(params) })) {
                Ok(v) => v,
                Err(err) => return Ok(live_attach_import_error(err)),
            };
            let title = res.pointer("/payload/title").and_then(Value::as_str).unwrap_or("");
            let sheet_count = res.pointer("/payload/sheet_count").and_then(Value::as_u64).unwrap_or(0);
            let added_sheets = res.pointer("/payload/added_sheets").and_then(Value::as_u64).unwrap_or(0);
            let merged_sheet_ids = res.pointer("/payload/merged_sheet_ids").cloned().unwrap_or_else(|| json!([]));
            let source_label = res.pointer("/payload/source_label").and_then(Value::as_str).unwrap_or("");
            let warnings = res.get("warnings").cloned().unwrap_or_else(|| json!([]));
            ToolCallResult::ok_json(&json!({
                "opened": true,
                "merged": true,
                "mode": "live_attach",
                "control_port": port,
                "format": format_id,
                "path": resolved_path.display().to_string(),
                "title": title,
                "total_sheet_count": sheet_count,
                "added_sheets": added_sheets,
                "merged_sheet_ids": merged_sheet_ids,
                "source_label": source_label,
                "warnings": warnings,
            }))
        }
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
        ("prism" | "pzfx" | "xlsx", ImportPayload::Report(report)) => {
            let warnings = match serde_json::to_value(&report.warnings) {
                Ok(v) => v,
                Err(err) => {
                    return Ok(errors::result_from_code(ErrorCode::InternalError, format!("failed to serialize import warnings: {err}")));
                }
            };
            let cmd_id = report_import_command_id(format_id);
            let preserve_xlsx_title = !append && format_id == "xlsx" && title_override.is_some();
            let effective_merge = append || preserve_xlsx_title;
            let import_params =
                match build_live_report_import_params(Some(resolved_path), raw_bytes.as_deref(), format_id, import_opts, effective_merge) {
                    Ok(p) => p,
                    Err(err) => return Ok(live_attach_import_error(err)),
                };
            let send_res = if append {
                state.send_live_control("engine.execute", json!({ "command_id": cmd_id, "params": import_params }))
            } else {
                let file_new_params = if preserve_xlsx_title && let Some(explicit_title) = import_opts.xlsx.as_ref().and_then(|x| x.title.as_deref())
                {
                    json!({ "title": explicit_title })
                } else {
                    json!({})
                };
                state.send_live_control(
                    "command.batch",
                    json!({
                        "atomic": true,
                        "commands": [
                            { "command": "file.new", "params": file_new_params },
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
            let (live_method, live_payload) = match build_live_csv_control_envelope(resolved_path, format_id, &bytes, import_opts, append) {
                Ok(pair) => pair,
                Err(err_res) => return Ok(err_res),
            };
            if let Err(err) = state.send_live_control(live_method, live_payload) {
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

#[cfg(not(target_arch = "wasm32"))]
const LIVE_CSV_ENVELOPE_OVERHEAD_BYTES: usize = 4096;

#[cfg(not(target_arch = "wasm32"))]
fn build_live_csv_control_envelope(
    resolved_path: &Path,
    format_id: &str,
    raw_bytes: &[u8],
    import_opts: &ImportOptions,
    append: bool,
) -> Result<(&'static str, Value), ToolCallResult> {
    let wrap = |params: Value| -> (&'static str, Value) {
        if append {
            ("engine.execute", json!({ "command_id": "table.importCsv", "params": params }))
        } else {
            (
                "command.batch",
                json!({
                    "atomic": true,
                    "commands": [
                        { "command": "file.new", "params": { "title": import_opts.title } },
                        { "command": "table.importCsv", "params": params },
                    ]
                }),
            )
        }
    };

    let budget = crate::tools::MAX_CONTROL_RESPONSE_BYTES.saturating_sub(LIVE_CSV_ENVELOPE_OVERHEAD_BYTES);
    if raw_bytes.len() <= MAX_LIVE_INLINE_CSV_BYTES {
        let inline_params = build_csv_import_params(resolved_path, format_id, raw_bytes.to_vec(), import_opts, false)?;
        let (method, envelope) = wrap(inline_params);
        let encoded_len = serde_json::to_vec(&envelope).map_or(usize::MAX, |v| v.len());
        if encoded_len <= budget {
            return Ok((method, envelope));
        }
    }

    // Automatically fall back to `"path"` mode when JSON string escaping pushes the serialized
    // control frame over the 16 MiB budget (`AUD-13-14`).
    let path_params = build_csv_import_params(resolved_path, format_id, raw_bytes.to_vec(), import_opts, true)?;
    Ok(wrap(path_params))
}

fn report_import_command_id(format_id: &str) -> &'static str {
    match format_id {
        "pzfx" => "table.importPzfx",
        "xlsx" => "table.importXlsx",
        _ => "table.importPrism",
    }
}

fn apply_report_import_direct(
    state: &mut McpServerState,
    resolved_path: &Path,
    format_id: &str,
    document: ProjectDocument,
    warnings: Value,
) -> Result<ToolCallResult, ToolCallError> {
    // Install the already-parsed `ImportReport.document` directly without base64-encoding or
    // re-parsing the archive, preserving `XlsxImportOptions.title` (override or filename stem)
    // (`AUD-13-14`).
    let total_cells = match document.total_allocated_cells() {
        Ok(n) => n,
        Err(err) => return Ok(errors::result_from_engine_error(&EngineError::from(err))),
    };
    if let Err(err) = validate_project_cells(0, total_cells) {
        return Ok(errors::result_from_engine_error(&EngineError::from(err)));
    }
    let sheets = document.total_sheet_count();
    let title = document.title.clone();
    let mut next_session = Session::from_document(document);
    next_session.set_file_path(None, false);
    if !next_session.is_dirty() {
        let _ = next_session.collapse_atomic_group(Session::new());
    }
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

fn apply_project_merge_append_via_path(state: &mut McpServerState, resolved_path: &Path, format_id: &str) -> Result<ToolCallResult, ToolCallError> {
    let mut map = Map::new();
    map.insert(String::from("path"), json!(resolved_path.display().to_string()));
    if format_id == "curvebook.d" {
        map.insert(String::from("unpacked"), json!(true));
    }
    let params = Value::Object(map);
    let cancel = state.current_cancel_token();
    state.mutate_active_session(|session| {
        let outcome = match session.execute_with_caller("project.merge", params, curvebook_engine::Caller::Mcp, &cancel) {
            Ok(out) => out,
            Err(err) => return Ok(errors::result_from_engine_error(&err)),
        };
        let sheets = session.document().total_sheet_count();
        let title = session.document().title.clone();
        let added_sheets = outcome.payload.get("added_sheets").and_then(Value::as_u64).unwrap_or(0);
        let merged_sheet_ids = outcome.payload.get("merged_sheet_ids").cloned().unwrap_or_else(|| json!([]));
        let source_label = outcome.payload.get("source_label").and_then(Value::as_str).unwrap_or("");
        ToolCallResult::ok_json(&json!({
            "opened": true,
            "merged": true,
            "format": format_id,
            "path": resolved_path.display().to_string(),
            "title": title,
            "total_sheet_count": sheets,
            "added_sheets": added_sheets,
            "merged_sheet_ids": merged_sheet_ids,
            "source_label": source_label,
            "warnings": outcome.warnings,
        }))
    })
}

fn apply_report_import_append_via_path(
    state: &mut McpServerState,
    resolved_path: &Path,
    format_id: &str,
    import_opts: &ImportOptions,
) -> Result<ToolCallResult, ToolCallError> {
    let command_id = report_import_command_id(format_id);
    let mut map = Map::new();
    map.insert(String::from("path"), json!(resolved_path.display().to_string()));
    map.insert(String::from("merge"), json!(true));
    if format_id == "xlsx"
        && let Some(xopts) = import_opts.xlsx.as_ref()
    {
        attach_xlsx_command_params(&mut map, xopts);
    }
    let params = Value::Object(map);
    let cancel = state.current_cancel_token();
    state.mutate_active_session(|session| {
        let outcome = match session.execute_with_caller(command_id, params, curvebook_engine::Caller::Mcp, &cancel) {
            Ok(out) => out,
            Err(err) => return Ok(errors::result_from_engine_error(&err)),
        };
        let warnings = outcome.payload.get("warnings").cloned().unwrap_or_else(|| json!([]));
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
    })
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
    let mut payload = json!({
        "opened": true,
        "format": format_id,
        "path": resolved_path.display().to_string(),
        "sheet_id": ds.id.as_str(),
        "title": ds.title,
        "table_kind": ds.kind().label(),
        "rows": ds.archetype.row_count(),
        "columns": ds.archetype.column_count(),
    });
    let text_warnings = inspect_delimited_import_warnings(ds);
    if !text_warnings.is_empty()
        && let Some(obj) = payload.as_object_mut()
    {
        obj.insert(String::from("warnings"), json!(text_warnings));
        obj.insert(
            String::from("next_step_hint"),
            json!(format!(
                "Inspect cells with `doc_inspect({{\"sheet_id\":\"{}\",\"include_cells\":true,\"max_rows\":20}})` and clean placeholders/preamble rows via `table.clearCells`, `table.setCells`, or `table.excludeCells`.",
                ds.id.as_str()
            )),
        );
    }
    ToolCallResult::ok_json(&payload)
}

fn inspect_delimited_import_warnings(ds: &DataSheet) -> Vec<String> {
    use curvebook_doc::{CellValue, DataSetColumn, TableArchetype, XColumnMode};
    let mut non_numeric_text = 0_usize;
    let mut samples: Vec<String> = Vec::new();
    let mut record_text = |text: &str| {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            non_numeric_text += 1;
            if samples.len() < 3 && !samples.iter().any(|s| s == trimmed) {
                samples.push(trimmed.chars().take(32).collect());
            }
        }
    };
    let scan_data_sets = |cols: &[DataSetColumn], record: &mut dyn FnMut(&str)| {
        for col in cols {
            for row in &col.cells {
                for cell in row {
                    if !cell.excluded
                        && let CellValue::Text(s) = &cell.value
                    {
                        record(s);
                    }
                }
            }
        }
    };
    match &ds.archetype {
        TableArchetype::Xy(xy) => {
            if matches!(xy.x_mode, XColumnMode::Numbers { .. }) {
                for row in &xy.x_cells {
                    for cell in row {
                        if !cell.excluded
                            && let CellValue::Text(s) = &cell.value
                        {
                            record_text(s);
                        }
                    }
                }
            }
            scan_data_sets(&xy.data_sets, &mut record_text);
        }
        TableArchetype::Column(col_t) => scan_data_sets(&col_t.data_sets, &mut record_text),
        TableArchetype::Grouped(grp) => scan_data_sets(&grp.data_sets, &mut record_text),
        _ => {}
    }
    if non_numeric_text == 0 {
        return Vec::new();
    }
    vec![format!("Found {non_numeric_text} non-numeric text cell(s) in numeric data columns (samples: {}).", samples.join(", "))]
}

fn detect_foreign_export_format(explicit_format: Option<&str>, path: Option<&Path>) -> Option<&'static str> {
    if let Some(fmt) = explicit_format {
        let norm = fmt.trim().trim_start_matches('.').to_ascii_lowercase();
        if norm == "pzfx" {
            return Some("pzfx");
        }
        if norm == DESC_PRISM.id {
            return Some(DESC_PRISM.id);
        }
        return None;
    }
    let ext = path?.extension()?.to_str()?.to_ascii_lowercase();
    if ext == "pzfx" {
        Some("pzfx")
    } else if ext == DESC_PRISM.id {
        Some(DESC_PRISM.id)
    } else {
        None
    }
}

fn write_atomic_foreign_bytes(dest: &Path, bytes: &[u8]) -> Result<(), ToolCallResult> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(parent) = dest.parent()
            && !parent.as_os_str().is_empty()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            return Err(errors::result_from_payload(
                &StructuredErrorPayload::new(ErrorCode::IoError, format!("failed to prepare directory for '{}': {err}", dest.display()))
                    .with_retryable(true),
            ));
        }
        if let Err(err) = curvebook_platform::fs::write_atomic(dest, bytes) {
            return Err(errors::result_from_engine_error(&EngineError::from(err)));
        }
        Ok(())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (dest, bytes);
        Err(errors::result_from_code(ErrorCode::IoError, "filesystem export is not supported on wasm32"))
    }
}

fn handle_foreign_project_export(
    state: &mut McpServerState,
    format_id: &'static str,
    resolved_path: Option<&Path>,
    experimental: bool,
    as_save_file: bool,
) -> Result<ToolCallResult, ToolCallError> {
    let is_pzfx = format_id == "pzfx";

    #[cfg(not(target_arch = "wasm32"))]
    if let Some(port) = state.control_port() {
        let cmd_id = if is_pzfx { "project.exportPzfx" } else { "project.exportPrism" };
        let mut cmd_params = json!({ "return_bytes": resolved_path.is_none() });
        if let Some(obj) = cmd_params.as_object_mut() {
            if let Some(dest) = resolved_path {
                obj.insert(String::from("path"), json!(dest.display().to_string()));
            }
            if !is_pzfx {
                obj.insert(String::from("experimental"), json!(experimental));
            }
        }
        let remote = match state.send_live_control("engine.execute", json!({ "command_id": cmd_id, "params": cmd_params })) {
            Ok(v) => v,
            Err(err) => return Ok(errors::result_from_payload(&err)),
        };
        let payload = remote.get("payload").or_else(|| remote.get("result")).unwrap_or(&remote);
        let bytes_len = payload.get("bytes_len").and_then(Value::as_u64).unwrap_or(0);
        let warnings = payload.get("warnings").cloned().unwrap_or_else(|| json!([]));
        let mut out = if as_save_file {
            json!({
                "saved": true,
                "mode": "live_attach",
                "control_port": port,
                "format": format_id,
                "path": resolved_path.map(|p| p.display().to_string()),
                "bytes_written": bytes_len,
                "warnings": warnings,
            })
        } else {
            let mut obj = json!({
                "exported": true,
                "mode": "live_attach",
                "control_port": port,
                "format": format_id,
                "sheet_id": "project",
                "warnings": warnings,
            });
            if let Some(dest) = resolved_path {
                if let Some(map) = obj.as_object_mut() {
                    map.insert(String::from("path"), json!(dest.display().to_string()));
                    map.insert(String::from("bytes_written"), json!(bytes_len));
                }
            } else if let Some(b64) = payload.get("bytes_base64").and_then(Value::as_str)
                && let Some(map) = obj.as_object_mut()
            {
                map.insert(String::from("bytes_len"), json!(bytes_len));
                if is_pzfx {
                    let bytes = match decode_base64(b64) {
                        Ok(b) => b,
                        Err(err) => {
                            return Ok(errors::result_from_code(
                                ErrorCode::InternalError,
                                format!("invalid base64 from live project.exportPzfx: {err}"),
                            ));
                        }
                    };
                    let xml = String::from_utf8(bytes).map_err(|err| ToolCallError::Execution(format!("UTF-8 .pzfx export decode failed: {err}")))?;
                    map.insert(String::from("xml"), json!(xml));
                } else {
                    map.insert(String::from("encoding"), json!("base64"));
                    map.insert(String::from("data"), json!(b64));
                }
            }
            obj
        };
        if let Some(map) = out.as_object_mut() {
            if is_pzfx {
                if let Some(preflight) = payload.get("preflight") {
                    map.insert(String::from("preflight"), preflight.clone());
                }
                if let Some(c) = payload.get("data_sheet_count") {
                    map.insert(String::from("data_sheet_count"), c.clone());
                }
                if let Some(c) = payload.get("info_sheet_count") {
                    map.insert(String::from("info_sheet_count"), c.clone());
                }
                if let Some(c) = payload.get("total_dropped_count") {
                    map.insert(String::from("total_dropped_count"), c.clone());
                }
                map.insert(String::from("data_only_notice"), json!(PZFX_EXPORT_DATA_ONLY_NOTICE));
                map.insert(String::from("notice"), json!(PZFX_EXPORT_STRUCTURE_NOTICE));
            } else {
                map.insert(String::from("experimental"), json!(true));
            }
        }
        return ToolCallResult::ok_json(&out);
    }

    let doc = state.session().document();
    if is_pzfx {
        let report = match export_pzfx_bytes(doc) {
            Ok(rep) => rep,
            Err(err) => return Ok(errors::result_from_engine_error(&EngineError::from(err))),
        };
        if let Some(dest) = resolved_path {
            if let Err(err_res) = write_atomic_foreign_bytes(dest, &report.bytes) {
                return Ok(err_res);
            }
            let status_key = if as_save_file { "saved" } else { "exported" };
            let mut out = json!({
                status_key: true,
                "format": "pzfx",
                "path": dest.display().to_string(),
                "bytes_written": report.bytes.len(),
                "data_sheet_count": report.preflight.data_sheet_count,
                "info_sheet_count": report.preflight.info_sheet_count,
                "total_dropped_count": report.preflight.total_dropped_count(),
                "preflight": report.preflight,
                "warnings": report.warnings,
                "data_only_notice": PZFX_EXPORT_DATA_ONLY_NOTICE,
                "notice": PZFX_EXPORT_STRUCTURE_NOTICE,
            });
            if !as_save_file && let Some(map) = out.as_object_mut() {
                map.insert(String::from("sheet_id"), json!("project"));
            }
            return ToolCallResult::ok_json(&out);
        }
        let xml = String::from_utf8(report.bytes).map_err(|err| ToolCallError::Execution(format!("UTF-8 .pzfx export decode failed: {err}")))?;
        return ToolCallResult::ok_json(&json!({
            "exported": true,
            "format": "pzfx",
            "sheet_id": "project",
            "bytes_len": xml.len(),
            "xml": xml,
            "data_sheet_count": report.preflight.data_sheet_count,
            "info_sheet_count": report.preflight.info_sheet_count,
            "total_dropped_count": report.preflight.total_dropped_count(),
            "preflight": report.preflight,
            "warnings": report.warnings,
            "data_only_notice": PZFX_EXPORT_DATA_ONLY_NOTICE,
            "notice": PZFX_EXPORT_STRUCTURE_NOTICE,
        }));
    }

    let report = match export_prism_bytes_with_options(doc, ArchiveExportOptions { experimental }) {
        Ok(rep) => rep,
        Err(err) => return Ok(errors::result_from_engine_error(&EngineError::from(err))),
    };
    if let Some(dest) = resolved_path {
        if let Err(err_res) = write_atomic_foreign_bytes(dest, &report.bytes) {
            return Ok(err_res);
        }
        let status_key = if as_save_file { "saved" } else { "exported" };
        let mut out = json!({
            status_key: true,
            "format": DESC_PRISM.id,
            "experimental": true,
            "path": dest.display().to_string(),
            "bytes_written": report.bytes.len(),
            "warnings": report.warnings,
        });
        if !as_save_file && let Some(map) = out.as_object_mut() {
            map.insert(String::from("sheet_id"), json!("project"));
        }
        return ToolCallResult::ok_json(&out);
    }
    ToolCallResult::ok_json(&json!({
        "exported": true,
        "format": DESC_PRISM.id,
        "experimental": true,
        "sheet_id": "project",
        "encoding": "base64",
        "bytes_len": report.bytes.len(),
        "data": encode_base64(&report.bytes),
        "warnings": report.warnings,
    }))
}

/// Handle `save_file` dispatched via [`FormatRegistry`] (which persists atomically via `write_atomic`).
pub(crate) fn handle_save_file(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    if let Some(cancelled) = check_not_cancelled(state, "save_file") {
        return Ok(cancelled);
    }
    reject_unknown_args("save_file", args, &["path", "format", "experimental"])?;
    let explicit_format = optional_non_empty_str(args, "format")?.map(|s| s.trim().trim_start_matches('.').to_ascii_lowercase());
    let experimental = optional_bool(args, "experimental")?.unwrap_or(false);
    let explicit_path = optional_non_empty_str(args, "path")?;

    if let Some(foreign_fmt) = detect_foreign_export_format(explicit_format.as_deref(), explicit_path.as_deref().map(Path::new)) {
        let Some(raw_path) = explicit_path else {
            return Err(ToolCallError::InvalidParams(String::from("save_file requires an explicit \"path\" when exporting .pzfx or .prism files")));
        };
        let resolved_path = match resolve_mcp_path(state, &raw_path) {
            Ok(p) => p,
            Err(err) => return Ok(errors::result_from_engine_error(&err)),
        };
        return handle_foreign_project_export(state, foreign_fmt, Some(&resolved_path), experimental, true);
    }

    #[cfg(not(target_arch = "wasm32"))]
    if let Some(port) = state.control_port() {
        let raw_path = match explicit_path {
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
        let reg = FormatRegistry::default_registry();
        let exporter = resolve_project_save_exporter(&reg, explicit_format.as_deref(), &resolved_path)
            .map_err(|err| ToolCallError::InvalidParams(err.message().to_owned()))?;
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

    let raw_path = match explicit_path {
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
    let reg = FormatRegistry::default_registry();
    let exporter = resolve_project_save_exporter(&reg, explicit_format.as_deref(), &resolved_path)
        .map_err(|err| ToolCallError::InvalidParams(err.message().to_owned()))?;
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

/// Handle `render_preview`: compile GraphSheet/LayoutSheet and rasterize PNG preview with metadata.
pub(crate) fn handle_render_preview(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    if let Some(cancelled) = check_not_cancelled(state, "render_preview") {
        return Ok(cancelled);
    }
    reject_unknown_args("render_preview", args, &["sheet_id", "width_px", "height_px", "dpi"])?;
    let sheet_id = optional_non_empty_str(args, "sheet_id")?;
    let dpi = parse_dpi(args.get("dpi"), DEFAULT_PREVIEW_DPI).map_err(|err| ToolCallError::InvalidParams(err.message().to_owned()))?;
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
    let mut spec = ExportOptionsSpec::new(DocumentExportFormat::Png);
    spec.sheet_id = Some(target_id.clone());
    spec.width_px = requested_w;
    spec.height_px = requested_h;
    spec.dpi = Some(dpi);
    let req = match spec.into_artifact_request(doc, true) {
        Ok(req) => req,
        Err(err) => return Ok(errors::result_from_engine_error(&err)),
    };
    let artifact = match export_document_artifact(doc, &req, Some(&cancel), "render_preview") {
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
    if let Some(cancelled) = check_not_cancelled(state, "export") {
        return Ok(cancelled);
    }
    reject_unknown_args(
        "export",
        args,
        &[
            "format",
            "sheet_id",
            "output_path",
            "path",
            "experimental",
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
    let raw_format = required_non_empty_str(args, "format")?;
    let experimental = optional_bool(args, "experimental")?.unwrap_or(false);
    let output_path = optional_non_empty_str(args, "output_path")?.or(optional_non_empty_str(args, "path")?);
    if let Some(foreign_fmt) = detect_foreign_export_format(Some(&raw_format), None) {
        let resolved_path = match output_path.as_deref() {
            Some(raw) => match resolve_mcp_path(state, raw) {
                Ok(p) => Some(p),
                Err(err) => return Ok(errors::result_from_engine_error(&err)),
            },
            None => None,
        };
        return handle_foreign_project_export(state, foreign_fmt, resolved_path.as_deref(), experimental, false);
    }
    let format = DocumentExportFormat::parse(&raw_format).map_err(|err| ToolCallError::InvalidParams(err.message().to_owned()))?;

    let sheet_id = optional_non_empty_str(args, "sheet_id")?;
    let (mut spec, ttf_font) = parse_mcp_export_spec(args, format, sheet_id)?;

    let resolved_path = match output_path.as_deref() {
        Some(raw) => match resolve_mcp_path(state, raw) {
            Ok(p) => Some(p),
            Err(err) => return Ok(errors::result_from_engine_error(&err)),
        },
        None => None,
    };

    let resolved_ttf_font = match ttf_font.as_deref() {
        Some(raw) => match resolve_mcp_path(state, raw) {
            Ok(p) => Some(p),
            Err(err) => return Ok(errors::result_from_engine_error(&err)),
        },
        None => None,
    };

    let format_str = format.as_str();

    #[cfg(not(target_arch = "wasm32"))]
    if let Some(port) = state.control_port() {
        let mut live_params = json!({
            "format": format_str,
            "sheet_id": spec.sheet_id.as_ref().map(SheetId::as_str),
            "pretty": spec.pretty,
            "tidy": spec.tidy,
            "tab_id": spec.tab_id,
            "csv_delimiter": spec.csv_options.as_ref().map(|c| c.delimiter.to_string()),
            "svg_precision": spec.svg_precision,
            "pdf_color_space": pdf_color_space_str(spec.pdf_color_space),
            "pdf_version": pdf_version_str(spec.pdf_version),
            "ttf_font": resolved_ttf_font.as_ref().map(|p| p.display().to_string()),
            "all_graphs": spec.all_graphs,
            "dpi": spec.dpi.unwrap_or(DEFAULT_PREVIEW_DPI),
            "width_px": spec.width_px,
            "height_px": spec.height_px,
            "tiff_compression": tiff_compression_str(spec.tiff_compression),
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
                return attach_figure_warnings(
                    ToolCallResult::ok_json(&json!({
                        "exported": true,
                        "mode": "live_attach",
                        "control_port": port,
                        "format": format_str,
                        "sheet_id": target_id,
                        "path": written_path,
                        "bytes_written": bytes_written,
                    })),
                    remote.get("warnings"),
                );
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
            return attach_figure_warnings(write_atomic_export(&dest, format_str, &target_id, &bytes), remote.get("warnings"));
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
                let dpi_u16 =
                    remote.get("dpi").and_then(Value::as_u64).and_then(|d| u16::try_from(d).ok()).unwrap_or(spec.dpi.unwrap_or(DEFAULT_PREVIEW_DPI));
                Some(RasterMetadata { width_px: w32, height_px: h32, dpi: dpi_u16 })
            }
            _ => None,
        };
        return attach_figure_warnings(format_inline_export_result(format_str, &target_id, &bytes, raster_meta), remote.get("warnings"));
    }

    if format == DocumentExportFormat::Pdf
        && let Some(font_path) = resolved_ttf_font.as_deref()
    {
        spec.embedded_ttf_font = match load_validated_ttf_font_bytes(font_path) {
            Ok(bytes) => Some(bytes),
            Err(err) => return Ok(errors::result_from_engine_error(&err)),
        };
    }

    let doc = state.session().document();
    let req = match spec.into_artifact_request(doc, true) {
        Ok(req) => req,
        Err(err) => return Ok(errors::result_from_engine_error(&err)),
    };
    let cancel = state.current_cancel_token();
    let artifact = match export_document_artifact(doc, &req, Some(&cancel), "export") {
        Ok(art) => art,
        Err(err) => return Ok(errors::result_from_engine_error(&err)),
    };

    let warnings = Value::Array(artifact.warnings.iter().map(|w| json!({ "code": w.code, "message": w.message })).collect());
    if let Some(dest) = resolved_path {
        return attach_figure_warnings(write_atomic_export(&dest, format_str, &artifact.target_id, &artifact.bytes), Some(&warnings));
    }

    attach_figure_warnings(format_inline_export_result(format_str, &artifact.target_id, &artifact.bytes, artifact.raster), Some(&warnings))
}

/// Report figure warnings (for example a fitted curve omitted because its fit failed or is pending
/// recalculation) alongside an export result: as a `warnings` array in the structured content and as
/// a trailing text block, so a client cannot mistake an incomplete figure for a complete one.
fn attach_figure_warnings(result: Result<ToolCallResult, ToolCallError>, warnings: Option<&Value>) -> Result<ToolCallResult, ToolCallError> {
    let mut result = result?;
    let Some(Value::Array(items)) = warnings else {
        return Ok(result);
    };
    if items.is_empty() || result.is_error {
        return Ok(result);
    }
    let lines: Vec<String> = items
        .iter()
        .map(|w| {
            let code = w.get("code").and_then(Value::as_str).unwrap_or("figure_warning");
            let message = w.get("message").and_then(Value::as_str).unwrap_or_default();
            format!("- {code}: {message}")
        })
        .collect();
    result.content.push(ContentBlock::Text { text: format!("Figure warnings ({}):\n{}", items.len(), lines.join("\n")) });
    match result.structured_content.as_mut() {
        Some(Value::Object(obj)) => {
            obj.insert(String::from("warnings"), Value::Array(items.clone()));
        }
        Some(_) => {}
        None => result.structured_content = Some(json!({ "warnings": items })),
    }
    Ok(result)
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

fn invalid_params_from_engine(err: EngineError) -> ToolCallError {
    ToolCallError::InvalidParams(err.message().to_owned())
}

fn parse_mcp_export_spec(
    args: &Map<String, Value>,
    format: DocumentExportFormat,
    sheet_id: Option<String>,
) -> Result<(ExportOptionsSpec, Option<String>), ToolCallError> {
    let mut spec = ExportOptionsSpec::new(format);
    spec.sheet_id = sheet_id.map(SheetId::new);
    spec.pretty = optional_bool(args, "pretty")?.unwrap_or(true);
    let explicit_tidy = optional_bool(args, "tidy")?;
    let tab_id = optional_non_empty_str(args, "tab_id")?;
    spec.tidy = resolve_tidy_flag(explicit_tidy, tab_id.as_deref()).map_err(invalid_params_from_engine)?;
    spec.tab_id = tab_id;
    let csv_delim = optional_non_empty_str(args, "csv_delimiter")?;
    spec.csv_options = parse_csv_delimiter_opt(csv_delim.as_deref(), "csv_delimiter").map_err(invalid_params_from_engine)?;
    spec.svg_precision = match optional_u32_bounded(args, "svg_precision", 1, 8)? {
        Some(p) => Some(u8::try_from(p).map_err(|_| ToolCallError::InvalidParams(String::from("svg_precision out of bounds")))?),
        None => None,
    };
    spec.pdf_color_space = match optional_non_empty_str(args, "pdf_color_space")? {
        Some(raw) => parse_pdf_color_space(&raw).map_err(invalid_params_from_engine)?,
        None => PdfColorSpace::Srgb,
    };
    spec.pdf_version = match optional_non_empty_str(args, "pdf_version")? {
        Some(raw) => parse_pdf_version(&raw).map_err(invalid_params_from_engine)?,
        None => PdfVersion::Pdf17,
    };
    let ttf_font = optional_non_empty_str(args, "ttf_font")?;
    spec.all_graphs = optional_bool(args, "all_graphs")?.unwrap_or(false);
    spec.dpi = Some(parse_dpi(args.get("dpi"), DEFAULT_PREVIEW_DPI).map_err(invalid_params_from_engine)?);
    spec.width_px = optional_u32_bounded(args, "width_px", 1, MAX_CAPPED_RASTER_DIM_PX)?;
    spec.height_px = optional_u32_bounded(args, "height_px", 1, MAX_CAPPED_RASTER_DIM_PX)?;
    spec.tiff_compression = match optional_non_empty_str(args, "tiff_compression")? {
        Some(raw) => parse_tiff_compression(&raw).map_err(invalid_params_from_engine)?,
        None => TiffCompression::Lzw,
    };
    Ok((spec, ttf_font))
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
        "single" | "none" => Ok(SubcolumnMode::Single),
        "mean_sd" => Ok(SubcolumnMode::MeanSd),
        "mean_sem" => Ok(SubcolumnMode::MeanSem),
        "mean_sd_n" => Ok(SubcolumnMode::MeanSdN),
        "mean_sem_n" => Ok(SubcolumnMode::MeanSemN),
        "mean_cv_n" => Ok(SubcolumnMode::MeanCvN),
        "mean_low_high" => Ok(SubcolumnMode::MeanLowHigh),
        "upper_lower_limits" => Ok(SubcolumnMode::UpperLowerLimits),
        "median_iqr" => Ok(SubcolumnMode::MedianIqr),
        _ => Err(ToolCallError::InvalidParams(format!(
            "unknown subcolumn_mode '{raw}'; expected 'single' (or 'none'), 'replicates:<1..=256>', 'mean_sd', 'mean_sem', 'mean_sd_n', 'mean_sem_n', 'mean_cv_n', 'mean_low_high', 'upper_lower_limits', or 'median_iqr'"
        ))),
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
    use curvebook_format::ImportOptions;
    use serde_json::json;

    use super::{MAX_LIVE_INLINE_ARCHIVE_BYTES, build_live_report_import_params};

    #[test]
    fn live_report_import_prefers_path_and_rejects_oversized_inline_fallback_before_base64() {
        let opts = ImportOptions::default();
        let by_path = build_live_report_import_params(Some(Path::new("/study/big.pzfx")), Some(b"ignored"), "pzfx", &opts, true)
            .expect("path-based import params");
        assert_eq!(by_path, json!({ "path": "/study/big.pzfx", "merge": true }));

        let small_inline = build_live_report_import_params(None, Some(b"PK\x03\x04"), "xlsx", &opts, false).expect("small inline fallback");
        assert_eq!(small_inline, json!({ "bytes_base64": "UEsDBA==", "merge": false }));

        let oversized = vec![0x55_u8; MAX_LIVE_INLINE_ARCHIVE_BYTES + 1];
        let err = build_live_report_import_params(None, Some(&oversized), "xlsx", &opts, false).expect_err("oversized inline fallback must fail");
        assert_eq!(err.code, ErrorCode::ResourceLimit);
    }

    #[test]
    fn attach_xlsx_command_params_forwards_r17_7_options() {
        use curvebook_doc::ColumnEqualsFilter;
        use curvebook_format::XlsxImportOptions;

        let xopts = XlsxImportOptions {
            cell_range: Some(String::from("B4:M11")),
            skip_rows: 3,
            fill_merged_cells: true,
            split_by_column: Some(0),
            row_match: Some(ColumnEqualsFilter { column: 1, equals: String::from("ChA"), drop_column: false }),
            extract_preamble_info: true,
            ..XlsxImportOptions::default()
        };
        let opts = ImportOptions { xlsx: Some(xopts), ..ImportOptions::default() };
        let params = build_live_report_import_params(Some(Path::new("/study/kinetic.xlsx")), None, "xlsx", &opts, true).expect("live xlsx params");
        assert_eq!(params["cell_range"], "B4:M11");
        assert_eq!(params["skip_rows"], 3);
        assert_eq!(params["fill_merged_cells"], true);
        assert_eq!(params["split_by_column"], 0);
        assert_eq!(params["row_match"], json!({ "column": 1, "equals": "ChA", "drop_column": false }));
        assert_eq!(params["extract_preamble_info"], true);
    }
}
