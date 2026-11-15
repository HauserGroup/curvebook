//! Unified `FormatRegistry` adapters, `WorkdirGuard` path containment, atomic disk persistence,
//! figure rasterization/vector export, and `open_file`/`save_file`/`export`/`render_preview` handlers.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use lumencraft_chart::{ChartScene, compile_graph, compile_layout};
use lumencraft_doc::{DataSheet, ErrorCode, ProjectDocument, ResultSheet, SheetId, SubcolumnMode, TableKind};
use lumencraft_engine::{EngineError, Session};
use lumencraft_format::{
    CsvOptions, ExportOptions, ExportSource, Exporter, FormatDescriptor, FormatError, FormatKind, FormatRegistry, ImportOptions, ImportPayload,
    Importer, export_datasheet_to_csv, export_result_sheet_json, export_result_sheet_tidy_csv, export_result_tab_csv,
    strip_project_data_for_template,
};
use lumencraft_pdf::{PdfColorSpace, PdfExportOptions, render_pdf, render_pdf_pages};
use lumencraft_render::{RenderTargetSpec, TiffCompression, encode_png, encode_tiff, render_scene};
use lumencraft_svg::{SvgExportOptions, render_svg};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::errors::{self, StructuredErrorPayload};
use crate::tools::{
    ContentBlock, McpServerState, ToolCallError, ToolCallResult, encode_base64, optional_bool, optional_non_empty_str, optional_u32_bounded,
    required_non_empty_str,
};

const DEFAULT_PREVIEW_WIDTH_PX: u32 = 960;
const DEFAULT_PREVIEW_HEIGHT_PX: u32 = 640;
const DEFAULT_PREVIEW_DPI: u16 = 144;
const MAX_RASTER_DIM_PX: u32 = 8192;
const MAX_DPI: u16 = 2400;
const MIN_DPI: u16 = 18;

const DESC_SVG: FormatDescriptor =
    FormatDescriptor { id: "svg", label: "Scalable Vector Graphics", extensions: &["svg"], kind: FormatKind::FigureVector };
const DESC_PDF: FormatDescriptor =
    FormatDescriptor { id: "pdf", label: "Portable Document Format", extensions: &["pdf"], kind: FormatKind::FigureVector };
const DESC_PNG: FormatDescriptor =
    FormatDescriptor { id: "png", label: "Portable Network Graphics", extensions: &["png"], kind: FormatKind::FigureRaster };
const DESC_TIFF: FormatDescriptor =
    FormatDescriptor { id: "tiff", label: "Tagged Image File Format", extensions: &["tiff", "tif"], kind: FormatKind::FigureRaster };
const DESC_CSV: FormatDescriptor =
    FormatDescriptor { id: "csv", label: "Comma-Separated Values", extensions: &["csv"], kind: FormatKind::DelimitedText };
const DESC_JSON: FormatDescriptor = FormatDescriptor { id: "json", label: "JSON Result Table", extensions: &["json"], kind: FormatKind::ResultTable };

/// Resolve `raw_path` using the configured [`WorkdirGuard`] when present.
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
pub(crate) fn sanitize_command_params(state: &McpServerState, params: &mut Value) -> Result<(), EngineError> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(guard) = state.workdir_guard() {
        lumencraft_platform::PathResolver::Sandboxed(guard.clone()).sanitize_params(params).map_err(EngineError::from)?;
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (state, params);
    Ok(())
}

/// Handle `open_file` dispatched via [`FormatRegistry`] with optional `WorkdirGuard` enforcement.
pub(crate) fn handle_open_file(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let raw_path = required_non_empty_str(args, "path")?;
    let explicit_format = optional_non_empty_str(args, "format")?.map(|s| s.to_ascii_lowercase());
    let title_override = optional_non_empty_str(args, "title")?;
    let append = optional_bool(args, "append")?.unwrap_or(false);

    let raw_candidate = Path::new(&raw_path);
    let reg = FormatRegistry::default_registry();
    let importer = resolve_importer(&reg, explicit_format.as_deref(), raw_candidate)?;
    let import_opts = build_import_options(raw_candidate, args)?;

    let resolved_path = match resolve_mcp_path(state, &raw_path) {
        Ok(p) => p,
        Err(err) => return Ok(errors::result_from_engine_error(&err)),
    };

    let format_id = importer.descriptor().id;
    let payload = match importer.import_path(&resolved_path, &import_opts) {
        Ok(p) => p,
        Err(err) => return Ok(errors::result_from_engine_error(&EngineError::from(err))),
    };

    apply_imported_payload(state, &resolved_path, format_id, payload, title_override.as_deref(), &import_opts, append)
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

fn apply_imported_payload(
    state: &mut McpServerState,
    resolved_path: &Path,
    format_id: &str,
    payload: ImportPayload,
    title_override: Option<&str>,
    import_opts: &ImportOptions,
    append: bool,
) -> Result<ToolCallResult, ToolCallError> {
    match (format_id, payload) {
        ("lumencraft" | "lumencraft.d", ImportPayload::Document(doc)) => {
            let sheets = doc.total_sheet_count();
            let title = doc.title.clone();
            state.install_session(Session::from_document(doc));
            state.session.set_file_path(Some(resolved_path.to_path_buf()), true);
            ToolCallResult::ok_json(&json!({
                "opened": true,
                "format": format_id,
                "path": resolved_path.display().to_string(),
                "title": title,
                "total_sheet_count": sheets,
            }))
        }
        ("lctemplate", ImportPayload::Document(raw_doc)) => {
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
            state.session.set_file_path(None, true);
            ToolCallResult::ok_json(&json!({
                "opened": true,
                "format": "lctemplate",
                "path": resolved_path.display().to_string(),
                "title": title,
                "total_sheet_count": sheets,
            }))
        }
        ("prism" | "pzfx", ImportPayload::Report(report)) => {
            let warnings = match serde_json::to_value(&report.warnings) {
                Ok(v) => v,
                Err(err) => {
                    return Ok(errors::result_from_code(ErrorCode::InternalError, format!("failed to serialize import warnings: {err}")));
                }
            };
            apply_report_import(state, resolved_path, format_id, warnings)
        }
        ("csv" | "tsv", ImportPayload::DataSheet(_)) => apply_csv_import(state, resolved_path, format_id, import_opts, append),
        _ => Ok(errors::result_from_code(ErrorCode::UnsupportedFormat, format!("unexpected payload returned by importer '{format_id}'"))),
    }
}

fn apply_report_import(state: &mut McpServerState, resolved_path: &Path, format_id: &str, warnings: Value) -> Result<ToolCallResult, ToolCallError> {
    let bytes = match std::fs::read(resolved_path) {
        Ok(b) => b,
        Err(err) => {
            return Ok(errors::result_from_code(ErrorCode::IoError, format!("failed to read '{}': {err}", resolved_path.display())));
        }
    };
    let command_id = if format_id == "prism" { "table.importPrism" } else { "table.importPzfx" };
    let params = json!({
        "bytes_base64": encode_base64(&bytes),
        "merge": false,
    });

    let mut next_session = Session::new();
    let cancel = std::sync::atomic::AtomicBool::new(false);
    if let Err(err) = next_session.execute_with_caller(command_id, params, lumencraft_engine::Caller::Mcp, &cancel) {
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
    import_opts: &ImportOptions,
    append: bool,
) -> Result<ToolCallResult, ToolCallError> {
    if append && state.session.document().sheet_section(&import_opts.sheet_id).is_some() {
        return Ok(errors::result_from_engine_error(&EngineError::from(lumencraft_doc::DocError::DuplicateSheetId {
            sheet_id: import_opts.sheet_id.clone(),
        })));
    }

    let csv_text = match std::fs::read_to_string(resolved_path) {
        Ok(text) => text,
        Err(err) => {
            return Ok(errors::result_from_code(ErrorCode::IoError, format!("failed to read '{}': {err}", resolved_path.display())));
        }
    };
    let csv_opts = import_opts.csv.clone().unwrap_or_else(|| if format_id == "tsv" { CsvOptions::tsv() } else { CsvOptions::detect(&csv_text) });
    let subcolumn_mode = match serde_json::to_value(import_opts.subcolumn_mode) {
        Ok(v) => v,
        Err(err) => {
            return Ok(errors::result_from_code(ErrorCode::InternalError, format!("failed to serialize subcolumn mode: {err}")));
        }
    };
    let params = json!({
        "sheet_id": import_opts.sheet_id.as_str(),
        "csv": csv_text,
        "title": import_opts.title,
        "archetype": import_opts.table_kind.label(),
        "subcolumn_mode": subcolumn_mode,
        "delimiter": csv_opts.delimiter.to_string(),
        "decimal_separator": csv_opts.decimal_separator.to_string(),
    });

    if append {
        return execute_csv_import_on_session(&mut state.session, format_id, resolved_path, &import_opts.sheet_id, params);
    }

    let doc = match ProjectDocument::new(import_opts.title.clone()) {
        Ok(d) => d,
        Err(err) => return Ok(errors::result_from_engine_error(&EngineError::from(err))),
    };
    let mut next_session = Session::from_document(doc);
    let result = execute_csv_import_on_session(&mut next_session, format_id, resolved_path, &import_opts.sheet_id, params)?;
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
) -> Result<ToolCallResult, ToolCallError> {
    let cancel = std::sync::atomic::AtomicBool::new(false);
    if let Err(err) = session.execute_with_caller("table.importCsv", params, lumencraft_engine::Caller::Mcp, &cancel) {
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
    let raw_path = match optional_non_empty_str(args, "path")? {
        Some(p) => p,
        None => match state.session.file_path() {
            Some(existing) => existing.display().to_string(),
            None => {
                return Err(ToolCallError::InvalidParams(String::from("save_file requires \"path\" when session has no associated file_path")));
            }
        },
    };
    let explicit_format = optional_non_empty_str(args, "format")?.map(|s| s.to_ascii_lowercase());
    let reg = FormatRegistry::default_registry();
    let exporter = resolve_save_exporter(&reg, explicit_format.as_deref(), Path::new(&raw_path))?;
    let format_id = exporter.descriptor().id;

    let resolved_path = match resolve_mcp_path(state, &raw_path) {
        Ok(p) => p,
        Err(err) => return Ok(errors::result_from_engine_error(&err)),
    };

    let doc = state.session.document().clone();
    if let Err(err) = exporter.export_path(ExportSource::Document(&doc), &resolved_path, &ExportOptions::default()) {
        let engine_err = EngineError::from(err);
        return Ok(errors::result_from_engine_error(&engine_err));
    }

    if matches!(format_id, "lumencraft" | "lumencraft.d") {
        state.session.set_file_path(Some(resolved_path.clone()), true);
    }

    ToolCallResult::ok_json(&json!({
        "saved": true,
        "format": format_id,
        "path": resolved_path.display().to_string(),
        "schema_version": doc.schema_version,
        "total_sheet_count": doc.total_sheet_count(),
    }))
}

fn resolve_save_exporter<'a>(reg: &'a FormatRegistry, explicit: Option<&str>, path: &Path) -> Result<&'a dyn Exporter, ToolCallError> {
    if let Some(fmt) = explicit {
        if !matches!(fmt, "lumencraft" | "lumencraft.d" | "lctemplate") {
            return Err(ToolCallError::InvalidParams(format!("unsupported save format '{fmt}'")));
        }
        return reg.exporter_by_format(fmt).ok_or_else(|| ToolCallError::InvalidParams(format!("unsupported save format '{fmt}'")));
    }
    if let Some(found) = reg.exporter_for_path(path)
        && matches!(found.descriptor().id, "lumencraft" | "lumencraft.d" | "lctemplate")
    {
        return Ok(found);
    }
    reg.exporter_by_format("lumencraft").ok_or_else(|| ToolCallError::InvalidParams(String::from("lumencraft exporter unavailable")))
}

/// Handle `render_preview`: compile GraphSheet/LayoutSheet and rasterize PNG preview with metadata.
pub(crate) fn handle_render_preview(state: &McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let sheet_id = optional_non_empty_str(args, "sheet_id")?;
    let dpi = parse_dpi(args)?;
    let requested_w = optional_u32_bounded(args, "width_px", 1, MAX_RASTER_DIM_PX)?;
    let requested_h = optional_u32_bounded(args, "height_px", 1, MAX_RASTER_DIM_PX)?;

    let (target_id, sheet_kind, scene) = match compile_visual_sheet(state.session.document(), sheet_id.as_deref()) {
        Ok(compiled) => compiled,
        Err(msg) => return Ok(errors::result_from_code(ErrorCode::NotFound, msg)),
    };

    let spec = build_render_spec(&scene, requested_w, requested_h, dpi);
    let raster = match render_scene(&scene, spec) {
        Ok(img) => img,
        Err(err) => return Ok(errors::result_from_engine_error(&EngineError::from(err))),
    };
    let png_bytes = match encode_png(&raster) {
        Ok(bytes) => bytes,
        Err(err) => return Ok(errors::result_from_engine_error(&EngineError::from(err))),
    };

    let metadata = json!({
        "sheet_id": target_id,
        "sheet_kind": sheet_kind,
        "title": scene.title,
        "width_px": raster.width_px,
        "height_px": raster.height_px,
        "dpi": raster.dpi,
        "width_pt": scene.width_pt,
        "height_pt": scene.height_pt,
        "png_bytes": png_bytes.len(),
        "primitives_count": scene.primitives.len(),
        "warnings": scene.warnings,
    });
    let metadata_text = serde_json::to_string_pretty(&metadata).unwrap_or_default();

    Ok(ToolCallResult {
        content: vec![
            ContentBlock::Image { data: encode_base64(&png_bytes), mime_type: String::from("image/png") },
            ContentBlock::Text { text: metadata_text },
        ],
        is_error: false,
        structured_content: Some(metadata),
    })
}

/// Handle `export` dispatched through a populated [`FormatRegistry`] with `WorkdirGuard` path checks
/// and atomic writes (`write_atomic`).
pub(crate) fn handle_export(state: &McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let format = required_non_empty_str(args, "format")?.to_ascii_lowercase();
    if !matches!(format.as_str(), "svg" | "pdf" | "png" | "tiff" | "csv" | "json") {
        return Err(ToolCallError::InvalidParams(format!("unsupported export format '{format}'")));
    }

    let sheet_id = optional_non_empty_str(args, "sheet_id")?;
    let output_path = optional_non_empty_str(args, "output_path")?.or(optional_non_empty_str(args, "path")?);
    let cfg = ExportArtifactConfig::parse(args, sheet_id.clone())?;

    let resolved_path = match output_path.as_deref() {
        Some(raw) => match resolve_mcp_path(state, raw) {
            Ok(p) => Some(p),
            Err(err) => return Ok(errors::result_from_engine_error(&err)),
        },
        None => None,
    };

    let recorder = Arc::new(ExportArtifactRecorder::default());
    let reg = build_mcp_export_registry(&cfg, Arc::clone(&recorder));
    let Some(exporter) = reg.exporter_by_format(&format) else {
        return Err(ToolCallError::InvalidParams(format!("unsupported export format '{format}'")));
    };

    let export_opts = ExportOptions { csv: cfg.csv_opts.clone(), pretty: cfg.pretty, sheet_id };
    let doc = state.session.document();
    let bytes = match exporter.export_bytes(ExportSource::Document(doc), &export_opts) {
        Ok(b) => b,
        Err(err) => return Ok(errors::result_from_engine_error(&EngineError::from(err))),
    };
    let target_id = recorder.target_id.get().cloned().unwrap_or_else(|| String::from("project"));

    if let Some(dest) = resolved_path {
        return write_atomic_export(&dest, &format, &target_id, &bytes);
    }

    format_inline_export_result(&format, &target_id, &bytes, &recorder)
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
        if let Err(err) = lumencraft_platform::fs::write_atomic(dest, bytes) {
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
    recorder: &ExportArtifactRecorder,
) -> Result<ToolCallResult, ToolCallError> {
    match format {
        "svg" | "csv" => {
            let text = String::from_utf8(bytes.to_vec()).map_err(|err| ToolCallError::Execution(format!("UTF-8 export decode failed: {err}")))?;
            Ok(ToolCallResult::ok_text(text))
        }
        "json" => {
            let text = String::from_utf8(bytes.to_vec()).map_err(|err| ToolCallError::Execution(format!("UTF-8 export decode failed: {err}")))?;
            let structured = serde_json::from_str::<Value>(&text).ok();
            Ok(ToolCallResult { content: vec![ContentBlock::Text { text }], is_error: false, structured_content: structured })
        }
        "png" => {
            let (width_px, height_px, dpi) = recorder.raster_meta.get().copied().unwrap_or((0, 0, DEFAULT_PREVIEW_DPI));
            let meta = json!({
                "format": "png",
                "sheet_id": target_id,
                "width_px": width_px,
                "height_px": height_px,
                "dpi": dpi,
                "bytes_len": bytes.len(),
            });
            let text = serde_json::to_string_pretty(&meta).unwrap_or_default();
            Ok(ToolCallResult {
                content: vec![ContentBlock::Image { data: encode_base64(bytes), mime_type: String::from("image/png") }, ContentBlock::Text { text }],
                is_error: false,
                structured_content: Some(meta),
            })
        }
        "pdf" => ToolCallResult::ok_json(&json!({
            "format": "pdf",
            "sheet_id": target_id,
            "encoding": "base64",
            "bytes_len": bytes.len(),
            "data": encode_base64(bytes),
        })),
        "tiff" => {
            let (width_px, height_px, dpi) = recorder.raster_meta.get().copied().unwrap_or((0, 0, DEFAULT_PREVIEW_DPI));
            ToolCallResult::ok_json(&json!({
                "format": "tiff",
                "sheet_id": target_id,
                "encoding": "base64",
                "width_px": width_px,
                "height_px": height_px,
                "dpi": dpi,
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
    all_graphs: bool,
    width_px: Option<u32>,
    height_px: Option<u32>,
    dpi: u16,
    tiff_compression: TiffCompression,
}

impl ExportArtifactConfig {
    fn parse(args: &Map<String, Value>, sheet_id: Option<String>) -> Result<Self, ToolCallError> {
        let pretty = optional_bool(args, "pretty")?.unwrap_or(true);
        let tidy = optional_bool(args, "tidy")?.unwrap_or(true);
        let tab_id = optional_non_empty_str(args, "tab_id")?;
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
        let all_graphs = optional_bool(args, "all_graphs")?.unwrap_or(false);
        let dpi = parse_dpi(args)?;
        let width_px = optional_u32_bounded(args, "width_px", 1, MAX_RASTER_DIM_PX)?;
        let height_px = optional_u32_bounded(args, "height_px", 1, MAX_RASTER_DIM_PX)?;
        let tiff_compression = match optional_non_empty_str(args, "tiff_compression")? {
            Some(raw) => parse_tiff_compression(&raw)?,
            None => TiffCompression::Lzw,
        };

        Ok(Self { sheet_id, pretty, tidy, tab_id, csv_opts, svg_precision, pdf_color_space, all_graphs, width_px, height_px, dpi, tiff_compression })
    }
}

#[derive(Default)]
struct ExportArtifactRecorder {
    target_id: OnceLock<String>,
    raster_meta: OnceLock<(u32, u32, u16)>,
}

fn build_mcp_export_registry(cfg: &ExportArtifactConfig, recorder: Arc<ExportArtifactRecorder>) -> FormatRegistry {
    let mut reg = FormatRegistry::default_registry();
    reg.register_exporter(Arc::new(McpSvgExporter { cfg: cfg.clone(), recorder: Arc::clone(&recorder) }));
    reg.register_exporter(Arc::new(McpPdfExporter { cfg: cfg.clone(), recorder: Arc::clone(&recorder) }));
    reg.register_exporter(Arc::new(McpPngExporter { cfg: cfg.clone(), recorder: Arc::clone(&recorder) }));
    reg.register_exporter(Arc::new(McpTiffExporter { cfg: cfg.clone(), recorder: Arc::clone(&recorder) }));
    reg.register_exporter(Arc::new(McpCsvExporter { cfg: cfg.clone(), recorder: Arc::clone(&recorder) }));
    reg.register_exporter(Arc::new(McpJsonExporter { cfg: cfg.clone(), recorder }));
    reg
}

struct McpSvgExporter {
    cfg: ExportArtifactConfig,
    recorder: Arc<ExportArtifactRecorder>,
}

impl Exporter for McpSvgExporter {
    fn descriptor(&self) -> FormatDescriptor {
        DESC_SVG
    }

    fn export_bytes(&self, source: ExportSource<'_>, _opts: &ExportOptions) -> Result<Vec<u8>, FormatError> {
        let doc = expect_document(source, DESC_SVG.id)?;
        let (target_id, _, scene) = compile_visual_sheet(doc, self.cfg.sheet_id.as_deref()).map_err(FormatError::Io)?;
        let _ = self.recorder.target_id.set(target_id);

        let mut opts = SvgExportOptions { pretty: self.cfg.pretty, ..SvgExportOptions::default() };
        if let Some(p) = self.cfg.svg_precision {
            opts.precision = p;
        }
        let svg = render_svg(&scene, &opts).map_err(|err| FormatError::Io(format!("SVG export failed: {err}")))?;
        Ok(svg.into_bytes())
    }
}

struct McpPdfExporter {
    cfg: ExportArtifactConfig,
    recorder: Arc<ExportArtifactRecorder>,
}

impl Exporter for McpPdfExporter {
    fn descriptor(&self) -> FormatDescriptor {
        DESC_PDF
    }

    fn export_bytes(&self, source: ExportSource<'_>, _opts: &ExportOptions) -> Result<Vec<u8>, FormatError> {
        let doc = expect_document(source, DESC_PDF.id)?;
        let mut opts = PdfExportOptions { color_space: self.cfg.pdf_color_space, ..PdfExportOptions::default() };

        let (target_id, pdf_bytes) = if self.cfg.all_graphs && self.cfg.sheet_id.is_none() {
            let total_sheets = doc.graph_sheets.len().saturating_add(doc.layout_sheets.len());
            if total_sheets > lumencraft_pdf::MAX_PDF_PAGES {
                return Err(FormatError::Io(format!("PDF page count {total_sheets} exceeds maximum {}", lumencraft_pdf::MAX_PDF_PAGES)));
            }
            let mut scenes = Vec::with_capacity(total_sheets);
            let mut total_primitives: usize = 0;
            for gs in &doc.graph_sheets {
                let scene = compile_graph(gs, doc).map_err(|err| FormatError::Io(format!("chart compile failed for '{}': {err}", gs.id)))?;
                total_primitives = total_primitives.saturating_add(scene.primitives.len());
                if total_primitives > lumencraft_pdf::MAX_PDF_TOTAL_PRIMITIVES {
                    return Err(FormatError::Io(format!(
                        "PDF primitive count {total_primitives} exceeds maximum {}",
                        lumencraft_pdf::MAX_PDF_TOTAL_PRIMITIVES
                    )));
                }
                scenes.push(scene);
            }
            for ls in &doc.layout_sheets {
                let scene = compile_layout(ls, doc).map_err(|err| FormatError::Io(format!("layout compile failed for '{}': {err}", ls.id)))?;
                total_primitives = total_primitives.saturating_add(scene.primitives.len());
                if total_primitives > lumencraft_pdf::MAX_PDF_TOTAL_PRIMITIVES {
                    return Err(FormatError::Io(format!(
                        "PDF primitive count {total_primitives} exceeds maximum {}",
                        lumencraft_pdf::MAX_PDF_TOTAL_PRIMITIVES
                    )));
                }
                scenes.push(scene);
            }
            if scenes.is_empty() {
                return Err(FormatError::Io(String::from("no GraphSheet or LayoutSheet available to export")));
            }
            opts.title = Some(doc.title.clone());
            let bytes = render_pdf_pages(&scenes, &opts).map_err(|err| FormatError::Io(format!("PDF export failed: {err}")))?;
            (String::from("all"), bytes)
        } else {
            let (target_id, _, scene) = compile_visual_sheet(doc, self.cfg.sheet_id.as_deref()).map_err(FormatError::Io)?;
            opts.title = Some(scene.title.clone());
            let bytes = render_pdf(&scene, &opts).map_err(|err| FormatError::Io(format!("PDF export failed: {err}")))?;
            (target_id, bytes)
        };

        let _ = self.recorder.target_id.set(target_id);
        Ok(pdf_bytes)
    }
}

struct McpPngExporter {
    cfg: ExportArtifactConfig,
    recorder: Arc<ExportArtifactRecorder>,
}

impl Exporter for McpPngExporter {
    fn descriptor(&self) -> FormatDescriptor {
        DESC_PNG
    }

    fn export_bytes(&self, source: ExportSource<'_>, _opts: &ExportOptions) -> Result<Vec<u8>, FormatError> {
        let doc = expect_document(source, DESC_PNG.id)?;
        let (target_id, _, scene) = compile_visual_sheet(doc, self.cfg.sheet_id.as_deref()).map_err(FormatError::Io)?;
        let spec = build_render_spec(&scene, self.cfg.width_px, self.cfg.height_px, self.cfg.dpi);
        let raster = render_scene(&scene, spec).map_err(|err| FormatError::Io(format!("rasterization failed: {err}")))?;
        let png_bytes = encode_png(&raster).map_err(|err| FormatError::Io(format!("PNG encoding failed: {err}")))?;
        let _ = self.recorder.target_id.set(target_id);
        let _ = self.recorder.raster_meta.set((raster.width_px, raster.height_px, raster.dpi));
        Ok(png_bytes)
    }
}

struct McpTiffExporter {
    cfg: ExportArtifactConfig,
    recorder: Arc<ExportArtifactRecorder>,
}

impl Exporter for McpTiffExporter {
    fn descriptor(&self) -> FormatDescriptor {
        DESC_TIFF
    }

    fn export_bytes(&self, source: ExportSource<'_>, _opts: &ExportOptions) -> Result<Vec<u8>, FormatError> {
        let doc = expect_document(source, DESC_TIFF.id)?;
        let (target_id, _, scene) = compile_visual_sheet(doc, self.cfg.sheet_id.as_deref()).map_err(FormatError::Io)?;
        let spec = build_render_spec(&scene, self.cfg.width_px, self.cfg.height_px, self.cfg.dpi);
        let raster = render_scene(&scene, spec).map_err(|err| FormatError::Io(format!("rasterization failed: {err}")))?;
        let tiff_bytes = encode_tiff(&raster, self.cfg.tiff_compression).map_err(|err| FormatError::Io(format!("TIFF encoding failed: {err}")))?;
        let _ = self.recorder.target_id.set(target_id);
        let _ = self.recorder.raster_meta.set((raster.width_px, raster.height_px, raster.dpi));
        Ok(tiff_bytes)
    }
}

struct McpCsvExporter {
    cfg: ExportArtifactConfig,
    recorder: Arc<ExportArtifactRecorder>,
}

impl Exporter for McpCsvExporter {
    fn descriptor(&self) -> FormatDescriptor {
        DESC_CSV
    }

    fn export_bytes(&self, source: ExportSource<'_>, _opts: &ExportOptions) -> Result<Vec<u8>, FormatError> {
        let doc = expect_document(source, DESC_CSV.id)?;
        let csv_opts = self.cfg.csv_opts.clone().unwrap_or_else(CsvOptions::csv);
        let (target_id, csv_text) = match resolve_tabular_export_target(doc, self.cfg.sheet_id.as_deref()).map_err(FormatError::Io)? {
            TabularTarget::Data(ds) => (ds.id.as_str().to_owned(), export_datasheet_to_csv(ds, &csv_opts)?),
            TabularTarget::Result(rs) => {
                let text = if self.cfg.tidy {
                    export_result_sheet_tidy_csv(rs, &csv_opts)
                } else if let Some(tid) = self.cfg.tab_id.as_deref() {
                    let tab = rs
                        .tabs
                        .iter()
                        .find(|t| t.id == tid)
                        .ok_or_else(|| FormatError::Io(format!("tab '{tid}' not found on result sheet '{}'", rs.id)))?;
                    export_result_tab_csv(tab, &csv_opts)
                } else if let Some(first_tab) = rs.tabs.first() {
                    export_result_tab_csv(first_tab, &csv_opts)
                } else {
                    export_result_sheet_tidy_csv(rs, &csv_opts)
                };
                (rs.id.as_str().to_owned(), text)
            }
        };
        let _ = self.recorder.target_id.set(target_id);
        Ok(csv_text.into_bytes())
    }
}

struct McpJsonExporter {
    cfg: ExportArtifactConfig,
    recorder: Arc<ExportArtifactRecorder>,
}

impl Exporter for McpJsonExporter {
    fn descriptor(&self) -> FormatDescriptor {
        DESC_JSON
    }

    fn export_bytes(&self, source: ExportSource<'_>, _opts: &ExportOptions) -> Result<Vec<u8>, FormatError> {
        let doc = expect_document(source, DESC_JSON.id)?;
        let (target_id, json_text) = if let Some(id_str) = self.cfg.sheet_id.as_deref() {
            let id = SheetId::new(id_str);
            if let Some(rs) = doc.result_sheet(&id) {
                (id_str.to_owned(), export_result_sheet_json(rs, self.cfg.pretty)?)
            } else if let Some(ds) = doc.data_sheet(&id) {
                (id_str.to_owned(), serialize_json_value(ds, self.cfg.pretty)?)
            } else if let Some(info) = doc.info_sheet(&id) {
                (id_str.to_owned(), serialize_json_value(info, self.cfg.pretty)?)
            } else if let Some(gs) = doc.graph_sheet(&id) {
                (id_str.to_owned(), serialize_json_value(gs, self.cfg.pretty)?)
            } else if let Some(ls) = doc.layout_sheet(&id) {
                (id_str.to_owned(), serialize_json_value(ls, self.cfg.pretty)?)
            } else {
                return Err(FormatError::Io(format!("sheet '{id_str}' not found")));
            }
        } else {
            (String::from("project"), serialize_json_value(doc, self.cfg.pretty)?)
        };

        let _ = self.recorder.target_id.set(target_id);
        Ok(json_text.into_bytes())
    }
}

fn expect_document<'a>(source: ExportSource<'a>, format_id: &str) -> Result<&'a ProjectDocument, FormatError> {
    match source {
        ExportSource::Document(doc) => Ok(doc),
        ExportSource::DataSheet(_) | ExportSource::ResultSheet(_) => {
            Err(FormatError::UnsupportedFormat { reason: format!("MCP format `{format_id}` requires a ProjectDocument source") })
        }
    }
}

fn serialize_json_value(val: &impl Serialize, pretty: bool) -> Result<String, FormatError> {
    if pretty { Ok(serde_json::to_string_pretty(val)?) } else { Ok(serde_json::to_string(val)?) }
}

fn compile_visual_sheet(doc: &ProjectDocument, sheet_id: Option<&str>) -> Result<(String, &'static str, ChartScene), String> {
    if let Some(id_str) = sheet_id {
        let id = SheetId::new(id_str);
        if let Some(gs) = doc.graph_sheet(&id) {
            let scene = compile_graph(gs, doc).map_err(|err| format!("failed to compile graph '{id_str}': {err}"))?;
            return Ok((id_str.to_owned(), "graph", scene));
        }
        if let Some(ls) = doc.layout_sheet(&id) {
            let scene = compile_layout(ls, doc).map_err(|err| format!("failed to compile layout '{id_str}': {err}"))?;
            return Ok((id_str.to_owned(), "layout", scene));
        }
        return Err(format!("graph or layout sheet '{id_str}' not found"));
    }

    if let Some(active) = &doc.active_sheet_id {
        if let Some(gs) = doc.graph_sheet(active) {
            let scene = compile_graph(gs, doc).map_err(|err| format!("failed to compile graph '{}': {err}", active.as_str()))?;
            return Ok((active.as_str().to_owned(), "graph", scene));
        }
        if let Some(ls) = doc.layout_sheet(active) {
            let scene = compile_layout(ls, doc).map_err(|err| format!("failed to compile layout '{}': {err}", active.as_str()))?;
            return Ok((active.as_str().to_owned(), "layout", scene));
        }
    }

    if let Some(gs) = doc.graph_sheets.first() {
        let scene = compile_graph(gs, doc).map_err(|err| format!("failed to compile graph '{}': {err}", gs.id.as_str()))?;
        return Ok((gs.id.as_str().to_owned(), "graph", scene));
    }
    if let Some(ls) = doc.layout_sheets.first() {
        let scene = compile_layout(ls, doc).map_err(|err| format!("failed to compile layout '{}': {err}", ls.id.as_str()))?;
        return Ok((ls.id.as_str().to_owned(), "layout", scene));
    }

    Err(String::from("no GraphSheet or LayoutSheet exists in the current document"))
}

enum TabularTarget<'a> {
    Data(&'a DataSheet),
    Result(&'a ResultSheet),
}

fn resolve_tabular_export_target<'a>(doc: &'a ProjectDocument, sheet_id: Option<&str>) -> Result<TabularTarget<'a>, String> {
    if let Some(id_str) = sheet_id {
        let id = SheetId::new(id_str);
        if let Some(rs) = doc.result_sheet(&id) {
            return Ok(TabularTarget::Result(rs));
        }
        if let Some(ds) = doc.data_sheet(&id) {
            return Ok(TabularTarget::Data(ds));
        }
        return Err(format!("data or result sheet '{id_str}' not found"));
    }

    if let Some(active) = &doc.active_sheet_id {
        if let Some(rs) = doc.result_sheet(active) {
            return Ok(TabularTarget::Result(rs));
        }
        if let Some(ds) = doc.data_sheet(active) {
            return Ok(TabularTarget::Data(ds));
        }
    }

    if let Some(rs) = doc.result_sheets.first() {
        return Ok(TabularTarget::Result(rs));
    }
    if let Some(ds) = doc.data_sheets.first() {
        return Ok(TabularTarget::Data(ds));
    }

    Err(String::from("no DataSheet or ResultSheet exists in the current document"))
}

fn build_render_spec(scene: &ChartScene, requested_w: Option<u32>, requested_h: Option<u32>, dpi: u16) -> RenderTargetSpec {
    let default_w = if scene.width_pt.is_finite() && scene.width_pt > 0.0 {
        let scaled = (scene.width_pt * f64::from(dpi) / 72.0).round();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let clamped = scaled.clamp(64.0, f64::from(MAX_RASTER_DIM_PX)) as u32;
        clamped
    } else {
        DEFAULT_PREVIEW_WIDTH_PX
    };
    let default_h = if scene.height_pt.is_finite() && scene.height_pt > 0.0 {
        let scaled = (scene.height_pt * f64::from(dpi) / 72.0).round();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let clamped = scaled.clamp(64.0, f64::from(MAX_RASTER_DIM_PX)) as u32;
        clamped
    } else {
        DEFAULT_PREVIEW_HEIGHT_PX
    };

    RenderTargetSpec { width_px: requested_w.unwrap_or(default_w), height_px: requested_h.unwrap_or(default_h), dpi }
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
