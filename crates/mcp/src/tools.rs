//! MCP tool schemas, server state management, live desktop `--control` attach client,
//! headless `Session` handlers, PNG/TIFF/PDF/SVG/CSV/JSON exporters, and RFC 4648 base64 encoding.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use lumencraft_chart::{ChartScene, compile_graph, compile_layout};
use lumencraft_doc::{DataSheet, ProjectDocument, ResultSheet, SheetId, SheetSection, SubcolumnMode, TableKind};
use lumencraft_engine::Session;
use lumencraft_engine::catalog::COMMANDS;
use lumencraft_format::{
    CsvOptions, export_datasheet_to_csv, export_result_sheet_json, export_result_sheet_tidy_csv, export_result_tab_csv, import_csv_to_datasheet,
    import_prism_bytes, import_pzfx_bytes, instantiate_template_bytes, open_project_file, save_project_file, save_template_file,
};
use lumencraft_pdf::{PdfColorSpace, PdfExportOptions, render_pdf, render_pdf_pages};
use lumencraft_render::{RenderTargetSpec, TiffCompression, encode_png, encode_tiff, render_scene};
use lumencraft_svg::{SvgExportOptions, render_svg};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::ui_catalog::UI_COMMANDS;

const DEFAULT_PREVIEW_WIDTH_PX: u32 = 960;
const DEFAULT_PREVIEW_HEIGHT_PX: u32 = 640;
const DEFAULT_PREVIEW_DPI: u16 = 144;
const MAX_RASTER_DIM_PX: u32 = 8192;
const MAX_DPI: u16 = 2400;
const MIN_DPI: u16 = 18;
const MAX_BATCH_COMMANDS: usize = 1024;

/// Stateful execution context shared across requests on an MCP server connection.
#[derive(Debug, Clone)]
pub struct McpServerState {
    session: Session,
    #[cfg(not(target_arch = "wasm32"))]
    control_port: Option<u16>,
    initialized: bool,
}

impl Default for McpServerState {
    fn default() -> Self {
        Self::new()
    }
}

impl McpServerState {
    /// Construct a fresh headless MCP server state backed by an empty [`Session`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            session: Session::new(),
            #[cfg(not(target_arch = "wasm32"))]
            control_port: None,
            initialized: false,
        }
    }

    /// Wrap an existing headless [`Session`].
    #[must_use]
    pub fn from_session(session: Session) -> Self {
        Self {
            session,
            #[cfg(not(target_arch = "wasm32"))]
            control_port: None,
            initialized: false,
        }
    }

    /// Construct an MCP server state attached to a running desktop window listening on
    /// `127.0.0.1:<port>`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_control_port(port: u16) -> Result<Self, String> {
        if port == 0 {
            return Err(String::from("control port must be in 1..=65535"));
        }
        Ok(Self { session: Session::new(), control_port: Some(port), initialized: false })
    }

    /// Borrow the underlying headless [`Session`].
    #[must_use]
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Mutably borrow the underlying headless [`Session`].
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// Configured live desktop control port on `127.0.0.1`, if attached.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn control_port(&self) -> Option<u16> {
        self.control_port
    }

    /// Live desktop attach is unavailable on `wasm32`.
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn control_port(&self) -> Option<u16> {
        None
    }

    /// Attach or detach a live desktop control port on `127.0.0.1`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_control_port(&mut self, port: Option<u16>) -> Result<(), String> {
        if port == Some(0) {
            return Err(String::from("control port must be in 1..=65535"));
        }
        self.control_port = port;
        Ok(())
    }

    /// Record whether MCP `initialize` / `notifications/initialized` has been observed.
    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Mark this server session as initialized.
    pub fn mark_initialized(&mut self) {
        self.initialized = true;
    }
}

/// JSON Schema descriptor exposed in MCP `tools/list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// Content item returned inside an MCP `tools/call` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
}

/// Result envelope returned by MCP `tools/call`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallResult {
    pub content: Vec<ContentBlock>,
    #[serde(rename = "isError")]
    pub is_error: bool,
}

impl ToolCallResult {
    /// Construct a non-error text result.
    #[must_use]
    pub fn ok_text(text: impl Into<String>) -> Self {
        Self { content: vec![ContentBlock::Text { text: text.into() }], is_error: false }
    }

    /// Construct a non-error pretty-printed JSON text result.
    pub fn ok_json(value: &impl Serialize) -> Result<Self, ToolCallError> {
        let text = serde_json::to_string_pretty(value).map_err(|err| ToolCallError::Execution(format!("JSON serialization error: {err}")))?;
        Ok(Self::ok_text(text))
    }

    /// Construct a domain/execution error result (`isError: true`).
    #[must_use]
    pub fn error_text(message: impl Into<String>) -> Self {
        Self { content: vec![ContentBlock::Text { text: message.into() }], is_error: true }
    }
}

/// Protocol-level or execution-level failure returned by [`call_tool`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallError {
    /// Tool name is not registered in [`list_tools`].
    UnknownTool(String),
    /// Caller supplied malformed or out-of-range tool arguments (`-32602`).
    InvalidParams(String),
    /// Domain, file I/O, or engine execution error (surfaced as MCP `isError: true`).
    Execution(String),
}

impl fmt::Display for ToolCallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTool(name) => write!(f, "unknown tool: {name}"),
            Self::InvalidParams(msg) | Self::Execution(msg) => f.write_str(msg),
        }
    }
}

/// Return all registered MCP tool definitions with full JSON Schemas.
#[must_use]
pub fn list_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: String::from("doc_inspect"),
            description: String::from("Inspect the active project's sheets, variables, hooks, analyses, graphs, and layouts."),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sheet_id": { "type": "string", "description": "Optional specific sheet ID to inspect in full detail." },
                    "include_cells": { "type": "boolean", "description": "Include preview cell contents when inspecting a single sheet." }
                },
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: String::from("open_file"),
            description: String::from("Open or import a .lumencraft, .lctemplate, .prism, .pzfx, .csv, or .tsv file into the session."),
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string", "description": "Filesystem path to open or import." },
                    "format": { "type": "string", "enum": ["lumencraft", "lctemplate", "prism", "pzfx", "csv", "tsv"] },
                    "target_kind": { "type": "string", "enum": ["xy", "column", "grouped", "contingency", "survival", "parts_of_whole", "multiple_variables", "nested"] },
                    "subcolumn_mode": { "type": "string" },
                    "delimiter": { "type": "string", "description": "Optional single-character delimiter override for delimited text." },
                    "sheet_id": { "type": "string", "description": "Sheet ID used when importing delimited data." },
                    "title": { "type": "string", "description": "Optional title override for imported sheet or instantiated template." },
                    "append": { "type": "boolean", "description": "When importing CSV/TSV, append as a new sheet into the active project instead of replacing it." }
                },
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: String::from("save_file"),
            description: String::from("Save the current document as a .lumencraft archive or .lctemplate template."),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Destination file path (defaults to current project path when available)." },
                    "format": { "type": "string", "enum": ["lumencraft", "lctemplate"] }
                },
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: String::from("command_list"),
            description: String::from("List all headless engine commands and desktop UI commands (marked with \"ui\": true)."),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "filter": { "type": "string", "description": "Case-insensitive substring filter across id, label, and menu_path." },
                    "include_ui": { "type": "boolean", "description": "Include UI-only commands from ui_catalog (default: true)." },
                    "ui_only": { "type": "boolean", "description": "Return only UI-only commands (default: false)." }
                },
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: String::from("command_run"),
            description: String::from("Execute a single engine command (or live UI command when attached to --control) with JSON parameters."),
            input_schema: json!({
                "type": "object",
                "required": ["command_id"],
                "properties": {
                    "command_id": { "type": "string", "description": "Catalog command identifier (e.g. sheet.createData, table.setCell, fit.run)." },
                    "params": { "type": "object", "description": "Command parameter payload object." }
                },
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: String::from("command_batch"),
            description: String::from("Execute a sequence of commands, optionally with atomic rollback if any step fails."),
            input_schema: json!({
                "type": "object",
                "required": ["commands"],
                "properties": {
                    "commands": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["command_id"],
                            "properties": {
                                "command_id": { "type": "string" },
                                "params": { "type": "object" }
                            }
                        }
                    },
                    "atomic": { "type": "boolean", "description": "Roll back all mutations if any command in the batch fails (default: false)." }
                },
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: String::from("render_preview"),
            description: String::from("Compile a GraphSheet or LayoutSheet and rasterize a PNG preview with metadata."),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sheet_id": { "type": "string", "description": "GraphSheet or LayoutSheet ID (defaults to active or first graph/layout)." },
                    "width_px": { "type": "integer", "minimum": 1, "maximum": 8192 },
                    "height_px": { "type": "integer", "minimum": 1, "maximum": 8192 },
                    "dpi": { "type": "integer", "minimum": 18, "maximum": 2400 }
                },
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: String::from("export"),
            description: String::from("Export a sheet or project to svg, pdf, png, tiff, csv, or json (to disk or inline)."),
            input_schema: json!({
                "type": "object",
                "required": ["format"],
                "properties": {
                    "format": { "type": "string", "enum": ["svg", "pdf", "png", "tiff", "csv", "json"] },
                    "sheet_id": { "type": "string" },
                    "output_path": { "type": "string" },
                    "path": { "type": "string", "description": "Alias for output_path." },
                    "width_px": { "type": "integer", "minimum": 1, "maximum": 8192 },
                    "height_px": { "type": "integer", "minimum": 1, "maximum": 8192 },
                    "dpi": { "type": "integer", "minimum": 18, "maximum": 2400 },
                    "tiff_compression": { "type": "string", "enum": ["none", "deflate", "lzw"] },
                    "pdf_color_space": { "type": "string", "enum": ["srgb", "cmyk", "grayscale"] },
                    "all_graphs": { "type": "boolean" },
                    "svg_precision": { "type": "integer", "minimum": 1, "maximum": 8 },
                    "pretty": { "type": "boolean" },
                    "tidy": { "type": "boolean" },
                    "tab_id": { "type": "string" },
                    "csv_delimiter": { "type": "string" }
                },
                "additionalProperties": false
            }),
        },
    ]
}

/// Dispatch an MCP `tools/call` invocation onto `state`.
pub fn call_tool(state: &mut McpServerState, name: &str, arguments: Value) -> Result<ToolCallResult, ToolCallError> {
    let args = arguments.as_object().ok_or_else(|| ToolCallError::InvalidParams(String::from("tool arguments must be an object")))?;

    match name {
        "doc_inspect" => handle_doc_inspect(state, args),
        "open_file" => handle_open_file(state, args),
        "save_file" => handle_save_file(state, args),
        "command_list" => handle_command_list(args),
        "command_run" => handle_command_run(state, args),
        "command_batch" => handle_command_batch(state, args),
        "render_preview" => handle_render_preview(state, args),
        "export" => handle_export(state, args),
        _ => Err(ToolCallError::UnknownTool(name.to_owned())),
    }
}

fn handle_doc_inspect(state: &McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let sheet_id = optional_non_empty_str(args, "sheet_id")?;
    let include_cells = optional_bool(args, "include_cells")?.unwrap_or(false);

    #[cfg(not(target_arch = "wasm32"))]
    if let Some(port) = state.control_port()
        && sheet_id.is_none()
    {
        return match live_control::send_control(port, "state.snapshot", json!({})) {
            Ok(snapshot) => ToolCallResult::ok_json(&json!({
                "mode": "live_attach",
                "control_port": port,
                "snapshot": snapshot,
            })),
            Err(err) => Ok(ToolCallResult::error_text(err)),
        };
    }

    let doc = state.session.document();
    if let Some(target_id) = sheet_id {
        let target = SheetId::new(target_id);
        return inspect_single_sheet(doc, &target, include_cells);
    }

    let summary = build_document_summary(&state.session);
    ToolCallResult::ok_json(&summary)
}

fn inspect_single_sheet(doc: &ProjectDocument, target: &SheetId, include_cells: bool) -> Result<ToolCallResult, ToolCallError> {
    let deps = doc.upstream_dependencies(target);
    let Some(section) = doc.sheet_section(target) else {
        return Ok(ToolCallResult::error_text(format!("sheet '{}' not found", target.as_str())));
    };

    let detail = match section {
        SheetSection::Data => doc.data_sheet(target).map(|ds| {
            let mut val = json!({
                "section": "data",
                "id": ds.id,
                "title": ds.title,
                "kind": ds.kind().label(),
                "frozen": ds.frozen,
                "info_sheet_id": ds.info_sheet_id,
                "rows": ds.archetype.row_count(),
                "columns": ds.archetype.column_count(),
                "linked_regions": ds.linked_regions,
                "upstream_dependencies": deps,
            });
            if include_cells && let Some(obj) = val.as_object_mut() {
                let csv = export_datasheet_to_csv(ds, &CsvOptions::csv()).unwrap_or_default();
                obj.insert(String::from("csv_preview"), Value::String(csv));
            }
            val
        }),
        SheetSection::Info => doc.info_sheet(target).map(|info| {
            json!({
                "section": "info",
                "id": info.id,
                "title": info.title,
                "linked_data_sheets": info.linked_data_sheets,
                "constants": info.constants,
                "notes_markdown": info.notes_markdown,
                "upstream_dependencies": deps,
            })
        }),
        SheetSection::Analyses => doc.result_sheet(target).map(|rs| {
            let mut val = summarize_result_sheet(rs, &deps);
            if include_cells && let Some(obj) = val.as_object_mut() {
                obj.insert(String::from("tabs"), serde_json::to_value(&rs.tabs).unwrap_or(Value::Null));
                obj.insert(String::from("checklist"), serde_json::to_value(&rs.checklist).unwrap_or(Value::Null));
            }
            val
        }),
        SheetSection::Graphs => doc.graph_sheet(target).map(|gs| {
            json!({
                "section": "graphs",
                "id": gs.id,
                "title": gs.title,
                "initialized": gs.initialized,
                "spec": gs.spec,
                "upstream_dependencies": deps,
            })
        }),
        SheetSection::Layouts => doc.layout_sheet(target).map(|ls| {
            json!({
                "section": "layouts",
                "id": ls.id,
                "title": ls.title,
                "spec": ls.spec,
                "upstream_dependencies": deps,
            })
        }),
    };

    match detail {
        Some(value) => ToolCallResult::ok_json(&value),
        None => Ok(ToolCallResult::error_text(format!("sheet '{}' not found", target.as_str()))),
    }
}

fn build_document_summary(session: &Session) -> Value {
    let doc = session.document();
    let data_list: Vec<Value> = doc
        .data_sheets
        .iter()
        .map(|ds| {
            json!({
                "id": ds.id,
                "title": ds.title,
                "kind": ds.kind().label(),
                "frozen": ds.frozen,
                "info_sheet_id": ds.info_sheet_id,
                "rows": ds.archetype.row_count(),
                "columns": ds.archetype.column_count(),
                "linked_regions_count": ds.linked_regions.len(),
                "upstream_dependencies": doc.upstream_dependencies(&ds.id),
            })
        })
        .collect();

    let info_list: Vec<Value> = doc
        .info_sheets
        .iter()
        .map(|info| {
            json!({
                "id": info.id,
                "title": info.title,
                "linked_data_sheets": info.linked_data_sheets,
                "constants": info.constants,
                "notes_markdown_len": info.notes_markdown.len(),
            })
        })
        .collect();

    let analyses_list: Vec<Value> = doc.result_sheets.iter().map(|rs| summarize_result_sheet(rs, &doc.upstream_dependencies(&rs.id))).collect();

    let graphs_list: Vec<Value> = doc
        .graph_sheets
        .iter()
        .map(|gs| {
            json!({
                "id": gs.id,
                "title": gs.title,
                "kind": gs.spec.kind,
                "primary_source_sheet_id": gs.spec.primary_source_sheet_id,
                "initialized": gs.initialized,
                "canvas_size_pt": gs.spec.canvas_size_pt,
                "frame_preset": gs.spec.frame_preset,
                "series_count": gs.spec.series.len(),
                "annotations_count": gs.spec.annotations.len(),
                "comparisons_count": gs.spec.comparisons.len(),
                "upstream_dependencies": doc.upstream_dependencies(&gs.id),
            })
        })
        .collect();

    let layouts_list: Vec<Value> = doc
        .layout_sheets
        .iter()
        .map(|ls| {
            json!({
                "id": ls.id,
                "title": ls.title,
                "width": ls.spec.width,
                "height": ls.spec.height,
                "unit": ls.spec.unit,
                "page_size_pt": ls.spec.page_size_pt(),
                "orientation": ls.spec.orientation,
                "items_count": ls.spec.items.len(),
                "upstream_dependencies": doc.upstream_dependencies(&ls.id),
            })
        })
        .collect();

    let (variables, hooks) = collect_variables_and_hooks(doc);

    json!({
        "title": doc.title,
        "schema_version": doc.schema_version,
        "file_path": session.file_path().map(|p| p.display().to_string()),
        "dirty": session.is_dirty(),
        "total_sheet_count": doc.total_sheet_count(),
        "active_sheet_id": doc.active_sheet_id,
        "family_anchor_sheet_id": doc.family_anchor_sheet_id,
        "sheets": {
            "data": data_list,
            "info": info_list,
            "analyses": analyses_list,
            "graphs": graphs_list,
            "layouts": layouts_list,
        },
        "variables": variables,
        "hooks": hooks,
        "analyses": analyses_list,
        "graphs": graphs_list,
        "layouts": layouts_list,
    })
}

fn summarize_result_sheet(rs: &ResultSheet, deps: &[SheetId]) -> Value {
    json!({
        "section": "analyses",
        "id": rs.id,
        "title": rs.title(),
        "source_sheet_id": rs.analysis.source_sheet_id,
        "selected_data_sets": rs.analysis.selected_data_sets,
        "status": rs.status,
        "has_analyzable_data": rs.analyzable_data.is_some(),
        "tabs_count": rs.tabs.len(),
        "scalars": rs.scalars,
        "pairwise_comparisons_count": rs.pairwise_comparisons.len(),
        "fitted_curves_count": rs.fitted_curves.len(),
        "checklist_count": rs.checklist.len(),
        "upstream_dependencies": deps,
    })
}

fn collect_variables_and_hooks(doc: &ProjectDocument) -> (Value, Vec<Value>) {
    let mut info_constants = Vec::new();
    for info in &doc.info_sheets {
        for c in &info.constants {
            info_constants.push(json!({
                "info_sheet_id": info.id,
                "name": c.name,
                "value": c.value,
                "units": c.units,
            }));
        }
    }

    let mut result_scalars = Vec::new();
    let mut hooks = Vec::new();
    for rs in &doc.result_sheets {
        for s in &rs.scalars {
            result_scalars.push(json!({
                "result_sheet_id": rs.id,
                "key": s.key,
                "data_set_index": s.data_set_index,
                "value": s.value,
                "units": s.units,
            }));
        }
        for hook in rs.analysis.kind.hooked_sources() {
            hooks.push(json!({ "consumer_sheet_id": rs.id, "consumer_section": "analyses", "hook": hook }));
        }
    }

    for gs in &doc.graph_sheets {
        for hook in gs.spec.inline_hooks() {
            hooks.push(json!({ "consumer_sheet_id": gs.id, "consumer_section": "graphs", "hook": hook }));
        }
    }
    for ls in &doc.layout_sheets {
        for hook in ls.spec.inline_hooks() {
            hooks.push(json!({ "consumer_sheet_id": ls.id, "consumer_section": "layouts", "hook": hook }));
        }
    }

    let variables = json!({
        "info_constants": info_constants,
        "result_scalars": result_scalars,
    });
    (variables, hooks)
}

fn handle_open_file(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let path_str = required_non_empty_str(args, "path")?;
    let path = PathBuf::from(&path_str);
    let format = resolve_open_format(&path, args)?;

    match format.as_str() {
        "lumencraft" => {
            let doc = match open_project_file(&path) {
                Ok(doc) => doc,
                Err(err) => return Ok(ToolCallResult::error_text(format!("failed to open '{}': {err}", path.display()))),
            };
            let sheets = doc.total_sheet_count();
            let title = doc.title.clone();
            state.session = Session::from_document(doc);
            state.session.set_file_path(Some(path.clone()), true);
            ToolCallResult::ok_json(&json!({
                "opened": true,
                "format": "lumencraft",
                "path": path.display().to_string(),
                "title": title,
                "total_sheet_count": sheets,
            }))
        }
        "lctemplate" => {
            let title_override = optional_non_empty_str(args, "title")?;
            let bytes = match fs::read(&path) {
                Ok(b) => b,
                Err(err) => return Ok(ToolCallResult::error_text(format!("failed to read '{}': {err}", path.display()))),
            };
            let doc = match instantiate_template_bytes(&bytes, title_override.as_deref()) {
                Ok(doc) => doc,
                Err(err) => return Ok(ToolCallResult::error_text(format!("failed to instantiate template '{}': {err}", path.display()))),
            };
            let sheets = doc.total_sheet_count();
            let title = doc.title.clone();
            state.session = Session::from_document(doc);
            state.session.set_file_path(None, true);
            ToolCallResult::ok_json(&json!({
                "opened": true,
                "format": "lctemplate",
                "path": path.display().to_string(),
                "title": title,
                "total_sheet_count": sheets,
            }))
        }
        "prism" | "pzfx" => open_cleanroom_import(state, &path, &format),
        "csv" | "tsv" => open_delimited_file(state, &path, &format, args),
        other => Err(ToolCallError::InvalidParams(format!("unsupported open format '{other}'"))),
    }
}

fn open_cleanroom_import(state: &mut McpServerState, path: &Path, format: &str) -> Result<ToolCallResult, ToolCallError> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(err) => return Ok(ToolCallResult::error_text(format!("failed to read '{}': {err}", path.display()))),
    };
    let report = if format == "prism" { import_prism_bytes(&bytes) } else { import_pzfx_bytes(&bytes) };
    let report = match report {
        Ok(r) => r,
        Err(err) => return Ok(ToolCallResult::error_text(format!("failed to import '{}': {err}", path.display()))),
    };
    let sheets = report.document.total_sheet_count();
    let title = report.document.title.clone();
    state.session = Session::from_document(report.document);
    state.session.set_file_path(None, false);
    ToolCallResult::ok_json(&json!({
        "opened": true,
        "format": format,
        "path": path.display().to_string(),
        "title": title,
        "total_sheet_count": sheets,
        "warnings": report.warnings,
    }))
}

fn open_delimited_file(state: &mut McpServerState, path: &Path, format: &str, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let text = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(err) => return Ok(ToolCallResult::error_text(format!("failed to read '{}': {err}", path.display()))),
    };
    let target_kind = match optional_non_empty_str(args, "target_kind")? {
        Some(raw) => parse_table_kind(&raw)?,
        None => TableKind::Xy,
    };
    let subcolumn_mode = match optional_non_empty_str(args, "subcolumn_mode")? {
        Some(raw) => parse_subcolumn_mode(&raw)?,
        None => SubcolumnMode::Single,
    };
    let mut opts = if format == "tsv" { CsvOptions::tsv() } else { CsvOptions::detect(&text) };
    if let Some(delim) = optional_non_empty_str(args, "delimiter")? {
        let mut chars = delim.chars();
        let Some(ch) = chars.next() else {
            return Err(ToolCallError::InvalidParams(String::from("delimiter must not be empty")));
        };
        if chars.next().is_some() {
            return Err(ToolCallError::InvalidParams(String::from("delimiter must be a single character")));
        }
        opts.delimiter = ch;
    }

    let append = optional_bool(args, "append")?.unwrap_or(false);
    let stem = path.file_stem().and_then(|s| s.to_str()).filter(|s| !s.trim().is_empty()).unwrap_or("Imported Data");
    let sheet_title = optional_non_empty_str(args, "title")?.unwrap_or_else(|| stem.to_owned());
    let sheet_id_str = optional_non_empty_str(args, "sheet_id")?.unwrap_or_else(|| sanitize_sheet_id(stem, "data-1"));

    let ds = match import_csv_to_datasheet(sheet_id_str.clone(), sheet_title.clone(), &text, target_kind, subcolumn_mode, &opts) {
        Ok(ds) => ds,
        Err(err) => return Ok(ToolCallResult::error_text(format!("failed to parse '{}': {err}", path.display()))),
    };
    let rows = ds.archetype.row_count();
    let cols = ds.archetype.column_count();

    if append {
        let mut doc = state.session.document().clone();
        if let Err(err) = doc.add_data_sheet(ds) {
            return Ok(ToolCallResult::error_text(format!("failed to add imported sheet: {err}")));
        }
        let is_dirty = state.session.is_dirty();
        let existing_path = state.session.file_path().map(Path::to_path_buf);
        state.session = Session::from_document(doc);
        state.session.set_file_path(existing_path, !is_dirty);
    } else {
        let mut doc = match ProjectDocument::new(sheet_title.clone()) {
            Ok(d) => d,
            Err(err) => return Ok(ToolCallResult::error_text(format!("invalid document title: {err}"))),
        };
        if let Err(err) = doc.add_data_sheet(ds) {
            return Ok(ToolCallResult::error_text(format!("failed to register imported sheet: {err}")));
        }
        state.session = Session::from_document(doc);
        state.session.set_file_path(None, false);
    }

    ToolCallResult::ok_json(&json!({
        "opened": true,
        "format": format,
        "path": path.display().to_string(),
        "sheet_id": sheet_id_str,
        "title": sheet_title,
        "table_kind": target_kind.label(),
        "rows": rows,
        "columns": cols,
    }))
}

fn handle_save_file(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let explicit_path = optional_non_empty_str(args, "path")?.map(PathBuf::from);
    let path = match explicit_path.or_else(|| state.session.file_path().map(Path::to_path_buf)) {
        Some(p) => p,
        None => {
            return Err(ToolCallError::InvalidParams(String::from("save_file requires \"path\" when session has no associated file_path")));
        }
    };

    let format = resolve_save_format(&path, args)?;
    let doc = state.session.document().clone();
    let result = match format.as_str() {
        "lumencraft" => save_project_file(&path, &doc),
        "lctemplate" => save_template_file(&path, &doc),
        other => return Err(ToolCallError::InvalidParams(format!("unsupported save format '{other}'"))),
    };

    if let Err(err) = result {
        return Ok(ToolCallResult::error_text(format!("failed to save '{}': {err}", path.display())));
    }

    if format == "lumencraft" {
        state.session.set_file_path(Some(path.clone()), true);
    }

    ToolCallResult::ok_json(&json!({
        "saved": true,
        "format": format,
        "path": path.display().to_string(),
        "schema_version": doc.schema_version,
        "total_sheet_count": doc.total_sheet_count(),
    }))
}

fn handle_command_list(args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let filter = optional_non_empty_str(args, "filter")?.map(|s| s.to_ascii_lowercase());
    let include_ui = optional_bool(args, "include_ui")?.unwrap_or(true);
    let ui_only = optional_bool(args, "ui_only")?.unwrap_or(false);

    let mut commands = Vec::new();
    if !ui_only {
        for desc in COMMANDS {
            if matches_filter(filter.as_deref(), desc.id, desc.label, desc.menu_path) {
                commands.push(json!({
                    "id": desc.id,
                    "label": desc.label,
                    "menu_path": desc.menu_path,
                    "shortcut": desc.shortcut,
                    "params_doc": desc.params_doc,
                    "ui": false,
                }));
            }
        }
    }

    if include_ui || ui_only {
        for &(id, label, menu_path, shortcut) in UI_COMMANDS {
            if matches_filter(filter.as_deref(), id, label, menu_path) {
                commands.push(json!({
                    "id": id,
                    "label": label,
                    "menu_path": menu_path,
                    "shortcut": shortcut,
                    "params_doc": "{}",
                    "ui": true,
                }));
            }
        }
    }

    ToolCallResult::ok_json(&json!({
        "count": commands.len(),
        "commands": commands,
    }))
}

fn matches_filter(filter: Option<&str>, id: &str, label: &str, menu_path: &str) -> bool {
    let Some(needle) = filter else {
        return true;
    };
    id.to_ascii_lowercase().contains(needle) || label.to_ascii_lowercase().contains(needle) || menu_path.to_ascii_lowercase().contains(needle)
}

fn handle_command_run(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let command_id = required_non_empty_str(args, "command_id")?;
    let params = extract_command_params(args)?;
    match execute_single_command(state, &command_id, params) {
        Ok(val) => ToolCallResult::ok_json(&val),
        Err(msg) => Ok(ToolCallResult::error_text(msg)),
    }
}

fn handle_command_batch(state: &mut McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let Some(items) = args.get("commands").and_then(Value::as_array) else {
        return Err(ToolCallError::InvalidParams(String::from("command_batch requires an array \"commands\"")));
    };
    if items.len() > MAX_BATCH_COMMANDS {
        return Err(ToolCallError::InvalidParams(format!("command_batch exceeds maximum {MAX_BATCH_COMMANDS} items")));
    }

    let atomic = optional_bool(args, "atomic")?.unwrap_or(false);
    let mut parsed_items = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let Some(obj) = item.as_object() else {
            return Err(ToolCallError::InvalidParams(format!("batch item {idx} must be an object")));
        };
        let command_id = required_non_empty_str(obj, "command_id")
            .map_err(|_| ToolCallError::InvalidParams(format!("batch item {idx} requires non-empty string \"command_id\"")))?;
        let params =
            extract_command_params(obj).map_err(|_| ToolCallError::InvalidParams(format!("batch item {idx} \"params\" must be an object")))?;
        parsed_items.push((command_id, params));
    }

    execute_batch_items(state, parsed_items, atomic)
}

fn execute_batch_items(state: &mut McpServerState, items: Vec<(String, Value)>, atomic: bool) -> Result<ToolCallResult, ToolCallError> {
    let snapshot = if atomic && !is_attached(state) { Some(state.session.clone()) } else { None };
    let mut outcomes = Vec::with_capacity(items.len());
    let mut remote_mutated_steps: usize = 0;

    for (idx, (command_id, params)) in items.into_iter().enumerate() {
        match execute_single_command(state, &command_id, params) {
            Ok(outcome) => {
                if outcome.get("mutated").and_then(Value::as_bool) == Some(true) {
                    remote_mutated_steps = remote_mutated_steps.saturating_add(1);
                }
                outcomes.push(json!({ "index": idx, "ok": true, "outcome": outcome }));
            }
            Err(err_msg) => {
                if atomic {
                    let rolled_back = rollback_batch(state, snapshot, remote_mutated_steps);
                    let payload = json!({
                        "atomic": true,
                        "rolled_back": rolled_back,
                        "failed_index": idx,
                        "failed_command_id": command_id,
                        "error": err_msg,
                        "completed_before_failure": outcomes,
                    });
                    let text = serde_json::to_string_pretty(&payload).unwrap_or(err_msg);
                    return Ok(ToolCallResult::error_text(text));
                }
                outcomes.push(json!({ "index": idx, "ok": false, "command_id": command_id, "error": err_msg }));
                let payload = json!({
                    "atomic": false,
                    "completed": idx,
                    "failed_index": idx,
                    "results": outcomes,
                });
                let text = serde_json::to_string_pretty(&payload).unwrap_or(err_msg);
                return Ok(ToolCallResult::error_text(text));
            }
        }
    }

    ToolCallResult::ok_json(&json!({
        "atomic": atomic,
        "count": outcomes.len(),
        "results": outcomes,
    }))
}

fn rollback_batch(state: &mut McpServerState, snapshot: Option<Session>, remote_mutated_steps: usize) -> bool {
    if let Some(prev) = snapshot {
        state.session = prev;
        return true;
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(port) = state.control_port() {
        let mut all_undone = true;
        for _ in 0..remote_mutated_steps {
            if live_control::send_control(port, "engine.execute", json!({ "command_id": "edit.undo", "params": {} })).is_err() {
                all_undone = false;
                break;
            }
        }
        return all_undone;
    }
    let _ = remote_mutated_steps;
    false
}

fn execute_single_command(state: &mut McpServerState, command_id: &str, params: Value) -> Result<Value, String> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(port) = state.control_port() {
        return execute_remote_command(port, command_id, params);
    }

    if is_ui_catalog_command(command_id) {
        return Err(format!("UI command '{command_id}' requires attaching to a live desktop --control <port>"));
    }

    let outcome = state.session.execute(command_id, params).map_err(|err| format!("command '{command_id}' failed: {}", err.message()))?;
    Ok(json!({
        "command_id": outcome.command_id,
        "mutated": outcome.mutated,
        "message": outcome.message,
        "payload": outcome.payload,
        "dirty": state.session.is_dirty(),
    }))
}

#[cfg(not(target_arch = "wasm32"))]
fn execute_remote_command(port: u16, command_id: &str, params: Value) -> Result<Value, String> {
    if command_id == "ping" || is_ui_catalog_command(command_id) {
        let res = live_control::send_control(port, command_id, params)?;
        return Ok(json!({
            "command_id": command_id,
            "mutated": false,
            "live_attach": true,
            "result": res,
        }));
    }
    live_control::send_control(port, "engine.execute", json!({ "command_id": command_id, "params": params }))
}

fn is_ui_catalog_command(command_id: &str) -> bool {
    UI_COMMANDS.iter().any(|&(id, _, _, _)| id == command_id)
}

const fn is_attached(state: &McpServerState) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        state.control_port.is_some()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = state;
        false
    }
}

fn handle_render_preview(state: &McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let sheet_id = optional_non_empty_str(args, "sheet_id")?;
    let dpi = parse_dpi(args)?;
    let requested_w = optional_u32_bounded(args, "width_px", 1, MAX_RASTER_DIM_PX)?;
    let requested_h = optional_u32_bounded(args, "height_px", 1, MAX_RASTER_DIM_PX)?;

    let (target_id, sheet_kind, scene) = match compile_visual_sheet(state.session.document(), sheet_id.as_deref()) {
        Ok(compiled) => compiled,
        Err(msg) => return Ok(ToolCallResult::error_text(msg)),
    };

    let spec = build_render_spec(&scene, requested_w, requested_h, dpi);
    let raster = match render_scene(&scene, spec) {
        Ok(img) => img,
        Err(err) => return Ok(ToolCallResult::error_text(format!("rasterization failed: {err}"))),
    };
    let png_bytes = match encode_png(&raster) {
        Ok(bytes) => bytes,
        Err(err) => return Ok(ToolCallResult::error_text(format!("PNG encoding failed: {err}"))),
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
    })
}

fn handle_export(state: &McpServerState, args: &Map<String, Value>) -> Result<ToolCallResult, ToolCallError> {
    let format = required_non_empty_str(args, "format")?.to_ascii_lowercase();
    let sheet_id = optional_non_empty_str(args, "sheet_id")?;
    let output_path = optional_non_empty_str(args, "output_path")?.or(optional_non_empty_str(args, "path")?);
    let doc = state.session.document();

    match format.as_str() {
        "svg" => export_svg_tool(doc, sheet_id.as_deref(), output_path.as_deref(), args),
        "pdf" => export_pdf_tool(doc, sheet_id.as_deref(), output_path.as_deref(), args),
        "png" => export_png_tool(doc, sheet_id.as_deref(), output_path.as_deref(), args),
        "tiff" => export_tiff_tool(doc, sheet_id.as_deref(), output_path.as_deref(), args),
        "csv" => export_csv_tool(doc, sheet_id.as_deref(), output_path.as_deref(), args),
        "json" => export_json_tool(doc, sheet_id.as_deref(), output_path.as_deref(), args),
        _ => Err(ToolCallError::InvalidParams(format!("unsupported export format '{format}'"))),
    }
}

fn export_svg_tool(
    doc: &ProjectDocument,
    sheet_id: Option<&str>,
    output_path: Option<&str>,
    args: &Map<String, Value>,
) -> Result<ToolCallResult, ToolCallError> {
    let pretty = optional_bool(args, "pretty")?.unwrap_or(true);
    let precision = optional_u32_bounded(args, "svg_precision", 1, 8)?;
    let (target_id, _, scene) = match compile_visual_sheet(doc, sheet_id) {
        Ok(compiled) => compiled,
        Err(msg) => return Ok(ToolCallResult::error_text(msg)),
    };

    let mut opts = SvgExportOptions { pretty, ..SvgExportOptions::default() };
    if let Some(p) = precision {
        opts.precision = p as u8;
    }

    let svg = match render_svg(&scene, &opts) {
        Ok(s) => s,
        Err(err) => return Ok(ToolCallResult::error_text(format!("SVG export failed: {err}"))),
    };

    if let Some(path) = output_path {
        return write_text_export(path, "svg", &target_id, &svg);
    }
    Ok(ToolCallResult::ok_text(svg))
}

fn export_pdf_tool(
    doc: &ProjectDocument,
    sheet_id: Option<&str>,
    output_path: Option<&str>,
    args: &Map<String, Value>,
) -> Result<ToolCallResult, ToolCallError> {
    let all_graphs = optional_bool(args, "all_graphs")?.unwrap_or(false);
    let color_space = match optional_non_empty_str(args, "pdf_color_space")? {
        Some(raw) => parse_pdf_color_space(&raw)?,
        None => PdfColorSpace::Srgb,
    };

    let mut opts = PdfExportOptions { color_space, ..PdfExportOptions::default() };

    let (target_id, pdf_bytes) = if all_graphs && sheet_id.is_none() {
        let mut scenes = Vec::new();
        for gs in &doc.graph_sheets {
            let scene = match compile_graph(gs, doc) {
                Ok(s) => s,
                Err(err) => return Ok(ToolCallResult::error_text(format!("chart compile failed for '{}': {err}", gs.id))),
            };
            scenes.push(scene);
        }
        for ls in &doc.layout_sheets {
            let scene = match compile_layout(ls, doc) {
                Ok(s) => s,
                Err(err) => return Ok(ToolCallResult::error_text(format!("layout compile failed for '{}': {err}", ls.id))),
            };
            scenes.push(scene);
        }
        if scenes.is_empty() {
            return Ok(ToolCallResult::error_text(String::from("no GraphSheet or LayoutSheet available to export")));
        }
        opts.title = Some(doc.title.clone());
        match render_pdf_pages(&scenes, &opts) {
            Ok(bytes) => (String::from("all"), bytes),
            Err(err) => return Ok(ToolCallResult::error_text(format!("PDF export failed: {err}"))),
        }
    } else {
        let (target_id, _, scene) = match compile_visual_sheet(doc, sheet_id) {
            Ok(compiled) => compiled,
            Err(msg) => return Ok(ToolCallResult::error_text(msg)),
        };
        opts.title = Some(scene.title.clone());
        match render_pdf(&scene, &opts) {
            Ok(bytes) => (target_id, bytes),
            Err(err) => return Ok(ToolCallResult::error_text(format!("PDF export failed: {err}"))),
        }
    };

    if let Some(path) = output_path {
        return write_binary_export(path, "pdf", &target_id, &pdf_bytes);
    }

    ToolCallResult::ok_json(&json!({
        "format": "pdf",
        "sheet_id": target_id,
        "encoding": "base64",
        "bytes_len": pdf_bytes.len(),
        "data": encode_base64(&pdf_bytes),
    }))
}

fn export_png_tool(
    doc: &ProjectDocument,
    sheet_id: Option<&str>,
    output_path: Option<&str>,
    args: &Map<String, Value>,
) -> Result<ToolCallResult, ToolCallError> {
    let dpi = parse_dpi(args)?;
    let requested_w = optional_u32_bounded(args, "width_px", 1, MAX_RASTER_DIM_PX)?;
    let requested_h = optional_u32_bounded(args, "height_px", 1, MAX_RASTER_DIM_PX)?;
    let (target_id, _, scene) = match compile_visual_sheet(doc, sheet_id) {
        Ok(compiled) => compiled,
        Err(msg) => return Ok(ToolCallResult::error_text(msg)),
    };

    let spec = build_render_spec(&scene, requested_w, requested_h, dpi);
    let raster = match render_scene(&scene, spec) {
        Ok(img) => img,
        Err(err) => return Ok(ToolCallResult::error_text(format!("rasterization failed: {err}"))),
    };
    let png_bytes = match encode_png(&raster) {
        Ok(bytes) => bytes,
        Err(err) => return Ok(ToolCallResult::error_text(format!("PNG encoding failed: {err}"))),
    };

    if let Some(path) = output_path {
        return write_binary_export(path, "png", &target_id, &png_bytes);
    }

    Ok(ToolCallResult {
        content: vec![
            ContentBlock::Image { data: encode_base64(&png_bytes), mime_type: String::from("image/png") },
            ContentBlock::Text {
                text: serde_json::to_string_pretty(&json!({
                    "format": "png",
                    "sheet_id": target_id,
                    "width_px": raster.width_px,
                    "height_px": raster.height_px,
                    "dpi": raster.dpi,
                    "bytes_len": png_bytes.len(),
                }))
                .unwrap_or_default(),
            },
        ],
        is_error: false,
    })
}

fn export_tiff_tool(
    doc: &ProjectDocument,
    sheet_id: Option<&str>,
    output_path: Option<&str>,
    args: &Map<String, Value>,
) -> Result<ToolCallResult, ToolCallError> {
    let dpi = parse_dpi(args)?;
    let requested_w = optional_u32_bounded(args, "width_px", 1, MAX_RASTER_DIM_PX)?;
    let requested_h = optional_u32_bounded(args, "height_px", 1, MAX_RASTER_DIM_PX)?;
    let compression = match optional_non_empty_str(args, "tiff_compression")? {
        Some(raw) => parse_tiff_compression(&raw)?,
        None => TiffCompression::Lzw,
    };
    let (target_id, _, scene) = match compile_visual_sheet(doc, sheet_id) {
        Ok(compiled) => compiled,
        Err(msg) => return Ok(ToolCallResult::error_text(msg)),
    };

    let spec = build_render_spec(&scene, requested_w, requested_h, dpi);
    let raster = match render_scene(&scene, spec) {
        Ok(img) => img,
        Err(err) => return Ok(ToolCallResult::error_text(format!("rasterization failed: {err}"))),
    };
    let tiff_bytes = match encode_tiff(&raster, compression) {
        Ok(bytes) => bytes,
        Err(err) => return Ok(ToolCallResult::error_text(format!("TIFF encoding failed: {err}"))),
    };

    if let Some(path) = output_path {
        return write_binary_export(path, "tiff", &target_id, &tiff_bytes);
    }

    ToolCallResult::ok_json(&json!({
        "format": "tiff",
        "sheet_id": target_id,
        "encoding": "base64",
        "width_px": raster.width_px,
        "height_px": raster.height_px,
        "dpi": raster.dpi,
        "bytes_len": tiff_bytes.len(),
        "data": encode_base64(&tiff_bytes),
    }))
}

fn export_csv_tool(
    doc: &ProjectDocument,
    sheet_id: Option<&str>,
    output_path: Option<&str>,
    args: &Map<String, Value>,
) -> Result<ToolCallResult, ToolCallError> {
    let tidy = optional_bool(args, "tidy")?.unwrap_or(true);
    let tab_id = optional_non_empty_str(args, "tab_id")?;
    let mut opts = CsvOptions::csv();
    if let Some(delim) = optional_non_empty_str(args, "csv_delimiter")? {
        let mut chars = delim.chars();
        let Some(ch) = chars.next() else {
            return Err(ToolCallError::InvalidParams(String::from("csv_delimiter must not be empty")));
        };
        if chars.next().is_some() {
            return Err(ToolCallError::InvalidParams(String::from("csv_delimiter must be a single character")));
        }
        opts.delimiter = ch;
    }

    let (target_id, csv_text) = match resolve_tabular_export_target(doc, sheet_id) {
        Ok(TabularTarget::Data(ds)) => match export_datasheet_to_csv(ds, &opts) {
            Ok(csv) => (ds.id.as_str().to_owned(), csv),
            Err(err) => return Ok(ToolCallResult::error_text(format!("CSV export failed: {err}"))),
        },
        Ok(TabularTarget::Result(rs)) => {
            let csv = if tidy {
                export_result_sheet_tidy_csv(rs, &opts)
            } else if let Some(tid) = tab_id {
                let Some(tab) = rs.tabs.iter().find(|t| t.id == tid) else {
                    return Ok(ToolCallResult::error_text(format!("tab '{tid}' not found on result sheet '{}'", rs.id)));
                };
                export_result_tab_csv(tab, &opts)
            } else if let Some(first_tab) = rs.tabs.first() {
                export_result_tab_csv(first_tab, &opts)
            } else {
                export_result_sheet_tidy_csv(rs, &opts)
            };
            (rs.id.as_str().to_owned(), csv)
        }
        Err(msg) => return Ok(ToolCallResult::error_text(msg)),
    };

    if let Some(path) = output_path {
        return write_text_export(path, "csv", &target_id, &csv_text);
    }
    Ok(ToolCallResult::ok_text(csv_text))
}

fn export_json_tool(
    doc: &ProjectDocument,
    sheet_id: Option<&str>,
    output_path: Option<&str>,
    args: &Map<String, Value>,
) -> Result<ToolCallResult, ToolCallError> {
    let pretty = optional_bool(args, "pretty")?.unwrap_or(true);
    let (target_id, json_text) = if let Some(id_str) = sheet_id {
        let id = SheetId::new(id_str);
        if let Some(rs) = doc.result_sheet(&id) {
            let text = match export_result_sheet_json(rs, pretty) {
                Ok(t) => t,
                Err(err) => return Ok(ToolCallResult::error_text(format!("JSON export failed: {err}"))),
            };
            (id_str.to_owned(), text)
        } else if let Some(ds) = doc.data_sheet(&id) {
            (id_str.to_owned(), serialize_json_value(ds, pretty)?)
        } else if let Some(info) = doc.info_sheet(&id) {
            (id_str.to_owned(), serialize_json_value(info, pretty)?)
        } else if let Some(gs) = doc.graph_sheet(&id) {
            (id_str.to_owned(), serialize_json_value(gs, pretty)?)
        } else if let Some(ls) = doc.layout_sheet(&id) {
            (id_str.to_owned(), serialize_json_value(ls, pretty)?)
        } else {
            return Ok(ToolCallResult::error_text(format!("sheet '{id_str}' not found")));
        }
    } else {
        (String::from("project"), serialize_json_value(doc, pretty)?)
    };

    if let Some(path) = output_path {
        return write_text_export(path, "json", &target_id, &json_text);
    }
    Ok(ToolCallResult::ok_text(json_text))
}

fn serialize_json_value(val: &impl Serialize, pretty: bool) -> Result<String, ToolCallError> {
    if pretty { serde_json::to_string_pretty(val) } else { serde_json::to_string(val) }
        .map_err(|err| ToolCallError::Execution(format!("JSON serialization error: {err}")))
}

fn write_text_export(path: &str, format: &str, target_id: &str, text: &str) -> Result<ToolCallResult, ToolCallError> {
    write_binary_export(path, format, target_id, text.as_bytes())
}

fn write_binary_export(path: &str, format: &str, target_id: &str, bytes: &[u8]) -> Result<ToolCallResult, ToolCallError> {
    let dest = Path::new(path);
    if let Err(err) = fs::write(dest, bytes) {
        return Ok(ToolCallResult::error_text(format!("failed to write '{}': {err}", dest.display())));
    }
    ToolCallResult::ok_json(&json!({
        "exported": true,
        "format": format,
        "sheet_id": target_id,
        "path": dest.display().to_string(),
        "bytes_written": bytes.len(),
    }))
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
        scaled.clamp(64.0, f64::from(MAX_RASTER_DIM_PX)) as u32
    } else {
        DEFAULT_PREVIEW_WIDTH_PX
    };
    let default_h = if scene.height_pt.is_finite() && scene.height_pt > 0.0 {
        let scaled = (scene.height_pt * f64::from(dpi) / 72.0).round();
        scaled.clamp(64.0, f64::from(MAX_RASTER_DIM_PX)) as u32
    } else {
        DEFAULT_PREVIEW_HEIGHT_PX
    };

    RenderTargetSpec { width_px: requested_w.unwrap_or(default_w), height_px: requested_h.unwrap_or(default_h), dpi }
}

fn parse_dpi(args: &Map<String, Value>) -> Result<u16, ToolCallError> {
    match optional_u32_bounded(args, "dpi", u32::from(MIN_DPI), u32::from(MAX_DPI))? {
        Some(v) => Ok(v as u16),
        None => Ok(DEFAULT_PREVIEW_DPI),
    }
}

fn resolve_open_format(path: &Path, args: &Map<String, Value>) -> Result<String, ToolCallError> {
    if let Some(fmt_str) = optional_non_empty_str(args, "format")? {
        return Ok(fmt_str.to_ascii_lowercase());
    }
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    match ext.as_str() {
        "lumencraft" | "lctemplate" | "prism" | "pzfx" | "csv" | "tsv" => Ok(ext),
        "txt" => Ok(String::from("csv")),
        "" => Err(ToolCallError::InvalidParams(String::from("cannot infer file format without extension; specify \"format\""))),
        other => Err(ToolCallError::InvalidParams(format!("unsupported file extension '.{other}'"))),
    }
}

fn resolve_save_format(path: &Path, args: &Map<String, Value>) -> Result<String, ToolCallError> {
    if let Some(fmt_str) = optional_non_empty_str(args, "format")? {
        return Ok(fmt_str.to_ascii_lowercase());
    }
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    match ext.as_str() {
        "lctemplate" => Ok(String::from("lctemplate")),
        _ => Ok(String::from("lumencraft")),
    }
}

fn parse_table_kind(raw: &str) -> Result<TableKind, ToolCallError> {
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
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

fn extract_command_params(args: &Map<String, Value>) -> Result<Value, ToolCallError> {
    match args.get("params") {
        None | Some(Value::Null) => Ok(Value::Object(Map::new())),
        Some(v) if v.is_object() => Ok(v.clone()),
        Some(_) => Err(ToolCallError::InvalidParams(String::from("\"params\" must be a JSON object"))),
    }
}

fn required_non_empty_str(args: &Map<String, Value>, key: &str) -> Result<String, ToolCallError> {
    optional_non_empty_str(args, key)?.ok_or_else(|| ToolCallError::InvalidParams(format!("missing or empty string parameter \"{key}\"")))
}

fn optional_non_empty_str(args: &Map<String, Value>, key: &str) -> Result<Option<String>, ToolCallError> {
    let Some(val) = args.get(key) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let Some(s) = val.as_str() else {
        return Err(ToolCallError::InvalidParams(format!("parameter \"{key}\" must be a string")));
    };
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(ToolCallError::InvalidParams(format!("parameter \"{key}\" must not be blank")));
    }
    Ok(Some(trimmed.to_owned()))
}

fn optional_bool(args: &Map<String, Value>, key: &str) -> Result<Option<bool>, ToolCallError> {
    let Some(val) = args.get(key) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    val.as_bool().map(Some).ok_or_else(|| ToolCallError::InvalidParams(format!("parameter \"{key}\" must be a boolean")))
}

fn optional_u32_bounded(args: &Map<String, Value>, key: &str, min: u32, max: u32) -> Result<Option<u32>, ToolCallError> {
    let Some(val) = args.get(key) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let Some(num) = val.as_u64() else {
        return Err(ToolCallError::InvalidParams(format!("parameter \"{key}\" must be a positive integer")));
    };
    let parsed = u32::try_from(num).map_err(|_| ToolCallError::InvalidParams(format!("parameter \"{key}\" out of range [{min}, {max}]")))?;
    if parsed < min || parsed > max {
        return Err(ToolCallError::InvalidParams(format!("parameter \"{key}\" must be in [{min}, {max}]")));
    }
    Ok(Some(parsed))
}

/// Encode raw binary data using standard RFC 4648 base64 (`=` padded).
#[must_use]
pub(crate) fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let triple = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);

        out.push(char::from(TABLE[((triple >> 18) & 0x3F) as usize]));
        out.push(char::from(TABLE[((triple >> 12) & 0x3F) as usize]));
        if chunk.len() > 1 {
            out.push(char::from(TABLE[((triple >> 6) & 0x3F) as usize]));
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(char::from(TABLE[(triple & 0x3F) as usize]));
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
mod live_control {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    use serde::Deserialize;
    use serde_json::{Value, json};

    const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
    const IO_TIMEOUT: Duration = Duration::from_secs(5);
    pub(crate) const MAX_CONTROL_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

    #[derive(Debug, Deserialize)]
    struct ControlResponse {
        id: String,
        ok: bool,
        #[serde(default)]
        result: Value,
        error: Option<String>,
    }

    pub fn send_control(port: u16, method: &str, params: Value) -> Result<Value, String> {
        if port == 0 {
            return Err(String::from("control port must be in 1..=65535"));
        }
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
            .map_err(|err| format!("failed to connect to desktop control port 127.0.0.1:{port}: {err}"))?;
        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));

        let mut writer = stream.try_clone().map_err(|err| format!("control stream clone failed: {err}"))?;
        send_control_on_io(&mut stream, &mut writer, method, params, MAX_CONTROL_RESPONSE_BYTES)
    }

    pub(crate) fn send_control_on_io<R: Read, W: Write>(
        reader_io: &mut R,
        writer_io: &mut W,
        method: &str,
        params: Value,
        max_response_bytes: usize,
    ) -> Result<Value, String> {
        let req_id = String::from("mcp-1");
        let payload = serde_json::to_vec(&json!({
            "id": req_id,
            "method": method,
            "params": params,
        }))
        .map_err(|err| format!("failed to encode control request: {err}"))?;

        writer_io.write_all(&payload).map_err(|err| format!("control write failed: {err}"))?;
        writer_io.write_all(b"\n").map_err(|err| format!("control write failed: {err}"))?;
        writer_io.flush().map_err(|err| format!("control flush failed: {err}"))?;

        let mut reader = BufReader::new(reader_io);
        let line_bytes = read_bounded_line(&mut reader, max_response_bytes)?;
        let resp: ControlResponse = serde_json::from_slice(&line_bytes).map_err(|err| format!("invalid control response JSON: {err}"))?;
        if resp.id != req_id {
            return Err(format!("mismatched control response id '{}'", resp.id));
        }
        if !resp.ok {
            return Err(resp.error.unwrap_or_else(|| String::from("desktop control command failed")));
        }
        Ok(resp.result)
    }

    fn read_bounded_line<R: BufRead>(reader: &mut R, max_bytes: usize) -> Result<Vec<u8>, String> {
        let mut buf = Vec::new();
        loop {
            let avail = reader.fill_buf().map_err(|err| format!("control read failed: {err}"))?;
            if avail.is_empty() {
                if buf.is_empty() {
                    return Err(String::from("desktop control connection closed without response"));
                }
                return Ok(buf);
            }
            if let Some(pos) = avail.iter().position(|&b| b == b'\n') {
                let take = pos + 1;
                if buf.len().saturating_add(take) > max_bytes {
                    return Err(format!("control response exceeds {max_bytes} bytes"));
                }
                buf.extend_from_slice(&avail[..take]);
                reader.consume(take);
                return Ok(buf);
            }
            if buf.len().saturating_add(avail.len()) > max_bytes {
                return Err(format!("control response exceeds {max_bytes} bytes"));
            }
            let take = avail.len();
            buf.extend_from_slice(avail);
            reader.consume(take);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{McpServerState, ToolCallError, call_tool, encode_base64, list_tools};
    use crate::ui_catalog::UI_COMMANDS;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
        assert_eq!(encode_base64(b"foob"), "Zm9vYg==");
        assert_eq!(encode_base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode_base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn list_tools_exposes_all_eight_tools() {
        let names: Vec<String> = list_tools().into_iter().map(|t| t.name).collect();
        assert_eq!(names, vec!["doc_inspect", "open_file", "save_file", "command_list", "command_run", "command_batch", "render_preview", "export"]);
    }

    #[test]
    fn command_list_merges_engine_and_ui_commands_with_ui_flag() {
        let mut state = McpServerState::new();
        let res = call_tool(&mut state, "command_list", json!({})).expect("command_list");
        assert!(!res.is_error);
        let super::ContentBlock::Text { text } = &res.content[0] else {
            panic!("expected text block");
        };
        let parsed: Value = serde_json::from_str(text).expect("json");
        let list = parsed["commands"].as_array().expect("commands array");

        for &(ui_id, _, _, _) in UI_COMMANDS {
            let entry = list.iter().find(|c| c["id"] == ui_id).expect("ui command entry");
            assert_eq!(entry["ui"], json!(true));
        }
        let non_ui = list.iter().find(|c| c["id"] == "sheet.createData").expect("engine command");
        assert_eq!(non_ui["ui"], json!(false));
    }

    #[test]
    fn unknown_tool_and_bad_params_return_protocol_errors() {
        let mut state = McpServerState::new();
        assert!(matches!(call_tool(&mut state, "nonexistent", json!({})), Err(ToolCallError::UnknownTool(_))));
        assert!(matches!(call_tool(&mut state, "command_run", json!({})), Err(ToolCallError::InvalidParams(_))));
        assert!(matches!(call_tool(&mut state, "export", json!({"format": "bmp"})), Err(ToolCallError::InvalidParams(_))));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn live_control_wire_protocol_validates_id_ok_and_size_caps() {
        use std::io::Cursor;

        let mut outbound = Vec::new();
        let mut inbound = Cursor::new(br#"{"id":"mcp-1","ok":true,"result":{"navigator_open":true}}"#.as_slice());
        let ok = super::live_control::send_control_on_io(&mut inbound, &mut outbound, "ui.navigator.toggle", json!({}), 1024).expect("control ok");
        assert_eq!(ok["navigator_open"], json!(true));
        let sent: Value = serde_json::from_slice(&outbound).expect("outbound jsonl");
        assert_eq!(sent, json!({"id": "mcp-1", "method": "ui.navigator.toggle", "params": {}}));

        let mut outbound = Vec::new();
        let mut bad_id = Cursor::new(br#"{"id":"wrong-id","ok":true,"result":{}}"#.as_slice());
        assert!(super::live_control::send_control_on_io(&mut bad_id, &mut outbound, "ping", json!({}), 1024).is_err());

        let mut outbound = Vec::new();
        let mut err_resp = Cursor::new(br#"{"id":"mcp-1","ok":false,"error":"denied"}"#.as_slice());
        let err = super::live_control::send_control_on_io(&mut err_resp, &mut outbound, "ping", json!({}), 1024).expect_err("control error");
        assert!(err.contains("denied"));

        let mut outbound = Vec::new();
        let huge = vec![b'a'; 256];
        let mut huge_cur = Cursor::new(huge.as_slice());
        assert!(super::live_control::send_control_on_io(&mut huge_cur, &mut outbound, "ping", json!({}), 64).is_err());
    }
}
