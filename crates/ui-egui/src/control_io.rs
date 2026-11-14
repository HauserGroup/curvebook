//! Local JSON-lines control channel and publication clipboard/print payload builders
//! (`docs/control-protocol.md`, `docs/hardware-parity.md`).

use std::io::{self, BufRead, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::net::{SocketAddr, TcpListener};
#[cfg(not(target_arch = "wasm32"))]
use std::process::{Command, Output, Stdio};

use lumencraft_chart::{ChartScene, Path2D, PathCmd, PointPt, RectPt, ScenePrimitive, StrokePaint};
use lumencraft_doc::{DataSheet, ProjectDocument, SheetCellRange, SheetFolderKind, SheetId, SheetSection};
use lumencraft_engine::Session;
use lumencraft_engine::catalog::COMMANDS;
use lumencraft_format::{CsvOptions, export_datasheet_to_csv, format_cell_token, write_delimited_records};
use lumencraft_palette::Rgba8;
use lumencraft_pdf::{PdfColorSpace, PdfExportOptions, render_pdf, render_pdf_pages};
use lumencraft_render::{RenderTargetSpec, encode_png, render_scene};
use lumencraft_svg::{SvgExportOptions, render_svg};
use lumencraft_tools::PhysicalUnit;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, render_workspace_frame_png};
use crate::i18n::{LocalePreset, is_rtl_locale, locale_display_name, normalize_locale};
use crate::menus::ui_commands::UI_COMMANDS;
use crate::theme::{UiThemeMode, UiThemeSpec};
use crate::workspace_nav::{ActiveWorkspacePane, WelcomeMode};

/// Hard bound per JSON-lines control request/response frame (16 MiB).
pub const MAX_CONTROL_LINE_BYTES: usize = 16 * 1024 * 1024;

const DEFAULT_SCREENSHOT_WIDTH_PX: u32 = 1024;
const DEFAULT_SCREENSHOT_HEIGHT_PX: u32 = 640;
const DEFAULT_SCREENSHOT_DPI: u16 = 144;
const MIN_SCREENSHOT_DIM_PX: u32 = 64;
const MAX_SCREENSHOT_DIM_PX: u32 = 4096;
const MIN_DPI: u16 = 72;
const MAX_DPI: u16 = 1200;
const CROP_MARGIN_PT: f64 = 18.0;

/// Control-loop continuation directive returned alongside a serialized response line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlLoopAction {
    Continue,
    Quit,
}

/// JSON-lines control request message received from automation or MCP clients.
///
/// Accepts both `{"id": "1", "method": "...", "params": {...}}` and
/// `{"id": 1, "op": "...", ...}` wire encodings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlRequest {
    #[serde(default)]
    pub id: Value,
    #[serde(alias = "op", default)]
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// JSON-lines control response emitted by [`handle_control_line`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlResponse {
    pub id: Value,
    pub ok: bool,
    pub result: Value,
    pub error: Option<String>,
}

impl ControlResponse {
    fn ok(id: Value, result: Value) -> Self {
        Self { id, ok: true, result, error: None }
    }

    fn err(id: Value, message: impl Into<String>) -> Self {
        Self { id, ok: false, result: Value::Null, error: Some(message.into()) }
    }
}

/// Multi-flavor publication clipboard payload (`PDF`, `SVG`, `HTML`, high-DPI `PNG`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipboardBundle {
    pub pdf_bytes: Vec<u8>,
    pub svg: String,
    pub html: String,
    pub png_bytes: Vec<u8>,
    pub width_pt: f64,
    pub height_pt: f64,
    pub width_px: u32,
    pub height_px: u32,
    pub dpi: u16,
}

impl ClipboardBundle {
    /// Build a multi-flavor clipboard payload (`PDF`, `SVG`, `HTML`, `PNG`) from a compiled [`ChartScene`].
    ///
    /// # Errors
    /// Returns an error if `dpi` is outside `72..=1200`, scene bounds are invalid, or any vector/raster encoder fails.
    pub fn from_scene(scene: &ChartScene, dpi: u16) -> Result<Self, String> {
        if !(MIN_DPI..=MAX_DPI).contains(&dpi) {
            return Err(format!("clipboard DPI {dpi} out of supported range {MIN_DPI}..={MAX_DPI}"));
        }
        if !scene.width_pt.is_finite() || !scene.height_pt.is_finite() || scene.width_pt <= 0.0 || scene.height_pt <= 0.0 {
            return Err(String::from("scene dimensions must be finite and positive"));
        }

        let scale = f64::from(dpi) / 72.0;
        let width_px = (scene.width_pt * scale).round().clamp(1.0, 8192.0) as u32;
        let height_px = (scene.height_pt * scale).round().clamp(1.0, 8192.0) as u32;

        let pdf_opts = PdfExportOptions { title: Some(scene.title.clone()), ..PdfExportOptions::default() };
        let pdf_bytes = render_pdf(scene, &pdf_opts).map_err(|err| format!("clipboard PDF encode failed: {err}"))?;
        let svg = render_svg(scene, &SvgExportOptions::default()).map_err(|err| format!("clipboard SVG encode failed: {err}"))?;
        let html = build_html_clipboard_wrapper(&scene.title, &svg, scene.width_pt, scene.height_pt);
        let raster =
            render_scene(scene, RenderTargetSpec { width_px, height_px, dpi }).map_err(|err| format!("clipboard raster render failed: {err}"))?;
        let png_bytes = encode_png(&raster).map_err(|err| format!("clipboard PNG encode failed: {err}"))?;

        Ok(Self { pdf_bytes, svg, html, png_bytes, width_pt: scene.width_pt, height_pt: scene.height_pt, width_px, height_px, dpi })
    }

    /// Compile `sheet_id` from `session` and build its multi-flavor [`ClipboardBundle`].
    ///
    /// # Errors
    /// Returns an error if compiling `sheet_id` or rendering any clipboard flavor fails.
    pub fn from_sheet(session: &Session, sheet_id: &SheetId, dpi: u16) -> Result<Self, String> {
        let scene = session.render_sheet_scene(sheet_id).map_err(|err| err.to_string())?;
        Self::from_scene(&scene, dpi)
    }

    /// Format an entire [`DataSheet`] as a tab-delimited TSV payload preserving `*` exclusions.
    ///
    /// # Errors
    /// Returns an error if TSV formatting fails.
    pub fn tsv_from_datasheet(sheet: &DataSheet, decimal_separator: char) -> Result<String, String> {
        let opts = CsvOptions { decimal_separator, ..CsvOptions::tsv() };
        export_datasheet_to_csv(sheet, &opts).map_err(|err| err.to_string())
    }

    /// Format a rectangular [`SheetCellRange`] of `sheet` as TSV text preserving `*` exclusions and locale decimals.
    #[must_use]
    pub fn tsv_from_datasheet_range(sheet: &DataSheet, range: SheetCellRange, decimal_separator: char) -> String {
        let grid = sheet.extract_cell_grid(range);
        if grid.is_empty() {
            return String::new();
        }
        let opts = CsvOptions { decimal_separator, ..CsvOptions::tsv() };
        let rows: Vec<Vec<String>> = grid.iter().map(|row| row.iter().map(|cell| format_cell_token(cell, &opts)).collect()).collect();
        write_delimited_records(&rows, &opts)
    }
}

/// Physical-size print spool specification (`docs/hardware-parity.md` `G10`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintJobSpec {
    pub sheet_ids: Vec<SheetId>,
    pub scale_percent: f64,
    pub crop_marks: bool,
    pub bleed_pt: f64,
    pub color_space: PdfColorSpace,
    pub title: Option<String>,
}

impl Default for PrintJobSpec {
    fn default() -> Self {
        Self { sheet_ids: Vec::new(), scale_percent: 100.0, crop_marks: false, bleed_pt: 0.0, color_space: PdfColorSpace::Srgb, title: None }
    }
}

/// Generated print spool artifact returned by [`PrintJobSpec::spool_pdf`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintSpoolOutput {
    pub pdf_bytes: Vec<u8>,
    pub page_count: usize,
    pub page_sizes_pt: Vec<(f64, f64)>,
    pub scale_percent: f64,
}

impl PrintJobSpec {
    /// Exact `100%` physical-size publication print preset.
    #[must_use]
    pub fn publication_100_percent() -> Self {
        Self::default()
    }

    /// Enable corner crop marks and symmetric bleed border (in PostScript points) around each printed page.
    #[must_use]
    pub fn with_crop_and_bleed(mut self, bleed_pt: f64) -> Self {
        self.crop_marks = true;
        self.bleed_pt = bleed_pt.max(0.0);
        self
    }

    /// Generate a physical-scale multi-page PDF spool payload from `session`.
    ///
    /// # Errors
    /// Returns an error if `scale_percent`/`bleed_pt` are invalid, no printable graph/layout sheets exist,
    /// or PDF encoding fails.
    pub fn spool_pdf(&self, session: &Session) -> Result<PrintSpoolOutput, String> {
        if !self.scale_percent.is_finite() || !(1.0..=1000.0).contains(&self.scale_percent) {
            return Err(format!("print scale_percent {} out of valid range 1.0..=1000.0", self.scale_percent));
        }
        if !self.bleed_pt.is_finite() || self.bleed_pt < 0.0 || self.bleed_pt > 144.0 {
            return Err(format!("print bleed_pt {} out of valid range 0.0..=144.0", self.bleed_pt));
        }

        let target_ids = resolve_print_sheet_ids(&self.sheet_ids, session.document())?;
        let scale_factor = self.scale_percent / 100.0;
        let outer_margin = if self.crop_marks { CROP_MARGIN_PT.max(self.bleed_pt + 4.0) } else { self.bleed_pt };

        let mut pages = Vec::with_capacity(target_ids.len());
        let mut trim_sizes = Vec::with_capacity(target_ids.len());
        for sheet_id in &target_ids {
            let compiled = session.render_sheet_scene(sheet_id).map_err(|err| err.to_string())?;
            let scaled = scale_scene_for_print(&compiled, scale_factor);
            trim_sizes.push((scaled.width_pt, scaled.height_pt));
            let decorated =
                if self.crop_marks || self.bleed_pt > 0.0 { decorate_scene_for_print(&scaled, outer_margin, self.crop_marks) } else { scaled };
            pages.push(decorated);
        }

        let page_sizes_pt: Vec<(f64, f64)> = pages.iter().map(|p| (p.width_pt, p.height_pt)).collect();
        let (trim_pt, bleed_pt) = compute_uniform_pdf_boxes(&trim_sizes, outer_margin, self.bleed_pt, self.crop_marks);
        let title = self.title.clone().or_else(|| Some(session.document().title.clone()));
        let pdf_opts = PdfExportOptions {
            color_space: self.color_space,
            compress_streams: true,
            title,
            creator: String::from("LumenCraft Print Spooler"),
            outline_text: false,
            trim_pt,
            bleed_pt,
            ..PdfExportOptions::default()
        };
        let pdf_bytes = render_pdf_pages(&pages, &pdf_opts).map_err(|err| format!("print PDF spool failed: {err}"))?;

        Ok(PrintSpoolOutput { pdf_bytes, page_count: pages.len(), page_sizes_pt, scale_percent: self.scale_percent })
    }
}

/// Parse a CLI or environment TCP control port (`1..=65535`).
///
/// # Errors
/// Returns a descriptive error string when `raw` is not an integer in `1..=65535`.
pub fn parse_control_port(raw: &str) -> Result<u16, String> {
    let trimmed = raw.trim();
    let port: u16 = trimmed.parse().map_err(|_| format!("invalid TCP port `{raw}`: expected 1..=65535"))?;
    if port == 0 {
        return Err(String::from("control port must be in 1..=65535 (port 0 is not valid for --control)"));
    }
    Ok(port)
}

/// Validate that `addr` targets a loopback interface (`127.0.0.1` / `::1`) only.
///
/// # Errors
/// Returns an error if `addr.ip()` is non-loopback (such as `0.0.0.0` or a LAN IP).
#[cfg(not(target_arch = "wasm32"))]
pub fn validate_loopback_addr(addr: SocketAddr) -> Result<SocketAddr, String> {
    if !addr.ip().is_loopback() {
        return Err(format!("refusing non-loopback control bind address {addr}; only loopback addresses are permitted"));
    }
    Ok(addr)
}

/// Bind a loopback-only TCP listener for `--control` JSON-lines automation.
///
/// # Errors
/// Returns `InvalidInput` if `addr` is non-loopback, or any underlying socket bind error.
#[cfg(not(target_arch = "wasm32"))]
pub fn bind_control_tcp_listener(addr: SocketAddr) -> io::Result<TcpListener> {
    if !addr.ip().is_loopback() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("refusing non-loopback control bind address {addr}; only loopback addresses are permitted"),
        ));
    }
    TcpListener::bind(addr)
}

/// Accept and process JSON-lines control connections from `listener` until `app.quit` or accept error.
///
/// A failure on one connection (invalid UTF-8, an oversized line, a client that disconnects
/// mid-response) closes only that connection. It must not end the server: returning the error
/// here would make the app exit and lose the researcher's unsaved session.
///
/// # Errors
/// Propagates listener accept errors.
#[cfg(not(target_arch = "wasm32"))]
pub fn serve_control_listener(app: &mut UiApp, listener: &TcpListener) -> io::Result<()> {
    for incoming in listener.incoming() {
        let stream = incoming?;
        match serve_control_connection(app, stream) {
            Ok(true) => break,
            Ok(false) => {}
            Err(err) => eprintln!("control connection closed after error: {err}"),
        }
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn serve_control_connection(app: &mut UiApp, mut stream: std::net::TcpStream) -> io::Result<bool> {
    let reader_stream = stream.try_clone()?;
    let mut reader = io::BufReader::new(reader_stream);
    serve_control_stream_with_quit(app, &mut reader, &mut stream)
}

/// Bind `addr` (loopback-only) and run the JSON-lines TCP control server until `app.quit`.
///
/// # Errors
/// Returns an error if `addr` is non-loopback or socket bind/accept fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_control_tcp_server(app: &mut UiApp, addr: SocketAddr) -> Result<(), String> {
    let checked = validate_loopback_addr(addr)?;
    let listener = bind_control_tcp_listener(checked).map_err(|err| format!("failed to bind control socket {checked}: {err}"))?;
    serve_control_listener(app, &listener).map_err(|err| format!("control server I/O error: {err}"))
}

/// Serve JSON-lines control requests from `reader` and emit serialized responses to `writer`,
/// stopping cleanly on EOF or `app.quit`.
///
/// # Errors
/// Returns an I/O error if reading, exceeding [`MAX_CONTROL_LINE_BYTES`], or writing fails.
pub fn serve_control_stream<R: BufRead, W: Write>(app: &mut UiApp, reader: &mut R, writer: &mut W) -> io::Result<()> {
    let _ = serve_control_stream_with_quit(app, reader, writer)?;
    Ok(())
}

/// Serve JSON-lines requests over `reader`/`writer` and return `Ok(true)` when `app.quit` stopped the stream.
///
/// # Errors
/// Returns an I/O error on underlying stream failure or oversized lines exceeding [`MAX_CONTROL_LINE_BYTES`].
pub fn serve_control_stream_with_quit<R: BufRead, W: Write>(app: &mut UiApp, reader: &mut R, writer: &mut W) -> io::Result<bool> {
    loop {
        let Some(raw_bytes) = read_bounded_control_line(reader, MAX_CONTROL_LINE_BYTES)? else {
            return Ok(false);
        };
        let line = std::str::from_utf8(&raw_bytes)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, format!("control line is not valid UTF-8: {err}")))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (response_json, action) = handle_control_line_with_action(app, trimmed);
        writer.write_all(response_json.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        if action == ControlLoopAction::Quit {
            return Ok(true);
        }
    }
}

/// Handle a single JSON-lines request against `app` and return serialized JSON response text.
#[must_use]
pub fn handle_control_line(app: &mut UiApp, line: &str) -> String {
    handle_control_line_with_action(app, line).0
}

/// Handle a single JSON-lines request against `app` and return `(response_json, loop_action)`.
#[must_use]
pub fn handle_control_line_with_action(app: &mut UiApp, line: &str) -> (String, ControlLoopAction) {
    let req = match parse_control_request(line) {
        Ok(parsed) => parsed,
        Err((id, message)) => {
            let resp = ControlResponse::err(id, message);
            return (serialize_response(&resp), ControlLoopAction::Continue);
        }
    };

    let (response, action) = dispatch_request(app, req);
    (serialize_response(&response), action)
}

/// Dispatch one front-end UI command from [`UI_COMMANDS`].
///
/// Returns `Ok(None)` when `command_id` is not a front-end UI command so callers can delegate
/// to engine command execution cleanly.
///
/// # Errors
/// Returns a descriptive error string when a recognized UI command fails validation or precondition checks.
pub fn dispatch_ui_command(app: &mut UiApp, command_id: &str, params: &Value) -> Result<Option<Value>, String> {
    match command_id {
        "ui.navigator.toggle" => {
            app.nav.navigator_open = !app.nav.navigator_open;
            Ok(Some(json!({ "navigator_open": app.nav.navigator_open })))
        }
        "ui.inspector.toggle" => {
            app.nav.inspector_open = !app.nav.inspector_open;
            Ok(Some(json!({ "inspector_open": app.nav.inspector_open })))
        }
        "ui.family.toggle" => {
            app.family_open = !app.family_open;
            Ok(Some(json!({ "family_open": app.family_open })))
        }
        "ui.palette.open" => {
            app.canvas_dialogs.open_command_palette();
            Ok(Some(json!({
                "palette_open": matches!(app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette),
                "dialog": "CommandPalette",
            })))
        }
        "ui.analyze.open" => dispatch_ui_analyze_open(app, params).map(Some),
        "ui.changeGraphType.open" => dispatch_ui_change_graph_type_open(app, params).map(Some),
        "ui.clonePipeline.open" => dispatch_ui_clone_pipeline_open(app, params).map(Some),
        "ui.zoom.fitSheet" => {
            app.canvas_dialogs.reset_zoom_to_fit();
            Ok(Some(json!({ "zoom": app.canvas_dialogs.canvas.zoom })))
        }
        "ui.gallery.toggle" => dispatch_ui_gallery_toggle(app, params).map(Some),
        "ui.pane.cycle" => dispatch_ui_pane_cycle(app, params).map(Some),
        "ui.rulers.toggle" => dispatch_ui_rulers_toggle(app, params).map(Some),
        "ui.shortcut.set" => dispatch_ui_shortcut_set(app, params).map(Some),
        "ui.shortcut.reset" => dispatch_ui_shortcut_reset(app, params).map(Some),
        "ui.locale.set" => dispatch_ui_locale_set(app, params).map(Some),
        "ui.theme.set" => dispatch_ui_theme_set(app, params).map(Some),
        "ui.screenshot" => dispatch_ui_screenshot(app, params).map(Some),
        "app.about" => {
            app.canvas_dialogs.active_dialog = ActiveDialog::About;
            Ok(Some(json!({
                "dialog": "About",
                "version": env!("CARGO_PKG_VERSION"),
            })))
        }
        _ => Ok(None),
    }
}

fn dispatch_request(app: &mut UiApp, req: ControlRequest) -> (ControlResponse, ControlLoopAction) {
    let ControlRequest { id, method, params } = req;
    if method == "app.quit" {
        return (ControlResponse::ok(id, json!({ "quit": true })), ControlLoopAction::Quit);
    }

    let result = match method.as_str() {
        "ping" => Ok(json!({
            "pong": true,
            "bundled_fonts": crate::bundled_fonts_enabled(),
            "version": env!("CARGO_PKG_VERSION"),
        })),
        "command.list" => Ok(handle_command_list(app)),
        "command.run" => handle_command_run(app, &params),
        "command.batch" => handle_command_batch(app, &params),
        "doc.inspect" => handle_doc_inspect(app.session().document(), app.session().is_dirty(), &params),
        "render.svg" => handle_render_svg(app, &params),
        "render.pdf" => handle_render_pdf(app, &params),
        "ui.state" | "state.snapshot" => Ok(handle_ui_state(app)),
        "engine.execute" => handle_engine_execute(app, &params),
        _ => dispatch_command_by_id(app, &method, params),
    };

    let response = match result {
        Ok(value) => ControlResponse::ok(id, value),
        Err(message) => ControlResponse::err(id, message),
    };
    (response, ControlLoopAction::Continue)
}

fn dispatch_command_by_id(app: &mut UiApp, command_id: &str, params: Value) -> Result<Value, String> {
    if let Some(ui_value) = dispatch_ui_command(app, command_id, &params)? {
        return Ok(ui_value);
    }
    let outcome = app.execute_command(command_id, params).map_err(|err| err.to_string())?;
    serde_json::to_value(outcome).map_err(|err| format!("failed to serialize command outcome: {err}"))
}

fn parse_control_request(line: &str) -> Result<ControlRequest, (Value, String)> {
    let raw: Value = serde_json::from_str(line).map_err(|err| (Value::String(String::new()), format!("invalid control JSON: {err}")))?;
    let Some(obj) = raw.as_object() else {
        return Err((Value::String(String::new()), String::from("control request must be a JSON object")));
    };
    let id = obj.get("id").cloned().unwrap_or_else(|| Value::String(String::new()));
    let Some(method) = obj.get("method").or_else(|| obj.get("op")).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) else {
        return Err((id, String::from("missing non-empty 'method' or 'op' field")));
    };

    let params = normalize_request_params(method, obj);
    Ok(ControlRequest { id, method: method.to_owned(), params })
}

fn normalize_request_params(method: &str, obj: &Map<String, Value>) -> Value {
    if method == "command.run" && (obj.contains_key("command") || obj.contains_key("command_id")) {
        let mut out = Map::new();
        if let Some(cmd) = obj.get("command") {
            out.insert(String::from("command"), cmd.clone());
        }
        if let Some(cmd_id) = obj.get("command_id") {
            out.insert(String::from("command_id"), cmd_id.clone());
        }
        if let Some(inner) = obj.get("params") {
            out.insert(String::from("params"), inner.clone());
        } else {
            let mut inner = Map::new();
            for (key, value) in obj {
                if !matches!(key.as_str(), "id" | "method" | "op" | "command" | "command_id" | "params") {
                    inner.insert(key.clone(), value.clone());
                }
            }
            if !inner.is_empty() {
                out.insert(String::from("params"), Value::Object(inner));
            }
        }
        return Value::Object(out);
    }

    let mut base = match obj.get("params") {
        Some(Value::Object(map)) => map.clone(),
        Some(other) if !other.is_null() => return other.clone(),
        _ => Map::new(),
    };
    for (key, value) in obj {
        if matches!(key.as_str(), "id" | "method" | "op" | "params") {
            continue;
        }
        base.entry(key.clone()).or_insert_with(|| value.clone());
    }
    Value::Object(base)
}

fn serialize_response(resp: &ControlResponse) -> String {
    serde_json::to_string(resp).unwrap_or_else(|_| String::from("{\"id\":\"\",\"ok\":false,\"result\":null,\"error\":\"serialization failure\"}"))
}

fn handle_command_list(app: &UiApp) -> Value {
    let mut commands = Vec::with_capacity(COMMANDS.len() + UI_COMMANDS.len());
    for desc in COMMANDS {
        let active_shortcut = app.shortcuts.active_combo(desc.id).unwrap_or(desc.shortcut);
        commands.push(json!({
            "id": desc.id,
            "label": desc.label,
            "menu_path": desc.menu_path,
            "shortcut": active_shortcut,
            "params_doc": desc.params_doc,
            "ui": false,
        }));
    }
    for &(id, label, shortcut, params_doc) in UI_COMMANDS {
        let active_shortcut = app.shortcuts.active_combo(id).unwrap_or(shortcut);
        commands.push(json!({
            "id": id,
            "label": label,
            "menu_path": "UI",
            "shortcut": active_shortcut,
            "params_doc": params_doc,
            "ui": true,
        }));
    }
    json!({
        "commands": commands,
        "engine_count": COMMANDS.len(),
        "ui_count": UI_COMMANDS.len(),
    })
}

fn handle_command_run(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let command_id = extract_command_id(params)?;
    let cmd_params = extract_nested_params(params);
    if command_id == "app.quit" {
        return Err(String::from("use top-level method 'app.quit' to shut down the control server"));
    }
    dispatch_command_by_id(app, &command_id, cmd_params)
}

fn handle_command_batch(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let Some(items) = params.get("commands").and_then(Value::as_array) else {
        return Err(String::from("command.batch requires a 'commands' array"));
    };
    let atomic = params.get("atomic").and_then(Value::as_bool).unwrap_or(false);
    let rollback_snapshot = atomic.then(|| app.clone());

    let mut results = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let cmd_id = match extract_command_id(item) {
            Ok(id) => id,
            Err(err) => {
                if let Some(snapshot) = rollback_snapshot {
                    *app = snapshot;
                }
                return Err(format!("batch step {idx}: {err}"));
            }
        };
        let cmd_params = extract_nested_params(item);
        match dispatch_command_by_id(app, &cmd_id, cmd_params) {
            Ok(val) => results.push(json!({ "index": idx, "command": cmd_id, "result": val })),
            Err(err) => {
                if let Some(snapshot) = rollback_snapshot {
                    *app = snapshot;
                }
                return Err(format!("batch step {idx} ({cmd_id}) failed (rolled_back={atomic}): {err}"));
            }
        }
    }

    Ok(json!({
        "atomic": atomic,
        "count": results.len(),
        "results": results,
    }))
}

fn handle_engine_execute(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let cmd_id = params
        .get("command_id")
        .or_else(|| params.get("command"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| String::from("engine.execute requires non-empty 'command_id'"))?;
    let cmd_params = params.get("params").cloned().unwrap_or(Value::Null);
    let outcome = app.execute_command(cmd_id, cmd_params).map_err(|err| err.to_string())?;
    serde_json::to_value(outcome).map_err(|err| format!("failed to serialize engine outcome: {err}"))
}

fn handle_ui_state(app: &mut UiApp) -> Value {
    app.sync_views();
    let doc = app.session().document();
    json!({
        "title": doc.title,
        "sheet_count": doc.total_sheet_count(),
        "dirty": app.session().is_dirty(),
        "active_sheet_id": doc.active_sheet_id.as_ref().map(SheetId::as_str),
        "family_anchor_sheet_id": doc.family_anchor_sheet_id.as_ref().map(SheetId::as_str),
        "navigator_open": app.nav.navigator_open,
        "inspector_open": app.nav.inspector_open,
        "family_open": app.family_open,
        "gallery_open": app.nav.gallery.open,
        "active_pane": active_pane_name(app.nav.active_pane),
        "rulers_visible": app.canvas_dialogs.rulers_visible,
        "ruler_unit": physical_unit_name(app.canvas_dialogs.ruler_unit),
        "locale": app.locale,
        "locale_display_name": locale_display_name(&app.locale),
        "is_rtl": is_rtl_locale(&app.locale),
        "decimal_separator": app.locale_preset.decimal_separator.to_string(),
        "theme_mode": theme_mode_name(app.theme.mode),
        "wcag_aa": app.theme.meets_wcag_aa(),
        "zoom": app.canvas_dialogs.canvas.zoom,
        "active_dialog": active_dialog_name(&app.canvas_dialogs.active_dialog),
        "active_inspector": app.canvas_dialogs.active_inspector.label(),
        "command_palette_open": matches!(app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette),
        "welcome_open": app.nav.welcome.open,
    })
}

fn handle_doc_inspect(doc: &ProjectDocument, dirty: bool, params: &Value) -> Result<Value, String> {
    if let Some(sheet_id) = resolve_sheet_id_param(params)? {
        return inspect_single_sheet(doc, &sheet_id);
    }

    let data_sheets: Vec<Value> = doc
        .data_sheets
        .iter()
        .map(|s| {
            json!({
                "id": s.id.as_str(),
                "title": s.title,
                "kind": s.kind().label(),
                "rows": s.archetype.row_count(),
                "cols": s.archetype.column_count(),
                "frozen": s.frozen,
            })
        })
        .collect();
    let info_sheets: Vec<Value> =
        doc.info_sheets.iter().map(|s| json!({ "id": s.id.as_str(), "title": s.title, "constants": s.constants.len() })).collect();
    let result_sheets: Vec<Value> = doc
        .result_sheets
        .iter()
        .map(|s| {
            json!({
                "id": s.id.as_str(),
                "title": s.analysis.effective_title(),
                "status": format!("{:?}", s.status),
                "tabs": s.tabs.len(),
                "scalars": s.scalars.len(),
                "checklist": s.checklist.len(),
                "has_analyzable_data": s.analyzable_data.is_some(),
            })
        })
        .collect();
    let graph_sheets: Vec<Value> = doc
        .graph_sheets
        .iter()
        .map(|s| {
            json!({
                "id": s.id.as_str(),
                "title": s.title,
                "initialized": s.initialized,
                "kind": format!("{:?}", s.spec.kind),
                "width_pt": s.spec.canvas_size_pt.width,
                "height_pt": s.spec.canvas_size_pt.height,
            })
        })
        .collect();
    let layout_sheets: Vec<Value> = doc
        .layout_sheets
        .iter()
        .map(|s| {
            let size = s.spec.page_size_pt();
            json!({
                "id": s.id.as_str(),
                "title": s.title,
                "items": s.spec.items.len(),
                "width_pt": size.width,
                "height_pt": size.height,
            })
        })
        .collect();

    Ok(json!({
        "title": doc.title,
        "schema_version": doc.schema_version,
        "dirty": dirty,
        "active_sheet_id": doc.active_sheet_id.as_ref().map(SheetId::as_str),
        "family_anchor_sheet_id": doc.family_anchor_sheet_id.as_ref().map(SheetId::as_str),
        "counts": {
            "total": doc.total_sheet_count(),
            "data": doc.data_sheets.len(),
            "info": doc.info_sheets.len(),
            "results": doc.result_sheets.len(),
            "graphs": doc.graph_sheets.len(),
            "layouts": doc.layout_sheets.len(),
        },
        "data_sheets": data_sheets,
        "info_sheets": info_sheets,
        "result_sheets": result_sheets,
        "graph_sheets": graph_sheets,
        "layout_sheets": layout_sheets,
    }))
}

fn inspect_single_sheet(doc: &ProjectDocument, sheet_id: &SheetId) -> Result<Value, String> {
    match doc.sheet_section(sheet_id) {
        Some(SheetSection::Data) => doc
            .data_sheet(sheet_id)
            .map(|s| {
                json!({
                    "section": "Data",
                    "id": s.id.as_str(),
                    "title": s.title,
                    "kind": s.kind().label(),
                    "rows": s.archetype.row_count(),
                    "cols": s.archetype.column_count(),
                    "frozen": s.frozen,
                    "linked_regions": s.linked_regions.len(),
                })
            })
            .ok_or_else(|| format!("data sheet `{}` not found", sheet_id.as_str())),
        Some(SheetSection::Info) => doc
            .info_sheet(sheet_id)
            .map(|s| {
                json!({
                    "section": "Info",
                    "id": s.id.as_str(),
                    "title": s.title,
                    "constants": s.constants.len(),
                    "linked_data_sheets": s.linked_data_sheets.iter().map(SheetId::as_str).collect::<Vec<_>>(),
                })
            })
            .ok_or_else(|| format!("info sheet `{}` not found", sheet_id.as_str())),
        Some(SheetSection::Analyses) => doc
            .result_sheet(sheet_id)
            .map(|s| {
                json!({
                    "section": "Results",
                    "id": s.id.as_str(),
                    "title": s.analysis.effective_title(),
                    "source_sheet_id": s.analysis.source_sheet_id.as_str(),
                    "status": format!("{:?}", s.status),
                    "tabs": s.tabs.len(),
                    "scalars": s.scalars.len(),
                    "checklist": s.checklist.len(),
                })
            })
            .ok_or_else(|| format!("result sheet `{}` not found", sheet_id.as_str())),
        Some(SheetSection::Graphs) => doc
            .graph_sheet(sheet_id)
            .map(|s| {
                json!({
                    "section": "Graphs",
                    "id": s.id.as_str(),
                    "title": s.title,
                    "initialized": s.initialized,
                    "kind": format!("{:?}", s.spec.kind),
                    "width_pt": s.spec.canvas_size_pt.width,
                    "height_pt": s.spec.canvas_size_pt.height,
                })
            })
            .ok_or_else(|| format!("graph sheet `{}` not found", sheet_id.as_str())),
        Some(SheetSection::Layouts) => doc
            .layout_sheet(sheet_id)
            .map(|s| {
                let size = s.spec.page_size_pt();
                json!({
                    "section": "Layouts",
                    "id": s.id.as_str(),
                    "title": s.title,
                    "items": s.spec.items.len(),
                    "width_pt": size.width,
                    "height_pt": size.height,
                })
            })
            .ok_or_else(|| format!("layout sheet `{}` not found", sheet_id.as_str())),
        None => Err(format!("sheet `{}` not found", sheet_id.as_str())),
    }
}

fn handle_render_svg(app: &UiApp, params: &Value) -> Result<Value, String> {
    let sheet_id = resolve_renderable_sheet_id(app.document(), params)?;
    let scene = app.session().render_sheet_scene(&sheet_id).map_err(|err| err.to_string())?;
    let opts = SvgExportOptions {
        pretty: params.get("pretty").and_then(Value::as_bool).unwrap_or(true),
        outline_text: params.get("outline_text").and_then(Value::as_bool).unwrap_or(false),
        ..SvgExportOptions::default()
    };
    let svg = render_svg(&scene, &opts).map_err(|err| format!("SVG render failed: {err}"))?;
    Ok(json!({
        "sheet_id": sheet_id.as_str(),
        "title": scene.title,
        "width_pt": scene.width_pt,
        "height_pt": scene.height_pt,
        "svg": svg,
    }))
}

fn handle_render_pdf(app: &UiApp, params: &Value) -> Result<Value, String> {
    let doc = app.document();
    let all = params.get("all").and_then(Value::as_bool).unwrap_or(false);
    let sheet_ids = if all { collect_all_renderable_ids(doc)? } else { vec![resolve_renderable_sheet_id(doc, params)?] };

    let color_space = parse_pdf_color_space(params)?;
    let mut scenes = Vec::with_capacity(sheet_ids.len());
    for sheet_id in &sheet_ids {
        scenes.push(app.session().render_sheet_scene(sheet_id).map_err(|err| err.to_string())?);
    }

    let opts = PdfExportOptions { color_space, title: Some(doc.title.clone()), ..PdfExportOptions::default() };
    let pdf_bytes = if scenes.len() == 1 {
        render_pdf(&scenes[0], &opts).map_err(|err| format!("PDF render failed: {err}"))?
    } else {
        render_pdf_pages(&scenes, &opts).map_err(|err| format!("PDF render failed: {err}"))?
    };

    if let Some(path) = params.get("path").and_then(Value::as_str).map(str::trim).filter(|p| !p.is_empty()) {
        write_host_file(path, &pdf_bytes)?;
    }

    let data_base64 = encode_base64(&pdf_bytes);
    let id_strings: Vec<&str> = sheet_ids.iter().map(SheetId::as_str).collect();
    Ok(json!({
        "sheet_id": id_strings.first().copied(),
        "sheet_ids": id_strings,
        "page_count": scenes.len(),
        "encoding": "base64",
        "bytes_len": pdf_bytes.len(),
        "data_base64": data_base64,
    }))
}

fn dispatch_ui_analyze_open(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let doc = app.session().document().clone();
    let target_id = if let Some(explicit) = resolve_sheet_id_param(params)? {
        explicit
    } else if let Some(active) = &doc.active_sheet_id {
        if doc.data_sheet(active).is_some() || doc.result_sheet(active).and_then(|rs| rs.analyzable_data.as_ref()).is_some() {
            active.clone()
        } else {
            doc.data_sheets.first().map(|ds| ds.id.clone()).ok_or_else(|| String::from("no data or analyzable sheet available for Analyze dialog"))?
        }
    } else {
        doc.data_sheets.first().map(|ds| ds.id.clone()).ok_or_else(|| String::from("no data or analyzable sheet available for Analyze dialog"))?
    };

    if !app.canvas_dialogs.open_analyze_dialog(&doc, &target_id) {
        return Err(format!("sheet `{}` is not a data sheet or analyzable transform sheet", target_id.as_str()));
    }
    Ok(json!({
        "dialog": "AnalyzeSheet",
        "source_sheet_id": target_id.as_str(),
    }))
}

fn dispatch_ui_change_graph_type_open(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let doc = app.session().document().clone();
    let graph = if let Some(explicit) = resolve_sheet_id_param(params)? {
        doc.graph_sheet(&explicit).ok_or_else(|| format!("graph sheet `{}` not found", explicit.as_str()))?
    } else if let Some(active) = doc.active_sheet_id.as_ref().and_then(|id| doc.graph_sheet(id)) {
        active
    } else {
        doc.graph_sheets.first().ok_or_else(|| String::from("no graph sheet available to change graph type"))?
    };

    app.canvas_dialogs.change_graph_type.open_for_graph(graph, &doc);
    app.canvas_dialogs.active_dialog = ActiveDialog::ChangeGraphType { graph_sheet_id: graph.id.clone() };
    Ok(json!({
        "dialog": "ChangeGraphType",
        "graph_sheet_id": graph.id.as_str(),
    }))
}

fn dispatch_ui_clone_pipeline_open(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let explicit = resolve_sheet_id_param(params)?;
    let target = if let Some(id) = explicit {
        let ds = app.document().data_sheet(&id).ok_or_else(|| format!("data sheet `{}` not found for clone pipeline", id.as_str()))?;
        Some((ds.id.clone(), ds.title.clone()))
    } else if let Some(active_ds) = app.document().active_sheet_id.as_ref().and_then(|id| app.document().data_sheet(id)) {
        Some((active_ds.id.clone(), active_ds.title.clone()))
    } else {
        app.document().data_sheets.first().map(|ds| (ds.id.clone(), ds.title.clone()))
    };

    if let Some((source_id, title)) = target {
        app.nav.welcome.trigger_clone_pipeline(source_id.clone(), title);
        Ok(json!({
            "dialog": "ClonePipeline",
            "open": true,
            "source_sheet_id": source_id.as_str(),
        }))
    } else {
        app.nav.welcome.open = true;
        app.nav.welcome.mode = WelcomeMode::ClonePipeline;
        Ok(json!({
            "dialog": "ClonePipeline",
            "open": true,
            "source_sheet_id": Value::Null,
        }))
    }
}

const fn physical_unit_name(unit: PhysicalUnit) -> &'static str {
    match unit {
        PhysicalUnit::Millimeters => "mm",
        PhysicalUnit::Inches => "in",
        PhysicalUnit::Points => "pt",
    }
}

const fn active_pane_name(pane: ActiveWorkspacePane) -> &'static str {
    match pane {
        ActiveWorkspacePane::Navigator => "Navigator",
        ActiveWorkspacePane::FamilyStrip => "FamilyStrip",
        ActiveWorkspacePane::MainCanvasOrGrid => "MainCanvasOrGrid",
        ActiveWorkspacePane::Inspector => "Inspector",
        ActiveWorkspacePane::FloatingNotes => "FloatingNotes",
    }
}

const fn sheet_folder_kind_name(folder: SheetFolderKind) -> &'static str {
    match folder {
        SheetFolderKind::Data => "data",
        SheetFolderKind::Info => "info",
        SheetFolderKind::Results => "results",
        SheetFolderKind::Graphs => "graphs",
        SheetFolderKind::Layouts => "layouts",
    }
}

fn parse_gallery_folder(raw: &str) -> Result<Option<SheetFolderKind>, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "all" => Ok(None),
        "data" => Ok(Some(SheetFolderKind::Data)),
        "info" => Ok(Some(SheetFolderKind::Info)),
        "results" | "analyses" | "analysis" => Ok(Some(SheetFolderKind::Results)),
        "graphs" | "graph" => Ok(Some(SheetFolderKind::Graphs)),
        "layouts" | "layout" => Ok(Some(SheetFolderKind::Layouts)),
        other => Err(format!("unsupported gallery folder `{other}`; expected data, info, results, graphs, layouts, or all")),
    }
}

fn dispatch_ui_gallery_toggle(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    if let Some(folder_val) = params.get("folder") {
        let folder = match folder_val {
            Value::Null => None,
            Value::String(raw) => parse_gallery_folder(raw)?,
            other => return Err(format!("ui.gallery.toggle 'folder' must be a string, got {other}")),
        };
        app.nav.set_gallery_filter(folder, app.session.document());
    } else {
        app.nav.sync_gallery(app.session.document());
    }
    match params.get("open") {
        None | Some(Value::Null) => {
            app.nav.toggle_gallery();
        }
        Some(Value::Bool(explicit)) => {
            app.nav.gallery.open = *explicit;
        }
        Some(other) => return Err(format!("ui.gallery.toggle 'open' must be a boolean, got {other}")),
    }
    Ok(json!({
        "gallery_open": app.nav.gallery.open,
        "filter_folder": app.nav.gallery.filter_folder.map(sheet_folder_kind_name),
        "card_count": app.nav.gallery.cards.len(),
    }))
}

fn dispatch_ui_pane_cycle(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let reverse = match params.get("reverse") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(other) => return Err(format!("ui.pane.cycle 'reverse' must be a boolean, got {other}")),
    };
    let active = app.nav.cycle_focus_pane(reverse);
    Ok(json!({
        "active_pane": active_pane_name(active),
        "reverse": reverse,
    }))
}

fn dispatch_ui_rulers_toggle(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    if let Some(raw_unit) = params.get("unit").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        app.canvas_dialogs.ruler_unit = match raw_unit.to_ascii_lowercase().as_str() {
            "mm" | "millimeters" | "millimeter" => PhysicalUnit::Millimeters,
            "in" | "inch" | "inches" => PhysicalUnit::Inches,
            "pt" | "points" | "point" => PhysicalUnit::Points,
            other => return Err(format!("unsupported ruler unit `{other}`; expected pt, in, or mm")),
        };
    }
    if let Some(explicit) = params.get("visible").and_then(Value::as_bool) {
        app.canvas_dialogs.rulers_visible = explicit;
    } else {
        app.canvas_dialogs.rulers_visible = !app.canvas_dialogs.rulers_visible;
    }
    Ok(json!({
        "rulers_visible": app.canvas_dialogs.rulers_visible,
        "unit": physical_unit_name(app.canvas_dialogs.ruler_unit),
    }))
}

fn dispatch_ui_shortcut_set(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let Some(command_id) = params.get("command_id").or_else(|| params.get("id")).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
    else {
        return Err(String::from("ui.shortcut.set requires non-empty string 'command_id'"));
    };
    let Some(shortcut) = params.get("shortcut").or_else(|| params.get("combo")).and_then(Value::as_str) else {
        return Err(String::from("ui.shortcut.set requires string 'shortcut'"));
    };
    app.shortcuts.set_shortcut(command_id, shortcut)?;
    let active = app.shortcuts.active_combo(command_id).unwrap_or("");
    Ok(json!({
        "command_id": command_id,
        "shortcut": active,
    }))
}

fn dispatch_ui_shortcut_reset(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let target = params.get("command_id").or_else(|| params.get("id")).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    if let Some(command_id) = target {
        app.shortcuts.reset_shortcut(command_id)?;
        let active = app.shortcuts.active_combo(command_id).unwrap_or("");
        Ok(json!({
            "command_id": command_id,
            "shortcut": active,
            "reset_all": false,
        }))
    } else {
        app.shortcuts.reset_all();
        Ok(json!({
            "reset_all": true,
        }))
    }
}

fn dispatch_ui_locale_set(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let Some(raw_locale) = params.get("locale").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) else {
        return Err(String::from("ui.locale.set requires non-empty string 'locale'"));
    };
    let lower = raw_locale.replace('_', "-").to_ascii_lowercase();
    let primary = lower.split('-').next().unwrap_or("");
    if !matches!(primary, "en" | "de" | "es" | "fr" | "ja" | "ko" | "pt" | "zh" | "it" | "nl" | "pl" | "ru" | "tr" | "ar" | "he" | "iw") {
        return Err(format!(
            "unsupported locale `{raw_locale}`; expected one of en, de, es, fr, ja, ko, pt-BR, zh-CN, zh-TW, it, nl, pl, ru, tr, ar, he"
        ));
    }
    let normalized = normalize_locale(raw_locale);
    app.set_locale(normalized);
    Ok(json!({
        "locale": app.locale,
        "display_name": locale_display_name(&app.locale),
        "decimal_separator": app.locale_preset.decimal_separator.to_string(),
        "csv_delimiter": app.locale_preset.csv_delimiter.to_string(),
        "preset_applied": app.locale_preset == LocalePreset::for_locale(&app.locale),
    }))
}

fn dispatch_ui_theme_set(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let Some(raw_mode) = params.get("mode").or_else(|| params.get("theme")).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) else {
        return Err(String::from("ui.theme.set requires non-empty 'mode' (light | dark | high_contrast)"));
    };

    let mode = match raw_mode.replace(['-', ' '], "_").to_ascii_lowercase().as_str() {
        "light" => UiThemeMode::Light,
        "dark" => UiThemeMode::Dark,
        "high_contrast" | "highcontrast" => UiThemeMode::HighContrast,
        other => return Err(format!("unsupported theme mode `{other}`; expected light, dark, or high_contrast")),
    };
    app.theme = UiThemeSpec::for_mode(mode);
    Ok(json!({
        "mode": theme_mode_name(app.theme.mode),
        "wcag_aa": app.theme.meets_wcag_aa(),
    }))
}

fn dispatch_ui_screenshot(app: &UiApp, params: &Value) -> Result<Value, String> {
    let width_px = parse_bounded_u32(params.get("width_px"), DEFAULT_SCREENSHOT_WIDTH_PX, MIN_SCREENSHOT_DIM_PX, MAX_SCREENSHOT_DIM_PX, "width_px")?;
    let height_px =
        parse_bounded_u32(params.get("height_px"), DEFAULT_SCREENSHOT_HEIGHT_PX, MIN_SCREENSHOT_DIM_PX, MAX_SCREENSHOT_DIM_PX, "height_px")?;
    let dpi_u32 = parse_bounded_u32(params.get("dpi"), u32::from(DEFAULT_SCREENSHOT_DPI), u32::from(MIN_DPI), 600, "dpi")?;
    let dpi = u16::try_from(dpi_u32).map_err(|_| String::from("dpi out of range"))?;

    let png_bytes =
        render_workspace_frame_png(app, RenderTargetSpec { width_px, height_px, dpi }).map_err(|err| format!("screenshot capture failed: {err}"))?;

    let saved_path = match params.get("path").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        Some(path) => {
            write_host_file(path, &png_bytes)?;
            Some(path.to_owned())
        }
        None => None,
    };

    let data_base64 = encode_base64(&png_bytes);
    Ok(json!({
        "format": "png",
        "width_px": width_px,
        "height_px": height_px,
        "dpi": dpi,
        "bytes_len": png_bytes.len(),
        "data_base64": data_base64,
        "path": saved_path,
    }))
}

#[cfg(not(target_arch = "wasm32"))]
fn write_host_file(path: &str, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|err| format!("failed to write file `{path}`: {err}"))
}

#[cfg(target_arch = "wasm32")]
fn write_host_file(path: &str, _bytes: &[u8]) -> Result<(), String> {
    Err(format!("writing local filesystem path `{path}` is unsupported on wasm32"))
}

fn parse_bounded_u32(raw: Option<&Value>, default: u32, min: u32, max: u32, field: &str) -> Result<u32, String> {
    let Some(val) = raw.filter(|v| !v.is_null()) else {
        return Ok(default);
    };
    let Some(n) = val.as_u64() else {
        return Err(format!("`{field}` must be a positive integer"));
    };
    let parsed = u32::try_from(n).map_err(|_| format!("`{field}` exceeds supported u32 range"))?;
    if !(min..=max).contains(&parsed) {
        return Err(format!("`{field}` ({parsed}) must be within {min}..={max}"));
    }
    Ok(parsed)
}

fn extract_command_id(params: &Value) -> Result<String, String> {
    params
        .get("command")
        .or_else(|| params.get("command_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| String::from("missing non-empty 'command' or 'command_id' field"))
}

fn extract_nested_params(params: &Value) -> Value {
    if let Some(inner) = params.get("params") {
        return inner.clone();
    }
    if let Some(obj) = params.as_object() {
        let mut stripped = obj.clone();
        stripped.remove("command");
        stripped.remove("command_id");
        return if stripped.is_empty() { Value::Null } else { Value::Object(stripped) };
    }
    Value::Null
}

fn resolve_sheet_id_param(params: &Value) -> Result<Option<SheetId>, String> {
    let Some(val) = params.get("sheet_id").or_else(|| params.get("sheetId")).filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let Some(raw) = val.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
        return Err(String::from("'sheet_id'/'sheetId' must be a non-empty string"));
    };
    SheetId::try_new(raw).map(Some).map_err(|err| err.to_string())
}

fn resolve_renderable_sheet_id(doc: &ProjectDocument, params: &Value) -> Result<SheetId, String> {
    if let Some(explicit) = resolve_sheet_id_param(params)? {
        return Ok(explicit);
    }
    if let Some(active) = &doc.active_sheet_id
        && matches!(doc.sheet_section(active), Some(SheetSection::Graphs | SheetSection::Layouts))
    {
        return Ok(active.clone());
    }
    if let Some(graph) = doc.graph_sheets.first() {
        return Ok(graph.id.clone());
    }
    if let Some(layout) = doc.layout_sheets.first() {
        return Ok(layout.id.clone());
    }
    Err(String::from("no graph or layout sheet exists in project to render"))
}

fn collect_all_renderable_ids(doc: &ProjectDocument) -> Result<Vec<SheetId>, String> {
    let mut ids: Vec<SheetId> = doc.graph_sheets.iter().map(|g| g.id.clone()).collect();
    ids.extend(doc.layout_sheets.iter().map(|l| l.id.clone()));
    if ids.is_empty() {
        return Err(String::from("no graph or layout sheets exist in project to render"));
    }
    Ok(ids)
}

fn resolve_print_sheet_ids(requested: &[SheetId], doc: &ProjectDocument) -> Result<Vec<SheetId>, String> {
    if !requested.is_empty() {
        return Ok(requested.to_vec());
    }
    collect_all_renderable_ids(doc)
}

fn parse_pdf_color_space(params: &Value) -> Result<PdfColorSpace, String> {
    let Some(raw) = params.get("color_space").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(PdfColorSpace::Srgb);
    };
    match raw.to_ascii_lowercase().as_str() {
        "srgb" | "rgb" => Ok(PdfColorSpace::Srgb),
        "cmyk" => Ok(PdfColorSpace::Cmyk),
        "grayscale" | "gray" => Ok(PdfColorSpace::Grayscale),
        other => Err(format!("unsupported PDF color_space `{other}`; expected srgb, cmyk, or grayscale")),
    }
}

fn build_html_clipboard_wrapper(title: &str, svg: &str, width_pt: f64, height_pt: f64) -> String {
    let escaped_title = escape_html(title);
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>{escaped_title}</title></head>\
<body><div class=\"lumencraft-figure\" style=\"width:{width_pt:.3}pt;height:{height_pt:.3}pt;\">{svg}</div></body></html>"
    )
}

fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn scale_scene_for_print(scene: &ChartScene, factor: f64) -> ChartScene {
    if (factor - 1.0).abs() <= f64::EPSILON {
        return scene.clone();
    }
    let scale_pt = |pt: PointPt| PointPt::new(pt.x * factor, pt.y * factor);
    let scale_rect = |r: RectPt| RectPt::new(r.x * factor, r.y * factor, r.width * factor, r.height * factor);
    let primitives = scene.primitives.iter().map(|prim| transform_primitive(prim, factor, &scale_pt, &scale_rect)).collect();

    ChartScene {
        sheet_id: scene.sheet_id.clone(),
        title: scene.title.clone(),
        width_pt: (scene.width_pt * factor).max(1.0),
        height_pt: (scene.height_pt * factor).max(1.0),
        background: scene.background,
        plot_rect_pt: scale_rect(scene.plot_rect_pt),
        primitives,
        warnings: scene.warnings.clone(),
    }
}

fn decorate_scene_for_print(scene: &ChartScene, margin_pt: f64, crop_marks: bool) -> ChartScene {
    let shift_pt = |pt: PointPt| PointPt::new(pt.x + margin_pt, pt.y + margin_pt);
    let shift_rect = |r: RectPt| RectPt::new(r.x + margin_pt, r.y + margin_pt, r.width, r.height);
    let mut primitives: Vec<ScenePrimitive> = scene.primitives.iter().map(|prim| transform_primitive(prim, 1.0, &shift_pt, &shift_rect)).collect();

    if crop_marks {
        push_corner_crop_marks(&mut primitives, margin_pt, scene.width_pt, scene.height_pt);
    }

    ChartScene {
        sheet_id: scene.sheet_id.clone(),
        title: scene.title.clone(),
        width_pt: scene.width_pt + 2.0 * margin_pt,
        height_pt: scene.height_pt + 2.0 * margin_pt,
        background: scene.background,
        plot_rect_pt: shift_rect(scene.plot_rect_pt),
        primitives,
        warnings: scene.warnings.clone(),
    }
}

fn transform_primitive<FP, FR>(prim: &ScenePrimitive, scalar: f64, map_pt: &FP, map_rect: &FR) -> ScenePrimitive
where
    FP: Fn(PointPt) -> PointPt,
    FR: Fn(RectPt) -> RectPt,
{
    match prim {
        ScenePrimitive::Path { path, fill, stroke, clip } => {
            let commands = path
                .commands
                .iter()
                .map(|cmd| match *cmd {
                    PathCmd::MoveTo(pt) => PathCmd::MoveTo(map_pt(pt)),
                    PathCmd::LineTo(pt) => PathCmd::LineTo(map_pt(pt)),
                    PathCmd::QuadTo { control, to } => PathCmd::QuadTo { control: map_pt(control), to: map_pt(to) },
                    PathCmd::CubicTo { c1, c2, to } => PathCmd::CubicTo { c1: map_pt(c1), c2: map_pt(c2), to: map_pt(to) },
                    PathCmd::Close => PathCmd::Close,
                })
                .collect();
            let scaled_stroke = stroke.map(|mut s| {
                s.width_pt *= scalar;
                s
            });
            ScenePrimitive::Path { path: Path2D { commands }, fill: *fill, stroke: scaled_stroke, clip: clip.map(map_rect) }
        }
        ScenePrimitive::Text(text) => {
            let mut next = text.clone();
            next.anchor_pt = map_pt(text.anchor_pt);
            next.font_size_pt *= scalar;
            ScenePrimitive::Text(next)
        }
        ScenePrimitive::PushClip { rect } => ScenePrimitive::PushClip { rect: map_rect(*rect) },
        ScenePrimitive::PopClip => ScenePrimitive::PopClip,
    }
}

fn push_corner_crop_marks(primitives: &mut Vec<ScenePrimitive>, margin_pt: f64, trim_w: f64, trim_h: f64) {
    let x0 = margin_pt;
    let y0 = margin_pt;
    let x1 = margin_pt + trim_w;
    let y1 = margin_pt + trim_h;
    let gap = 3.0;
    let len = (margin_pt - gap - 2.0).max(6.0);
    let mark_stroke = Some(StrokePaint::solid(Rgba8::BLACK, 0.5));

    let segments = [
        (PointPt::new(x0 - gap - len, y0), PointPt::new(x0 - gap, y0)),
        (PointPt::new(x0, y0 - gap - len), PointPt::new(x0, y0 - gap)),
        (PointPt::new(x1 + gap, y0), PointPt::new(x1 + gap + len, y0)),
        (PointPt::new(x1, y0 - gap - len), PointPt::new(x1, y0 - gap)),
        (PointPt::new(x0 - gap - len, y1), PointPt::new(x0 - gap, y1)),
        (PointPt::new(x0, y1 + gap), PointPt::new(x0, y1 + gap + len)),
        (PointPt::new(x1 + gap, y1), PointPt::new(x1 + gap + len, y1)),
        (PointPt::new(x1, y1 + gap), PointPt::new(x1, y1 + gap + len)),
    ];
    for (from, to) in segments {
        primitives.push(ScenePrimitive::Path { path: Path2D::line(from, to), fill: None, stroke: mark_stroke, clip: None });
    }
}

fn compute_uniform_pdf_boxes(trim_sizes: &[(f64, f64)], outer_margin: f64, bleed_pt: f64, crop_marks: bool) -> (Option<RectPt>, Option<RectPt>) {
    if !crop_marks && bleed_pt <= 0.0 {
        return (None, None);
    }
    let Some(&(first_w, first_h)) = trim_sizes.first() else {
        return (None, None);
    };
    let all_same = trim_sizes.iter().all(|&(w, h)| (w - first_w).abs() < 1e-6 && (h - first_h).abs() < 1e-6);
    if !all_same {
        return (None, None);
    }
    let trim_box = RectPt::new(outer_margin, outer_margin, first_w, first_h);
    let bleed_box = if bleed_pt > 0.0 {
        let inset = (outer_margin - bleed_pt).max(0.0);
        RectPt::new(inset, inset, first_w + 2.0 * bleed_pt, first_h + 2.0 * bleed_pt)
    } else {
        trim_box
    };
    (Some(trim_box), Some(bleed_box))
}

fn read_bounded_control_line<R: BufRead>(reader: &mut R, max_bytes: usize) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let avail = reader.fill_buf()?;
        if avail.is_empty() {
            return Ok(if line.is_empty() { None } else { Some(line) });
        }
        if let Some(newline_pos) = avail.iter().position(|&b| b == b'\n') {
            let take = newline_pos + 1;
            if line.len().saturating_add(take) > max_bytes {
                return Err(io::Error::new(io::ErrorKind::InvalidData, format!("control line exceeds {max_bytes} bytes")));
            }
            line.extend_from_slice(&avail[..take]);
            reader.consume(take);
            return Ok(Some(line));
        }
        if line.len().saturating_add(avail.len()) > max_bytes {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("control line exceeds {max_bytes} bytes")));
        }
        line.extend_from_slice(avail);
        let consumed = avail.len();
        reader.consume(consumed);
    }
}

fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = usize::from(chunk[0]);
        let b1 = usize::from(*chunk.get(1).unwrap_or(&0));
        let b2 = usize::from(*chunk.get(2).unwrap_or(&0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(char::from(TABLE[(triple >> 18) & 0x3F]));
        out.push(char::from(TABLE[(triple >> 12) & 0x3F]));
        out.push(if chunk.len() > 1 { char::from(TABLE[(triple >> 6) & 0x3F]) } else { '=' });
        out.push(if chunk.len() > 2 { char::from(TABLE[triple & 0x3F]) } else { '=' });
    }
    out
}

const fn theme_mode_name(mode: UiThemeMode) -> &'static str {
    match mode {
        UiThemeMode::Light => "light",
        UiThemeMode::Dark => "dark",
        UiThemeMode::HighContrast => "high_contrast",
    }
}

const fn active_dialog_name(dialog: &ActiveDialog) -> &'static str {
    match dialog {
        ActiveDialog::None => "None",
        ActiveDialog::FormatAxes { .. } => "FormatAxes",
        ActiveDialog::AnalyzeSheet { .. } => "AnalyzeSheet",
        ActiveDialog::DuplicateAnalysisPrompt { .. } => "DuplicateAnalysisPrompt",
        ActiveDialog::EditAnalysisParameters { .. } => "EditAnalysisParameters",
        ActiveDialog::ChangeDataAnalyzed { .. } => "ChangeDataAnalyzed",
        ActiveDialog::ResultViewer { .. } => "ResultViewer",
        ActiveDialog::ConstantHookPicker { .. } => "ConstantHookPicker",
        ActiveDialog::ChangeGraphType { .. } => "ChangeGraphType",
        ActiveDialog::FormatGraph { .. } => "FormatGraph",
        ActiveDialog::PairwiseComparisons { .. } => "PairwiseComparisons",
        ActiveDialog::BubblePlotInspector { .. } => "BubblePlotInspector",
        ActiveDialog::ApplyStyleFrom { .. } => "ApplyStyleFrom",
        ActiveDialog::PageLayoutComposer { .. } => "PageLayoutComposer",
        ActiveDialog::ExportFigure { .. } => "ExportFigure",
        ActiveDialog::CommandPalette => "CommandPalette",
        ActiveDialog::About => "About",
    }
}

/// Execution / availability state returned by native OS pasteboard and print spool adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OsAdapterAvailability {
    Submitted,
    Read,
    Unavailable,
}

/// Structured status returned by OS pasteboard and print spool integration helpers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OsAdapterStatus {
    pub available: bool,
    pub availability: OsAdapterAvailability,
    pub backend: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
}

impl OsAdapterStatus {
    /// Construct an [`OsAdapterStatus`] representing a missing tool or headless environment.
    #[must_use]
    pub fn unavailable(backend: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            available: false,
            availability: OsAdapterAvailability::Unavailable,
            backend: backend.into(),
            detail: detail.into(),
            text: None,
            job_id: None,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone)]
struct OsCommandCandidate {
    program: String,
    args: Vec<String>,
}

#[cfg(not(target_arch = "wasm32"))]
impl OsCommandCandidate {
    fn new(program: impl Into<String>, args: &[&str]) -> Self {
        Self { program: program.into(), args: args.iter().map(|&s| s.to_owned()).collect() }
    }
}

/// Write UTF-8 text (`TSV`, `SVG`, or `HTML`) to the host OS clipboard using platform tools
/// (`pbcopy` on macOS, `wl-copy`/`xclip` on Linux, `clip`/`powershell` on Windows).
///
/// Missing OS clipboard binaries or headless display environments return
/// `Ok(OsAdapterStatus { available: false, availability: OsAdapterAvailability::Unavailable, .. })`
/// without panicking.
///
/// # Errors
/// Returns a structured error string if spawning or executing an available clipboard adapter fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_os_pasteboard(text: &str) -> Result<OsAdapterStatus, String> {
    let candidates = os_pasteboard_write_candidates();
    if candidates.is_empty() {
        return Ok(OsAdapterStatus::unavailable("os-pasteboard", "no desktop clipboard display session is active in this environment"));
    }
    write_os_pasteboard_with(&candidates, text)
}

/// Read UTF-8 text from the host OS clipboard using platform tools (`pbpaste` on macOS,
/// `wl-paste`/`xclip` on Linux, `powershell Get-Clipboard` on Windows).
///
/// Missing OS clipboard binaries or headless display environments return
/// `Ok(OsAdapterStatus { available: false, availability: OsAdapterAvailability::Unavailable, .. })`
/// without panicking.
///
/// # Errors
/// Returns a structured error string if an available clipboard adapter fails or returns non-UTF-8
/// output.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_os_pasteboard() -> Result<OsAdapterStatus, String> {
    let candidates = os_pasteboard_read_candidates();
    if candidates.is_empty() {
        return Ok(OsAdapterStatus::unavailable("os-pasteboard", "no desktop clipboard display session is active in this environment"));
    }
    read_os_pasteboard_with(&candidates)
}

/// Submit a physical-scale [`PrintSpoolOutput`] PDF payload to the host OS print spooler
/// (`lp` / `lpr`).
///
/// Missing print-spool binaries return
/// `Ok(OsAdapterStatus { available: false, availability: OsAdapterAvailability::Unavailable, .. })`
/// without panicking.
///
/// # Errors
/// Returns a structured error string if `spool.pdf_bytes` is not a valid PDF payload, `printer` is
/// invalid, or an available print-spool binary rejects the job.
#[cfg(not(target_arch = "wasm32"))]
pub fn submit_os_print_job(spool: &PrintSpoolOutput, printer: Option<&str>) -> Result<OsAdapterStatus, String> {
    submit_os_print_pdf_bytes(&spool.pdf_bytes, printer)
}

/// Submit raw `%PDF-` bytes to the host OS print spooler (`lp` / `lpr`).
///
/// # Errors
/// Returns a structured error string if `pdf_bytes` is not a valid PDF payload, `printer` is
/// invalid, or an available print-spool binary fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn submit_os_print_pdf_bytes(pdf_bytes: &[u8], printer: Option<&str>) -> Result<OsAdapterStatus, String> {
    let clean_printer = validate_printer_name(printer)?;
    let candidates = os_print_candidates(clean_printer);
    submit_os_print_bytes_with(&candidates, pdf_bytes)
}

#[cfg(not(target_arch = "wasm32"))]
fn validate_printer_name(printer: Option<&str>) -> Result<Option<&str>, String> {
    let Some(raw) = printer else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(String::from("printer name must not be empty"));
    }
    if trimmed.starts_with('-') {
        return Err(format!("invalid printer name `{trimmed}`: must not start with '-'"));
    }
    Ok(Some(trimmed))
}

#[cfg(not(target_arch = "wasm32"))]
fn write_os_pasteboard_with(candidates: &[OsCommandCandidate], text: &str) -> Result<OsAdapterStatus, String> {
    let mut missing = Vec::new();
    for candidate in candidates {
        match spawn_candidate_with_stdin(candidate, Some(text.as_bytes()))? {
            Some(output) => {
                let detail = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                return Ok(OsAdapterStatus {
                    available: true,
                    availability: OsAdapterAvailability::Submitted,
                    backend: candidate.program.clone(),
                    detail: if detail.is_empty() { format!("wrote {} bytes to OS pasteboard", text.len()) } else { detail },
                    text: None,
                    job_id: None,
                });
            }
            None => missing.push(candidate.program.as_str()),
        }
    }
    Ok(OsAdapterStatus::unavailable(missing.join("/"), "no OS clipboard adapter binary found on PATH"))
}

#[cfg(not(target_arch = "wasm32"))]
fn read_os_pasteboard_with(candidates: &[OsCommandCandidate]) -> Result<OsAdapterStatus, String> {
    let mut missing = Vec::new();
    for candidate in candidates {
        match spawn_candidate_with_stdin(candidate, None)? {
            Some(output) => {
                let text = String::from_utf8(output.stdout).map_err(|err| format!("OS pasteboard returned non-UTF-8 text: {err}"))?;
                return Ok(OsAdapterStatus {
                    available: true,
                    availability: OsAdapterAvailability::Read,
                    backend: candidate.program.clone(),
                    detail: format!("read {} bytes from OS pasteboard", text.len()),
                    text: Some(text),
                    job_id: None,
                });
            }
            None => missing.push(candidate.program.as_str()),
        }
    }
    Ok(OsAdapterStatus::unavailable(missing.join("/"), "no OS clipboard adapter binary found on PATH"))
}

#[cfg(not(target_arch = "wasm32"))]
fn submit_os_print_bytes_with(candidates: &[OsCommandCandidate], pdf_bytes: &[u8]) -> Result<OsAdapterStatus, String> {
    if !pdf_bytes.starts_with(b"%PDF-") {
        return Err(String::from("print spool payload must start with `%PDF-` signature"));
    }
    let mut missing = Vec::new();
    for candidate in candidates {
        match spawn_candidate_with_stdin(candidate, Some(pdf_bytes))? {
            Some(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                let job_id = (!stdout.is_empty()).then_some(stdout.clone());
                return Ok(OsAdapterStatus {
                    available: true,
                    availability: OsAdapterAvailability::Submitted,
                    backend: candidate.program.clone(),
                    detail: if stdout.is_empty() { format!("submitted {} PDF bytes to print spool", pdf_bytes.len()) } else { stdout },
                    text: None,
                    job_id,
                });
            }
            None => missing.push(candidate.program.as_str()),
        }
    }
    Ok(OsAdapterStatus::unavailable(missing.join("/"), "no OS print spooler binary (`lp`/`lpr`) found on PATH"))
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_candidate_with_stdin(candidate: &OsCommandCandidate, stdin_bytes: Option<&[u8]>) -> Result<Option<Output>, String> {
    let mut cmd = Command::new(&candidate.program);
    cmd.args(&candidate.args).stdout(Stdio::piped()).stderr(Stdio::piped());
    if stdin_bytes.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("failed to launch `{}`: {err}", candidate.program)),
    };

    if let Some(bytes) = stdin_bytes
        && let Some(mut stdin) = child.stdin.take()
    {
        stdin.write_all(bytes).map_err(|err| format!("failed writing payload to `{}` stdin: {err}", candidate.program))?;
    }

    let output = child.wait_with_output().map_err(|err| format!("failed waiting on `{}`: {err}", candidate.program))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let detail = if stderr.is_empty() { String::from("no stderr output") } else { stderr };
        return Err(format!("`{}` exited with {}: {detail}", candidate.program, output.status));
    }
    Ok(Some(output))
}

#[cfg(not(target_arch = "wasm32"))]
fn os_pasteboard_write_candidates() -> Vec<OsCommandCandidate> {
    #[cfg(target_os = "macos")]
    {
        vec![OsCommandCandidate::new("pbcopy", &[])]
    }
    #[cfg(target_os = "windows")]
    {
        vec![
            OsCommandCandidate::new("clip", &[]),
            OsCommandCandidate::new("powershell", &["-NoProfile", "-NonInteractive", "-Command", "$input | Set-Clipboard"]),
        ]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut out = Vec::new();
        if has_nonempty_env("WAYLAND_DISPLAY") {
            out.push(OsCommandCandidate::new("wl-copy", &[]));
        }
        if has_nonempty_env("DISPLAY") {
            out.push(OsCommandCandidate::new("xclip", &["-selection", "clipboard"]));
        }
        out
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn os_pasteboard_read_candidates() -> Vec<OsCommandCandidate> {
    #[cfg(target_os = "macos")]
    {
        vec![OsCommandCandidate::new("pbpaste", &[])]
    }
    #[cfg(target_os = "windows")]
    {
        vec![OsCommandCandidate::new("powershell", &["-NoProfile", "-NonInteractive", "-Command", "Get-Clipboard"])]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut out = Vec::new();
        if has_nonempty_env("WAYLAND_DISPLAY") {
            out.push(OsCommandCandidate::new("wl-paste", &["--no-newline"]));
        }
        if has_nonempty_env("DISPLAY") {
            out.push(OsCommandCandidate::new("xclip", &["-selection", "clipboard", "-o"]));
        }
        out
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn os_print_candidates(printer: Option<&str>) -> Vec<OsCommandCandidate> {
    let lp = match printer {
        Some(p) => OsCommandCandidate::new("lp", &["-d", p]),
        None => OsCommandCandidate::new("lp", &[]),
    };
    let lpr = match printer {
        Some(p) => OsCommandCandidate::new("lpr", &["-P", p]),
        None => OsCommandCandidate::new("lpr", &[]),
    };
    #[cfg(target_os = "windows")]
    {
        vec![lpr, lp]
    }
    #[cfg(not(target_os = "windows"))]
    {
        vec![lp, lpr]
    }
}

#[cfg(all(not(target_arch = "wasm32"), all(unix, not(target_os = "macos"))))]
fn has_nonempty_env(key: &str) -> bool {
    std::env::var_os(key).is_some_and(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{
        OsAdapterAvailability, OsCommandCandidate, PrintJobSpec, PrintSpoolOutput, read_os_pasteboard_with, submit_os_print_bytes_with,
        submit_os_print_job, write_os_pasteboard_with,
    };

    #[test]
    fn os_adapters_handle_missing_tools_and_invalid_inputs_safely() -> Result<(), Box<dyn std::error::Error>> {
        let missing = [OsCommandCandidate::new("lumencraft-nonexistent-os-tool-1", &[])];
        let write_unavail = write_os_pasteboard_with(&missing, "hello")?;
        assert!(!write_unavail.available);
        assert_eq!(write_unavail.availability, OsAdapterAvailability::Unavailable);

        let read_unavail = read_os_pasteboard_with(&missing)?;
        assert!(!read_unavail.available);
        assert_eq!(read_unavail.availability, OsAdapterAvailability::Unavailable);
        assert!(read_unavail.text.is_none());

        let print_unavail = submit_os_print_bytes_with(&missing, b"%PDF-1.7\n%%EOF\n")?;
        assert!(!print_unavail.available);
        assert_eq!(print_unavail.availability, OsAdapterAvailability::Unavailable);

        let bad_spool = PrintSpoolOutput { pdf_bytes: b"not-a-pdf".to_vec(), page_count: 0, page_sizes_pt: Vec::new(), scale_percent: 100.0 };
        assert!(submit_os_print_job(&bad_spool, None).is_err());

        let valid_spool = PrintSpoolOutput {
            pdf_bytes: b"%PDF-1.7\n%%EOF\n".to_vec(),
            page_count: 1,
            page_sizes_pt: vec![(100.0, 100.0)],
            scale_percent: PrintJobSpec::default().scale_percent,
        };
        assert!(submit_os_print_job(&valid_spool, Some("-malicious-flag")).is_err());
        assert!(submit_os_print_job(&valid_spool, Some("   ")).is_err());

        #[cfg(unix)]
        {
            let write_fallback = [OsCommandCandidate::new("lumencraft-missing-bin", &[]), OsCommandCandidate::new("sh", &["-c", "cat >/dev/null"])];
            let ok_write = write_os_pasteboard_with(&write_fallback, "x\ty\n1\t2\n")?;
            assert!(ok_write.available);
            assert_eq!(ok_write.availability, OsAdapterAvailability::Submitted);

            let read_mock = [OsCommandCandidate::new("printf", &["alpha\tbeta"])];
            let ok_read = read_os_pasteboard_with(&read_mock)?;
            assert!(ok_read.available);
            assert_eq!(ok_read.availability, OsAdapterAvailability::Read);
            assert_eq!(ok_read.text.as_deref(), Some("alpha\tbeta"));

            let print_mock = [OsCommandCandidate::new("sh", &["-c", "cat >/dev/null && printf 'request id is Mock-1'"])];
            let ok_print = submit_os_print_bytes_with(&print_mock, &valid_spool.pdf_bytes)?;
            assert!(ok_print.available);
            assert_eq!(ok_print.availability, OsAdapterAvailability::Submitted);
            assert_eq!(ok_print.job_id.as_deref(), Some("request id is Mock-1"));

            let failing = [OsCommandCandidate::new("sh", &["-c", "echo 'printer offline' >&2; exit 2"])];
            match submit_os_print_bytes_with(&failing, &valid_spool.pdf_bytes) {
                Err(err) => assert!(err.contains("printer offline")),
                Ok(other) => return Err(format!("expected failure status, got {other:?}").into()),
            }
        }
        Ok(())
    }

    fn dispatch_json(app: &mut crate::UiApp, req: &serde_json::Value) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        Ok(serde_json::from_str(&app.dispatch_control_line(&req.to_string()))?)
    }

    #[test]
    fn wave3_shortcuts_gallery_rulers_and_pane_cycle() -> Result<(), Box<dyn std::error::Error>> {
        use serde_json::json;

        use crate::UiApp;
        use crate::workspace_nav::ActiveWorkspacePane;

        let mut app = UiApp::new();
        assert_eq!(app.nav.active_pane, ActiveWorkspacePane::MainCanvasOrGrid);
        assert!(!app.nav.gallery.open);
        assert!(!app.canvas_dialogs.rulers_visible);

        // F6 forward and Shift+F6 reverse pane cycling through UiApp::handle_key_combo.
        let fwd = app.handle_key_combo("F6")?.ok_or("F6 should match ui.pane.cycle")?;
        assert_eq!(fwd.command_id, "ui.pane.cycle");
        assert!(fwd.ui);
        assert_eq!(app.nav.active_pane, ActiveWorkspacePane::Inspector);

        let rev = app.handle_key_combo("Shift+F6")?.ok_or("Shift+F6 should match ui.pane.cycle")?;
        assert_eq!(rev.command_id, "ui.pane.cycle");
        assert_eq!(app.nav.active_pane, ActiveWorkspacePane::MainCanvasOrGrid);

        // Custom shortcut set -> command.list reflects override -> handle_key_combo dispatches -> reset restores default.
        let set_reply = dispatch_json(
            &mut app,
            &json!({"id": 1, "method": "ui.shortcut.set", "params": {"command_id": "ui.gallery.toggle", "shortcut": "Ctrl+Shift+G"}}),
        )?;
        assert_eq!(set_reply["ok"], json!(true));
        assert_eq!(set_reply["result"]["shortcut"], json!("Ctrl+Shift+G"));

        let list_reply = dispatch_json(&mut app, &json!({"id": 2, "method": "command.list"}))?;
        let gallery_entry = list_reply["result"]["commands"]
            .as_array()
            .and_then(|arr| arr.iter().find(|entry| entry["id"] == "ui.gallery.toggle"))
            .ok_or("missing ui.gallery.toggle in command.list")?;
        assert_eq!(gallery_entry["shortcut"], json!("Ctrl+Shift+G"));

        let gallery_key = app.handle_key_combo("shift + ctrl + g")?.ok_or("custom gallery shortcut must match")?;
        assert_eq!(gallery_key.command_id, "ui.gallery.toggle");
        assert!(app.nav.gallery.open);

        let reset_reply = dispatch_json(&mut app, &json!({"id": 3, "method": "ui.shortcut.reset", "params": {"command_id": "ui.gallery.toggle"}}))?;
        assert_eq!(reset_reply["ok"], json!(true));
        assert!(app.handle_key_combo("Ctrl+Shift+G")?.is_none());

        // Create one data sheet so Portfolio Gallery filter returns a concrete card.
        app.execute_command("sheet.createData", json!({ "kind": "xy", "title": "Gallery Table", "auto_family": false }))?;

        // Control-line dispatch for ui.gallery.toggle, ui.rulers.toggle, ui.pane.cycle.
        let gallery_filter = dispatch_json(&mut app, &json!({"id": 4, "method": "ui.gallery.toggle", "params": {"open": true, "folder": "data"}}))?;
        assert_eq!(gallery_filter["ok"], json!(true));
        assert_eq!(gallery_filter["result"]["filter_folder"], json!("data"));
        assert_eq!(gallery_filter["result"]["card_count"], json!(1));

        let rulers_reply = dispatch_json(&mut app, &json!({"id": 5, "method": "ui.rulers.toggle", "params": {"visible": true, "unit": "mm"}}))?;
        assert_eq!(rulers_reply["ok"], json!(true));
        assert_eq!(rulers_reply["result"]["rulers_visible"], json!(true));
        assert_eq!(rulers_reply["result"]["unit"], json!("mm"));

        let pane_reply = dispatch_json(&mut app, &json!({"id": 6, "method": "ui.pane.cycle", "params": {"reverse": true}}))?;
        assert_eq!(pane_reply["ok"], json!(true));
        assert_eq!(pane_reply["result"]["active_pane"], json!("FamilyStrip"));

        let state = dispatch_json(&mut app, &json!({"id": 7, "method": "ui.state"}))?;
        assert_eq!(state["result"]["gallery_open"], json!(true));
        assert_eq!(state["result"]["rulers_visible"], json!(true));
        assert_eq!(state["result"]["ruler_unit"], json!("mm"));
        assert_eq!(state["result"]["active_pane"], json!("FamilyStrip"));

        Ok(())
    }

    #[test]
    fn wave3_locale_set_covers_all_16_locales_and_rtl() -> Result<(), Box<dyn std::error::Error>> {
        use serde_json::json;

        use crate::UiApp;
        use crate::i18n::LOCALES;

        let mut app = UiApp::new();
        assert_eq!(LOCALES.len(), 16);

        for &locale in LOCALES {
            let reply = dispatch_json(&mut app, &json!({"id": 1, "method": "ui.locale.set", "params": {"locale": locale}}))?;
            assert_eq!(reply["ok"], json!(true), "locale {locale} failed: {reply}");
        }

        let state_he = dispatch_json(&mut app, &json!({"id": 2, "method": "ui.state"}))?;
        assert_eq!(state_he["result"]["locale"], json!("he"));
        assert_eq!(state_he["result"]["is_rtl"], json!(true));

        let reply_en = dispatch_json(&mut app, &json!({"id": 3, "method": "ui.locale.set", "params": {"locale": "en"}}))?;
        assert_eq!(reply_en["ok"], json!(true));
        let state_en = dispatch_json(&mut app, &json!({"id": 4, "method": "ui.state"}))?;
        assert_eq!(state_en["result"]["locale"], json!("en"));
        assert_eq!(state_en["result"]["is_rtl"], json!(false));

        Ok(())
    }
}
