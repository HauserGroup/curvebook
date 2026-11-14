//! Local JSON-lines control channel and publication clipboard/print payload builders
//! (`docs/control-protocol.md`, `docs/hardware-parity.md`).

use std::io::{self, BufRead, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::net::{SocketAddr, TcpListener};

use lumencraft_chart::{ChartScene, Path2D, PathCmd, PointPt, RectPt, ScenePrimitive, StrokePaint};
use lumencraft_doc::{CanvasResizeMode, ChecklistStatus, ErrorCode, FigureCanvasPreset, ProjectDocument, SheetFolderKind, SheetId, SheetSection};
use lumencraft_engine::{EngineError, Session, cmd};
use lumencraft_palette::{CanvasPreviewFilter, Rgba8};
use lumencraft_pdf::{PdfColorSpace, PdfExportOptions, render_pdf, render_pdf_pages};
use lumencraft_svg::{SvgExportOptions, render_svg};
use lumencraft_tools::{GridCellCoord, PhysicalUnit};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, AssumptionDiagnosticBanner, RemediationKind};
use crate::grid_view::ImportReshapeMode;
use crate::gui_actions::{dispatch as dispatch_gui_action_impl, is_gui_action_method};
use crate::i18n::{LocalePreset, is_rtl_locale, locale_display_name, normalize_locale};
use crate::interaction::{UiInputEvent, ViewportMetrics, dispatch_ui_event};
use crate::menus::ui_commands::UI_COMMANDS;
pub use crate::os_integration::ClipboardBundle;
use crate::theme::{UiThemeMode, UiThemeSpec};
use crate::workspace_nav::{ActiveWorkspacePane, WelcomeMode};

/// Hard bound per JSON-lines control request/response frame (16 MiB).
pub const MAX_CONTROL_LINE_BYTES: usize = 16 * 1024 * 1024;

pub(crate) const DEFAULT_SCREENSHOT_WIDTH_PX: u32 = 1024;
pub(crate) const DEFAULT_SCREENSHOT_HEIGHT_PX: u32 = 640;
pub(crate) const DEFAULT_SCREENSHOT_DPI: u16 = 144;
pub(crate) const MIN_SCREENSHOT_DIM_PX: u32 = 64;
pub(crate) const MAX_SCREENSHOT_DIM_PX: u32 = 4096;
pub(crate) const MIN_DPI: u16 = 72;
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
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error_detail: Option<Value>,
}

impl ControlResponse {
    fn ok(id: Value, result: Value) -> Self {
        Self { id, ok: true, result, error: None, error_detail: None }
    }

    fn err(id: Value, message: impl Into<String>) -> Self {
        Self { id, ok: false, result: Value::Null, error: Some(message.into()), error_detail: None }
    }

    fn err_with_detail(id: Value, error: ControlCallError) -> Self {
        Self { id, ok: false, result: Value::Null, error: Some(error.message), error_detail: error.detail }
    }
}

struct ControlCallError {
    message: String,
    detail: Option<Value>,
}

impl From<String> for ControlCallError {
    fn from(message: String) -> Self {
        Self { message, detail: None }
    }
}

impl From<&'static str> for ControlCallError {
    fn from(message: &'static str) -> Self {
        Self { message: message.to_owned(), detail: None }
    }
}

impl From<EngineError> for ControlCallError {
    fn from(err: EngineError) -> Self {
        Self { message: err.to_string(), detail: Some(engine_error_detail(&err)) }
    }
}

fn engine_error_detail(err: &EngineError) -> Value {
    serde_json::to_value(err).unwrap_or_else(|_| {
        json!({
            "code": err.code.as_str(),
            "category": err.category,
            "message": err.message(),
            "retryable": err.retryable,
            "remediation": err.remediation,
            "details": err.details,
        })
    })
}

fn map_ui_result(res: Result<Value, String>) -> Result<Option<Value>, ControlCallError> {
    res.map(Some).map_err(ControlCallError::from)
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
/// Dispatch one of the 8 interactive GUI actions (`ui.inspect`, `ui.screenshot`,
/// `ui.pointer_gesture`, `ui.type_text`, `ui.press_key`, `ui.invoke_menu`, `ui.switch_tab`,
/// `ui.answer_dialog`) against `app` for in-process callers (`lumencraft-cli` MCP bridge, `R8.7`).
///
/// All workspace mutations invoked via these GUI interactions run as `Caller::Ui`.
///
/// # Errors
/// Returns a structured [`EngineError`] on invalid parameters, disabled UI controls/tabs/menus
/// (`ErrorCode::Conflict`), missing widgets (`ErrorCode::NotFound`), or blocked disk writes under
/// strict `--gui-only` (`ErrorCode::GuiOnlyViolation`).
pub fn dispatch_gui_action(app: &mut UiApp, method: &str, params: Value) -> Result<Value, EngineError> {
    dispatch_gui_action_impl(app, method, params)
}

/// Dispatch a UI command using `Caller::Ui` privileges (invoked internally by menus, shortcuts,
/// and the command palette).
pub(crate) fn dispatch_ui_command_as_ui(app: &mut UiApp, command_id: &str, params: &Value) -> Result<Option<Value>, EngineError> {
    dispatch_ui_command_detailed(app, command_id, params).map_err(|err| {
        if let Some(detail) = err.detail
            && let Ok(parsed) = serde_json::from_value::<EngineError>(detail)
        {
            return parsed;
        }
        EngineError::with_code(ErrorCode::Conflict, err.message)
    })
}

/// # Errors
/// Returns a descriptive error string when a recognized UI command fails validation or precondition checks.
pub fn dispatch_ui_command(app: &mut UiApp, command_id: &str, params: &Value) -> Result<Option<Value>, String> {
    dispatch_ui_command_detailed(app, command_id, params).map_err(|err| err.message)
}

fn dispatch_ui_command_detailed(app: &mut UiApp, command_id: &str, params: &Value) -> Result<Option<Value>, ControlCallError> {
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
        "ui.analyze.open" => map_ui_result(dispatch_ui_analyze_open(app, params)),
        "ui.powerWizard.open" => map_ui_result(dispatch_ui_power_wizard_open(app, params)),
        "ui.formatPoints.open" => map_ui_result(dispatch_ui_format_points_open(app, params)),
        "ui.changeGraphType.open" => map_ui_result(dispatch_ui_change_graph_type_open(app, params)),
        "ui.clonePipeline.open" => map_ui_result(dispatch_ui_clone_pipeline_open(app, params)),
        "ui.zoom.fitSheet" => {
            app.canvas_dialogs.reset_zoom_to_fit();
            Ok(Some(json!({ "zoom": app.canvas_dialogs.canvas.zoom })))
        }
        "ui.gallery.toggle" => map_ui_result(dispatch_ui_gallery_toggle(app, params)),
        "ui.pane.cycle" => map_ui_result(dispatch_ui_pane_cycle(app, params)),
        "ui.rulers.toggle" => map_ui_result(dispatch_ui_rulers_toggle(app, params)),
        "ui.shortcut.set" => map_ui_result(dispatch_ui_shortcut_set(app, params)),
        "ui.shortcut.reset" => map_ui_result(dispatch_ui_shortcut_reset(app, params)),
        "ui.locale.set" => map_ui_result(dispatch_ui_locale_set(app, params)),
        "ui.theme.set" => map_ui_result(dispatch_ui_theme_set(app, params)),
        "ui.screenshot" => dispatch_gui_action(app, "ui.screenshot", params.clone()).map(Some).map_err(ControlCallError::from),
        "ui.event.dispatch" => map_ui_result(dispatch_ui_event_cmd(app, params)),
        "ui.split.toggle" => map_ui_result(dispatch_ui_split_toggle(app, params)),
        "ui.brush.point" => map_ui_result(dispatch_ui_brush_point(app, params)),
        "ui.result.copyMethodsReport" => dispatch_ui_result_copy_methods_report(app, params).map(Some),
        "ui.result.applyRemediation" => dispatch_ui_result_apply_remediation(app, params).map(Some),
        "ui.canvas.applyPreset" => dispatch_ui_canvas_apply_preset(app, params).map(Some),
        "ui.canvas.preflight" => dispatch_ui_canvas_preflight(app, params).map(Some),
        "ui.previewFilter.set" => map_ui_result(dispatch_ui_preview_filter_set(app, params)),
        "ui.presentation.toggle" => map_ui_result(dispatch_ui_presentation_toggle(app, params)),
        "ui.grid.excludeWithReason" => dispatch_ui_grid_exclude_with_reason(app, params).map(Some),
        "ui.table.convertArchetype.open" => map_ui_result(dispatch_ui_table_convert_archetype_open(app, params)),
        "ui.import.reshape.open" => map_ui_result(dispatch_ui_import_reshape_open(app, params)),
        "ui.print.open" | "file.print" => map_ui_result(dispatch_ui_print_open(app, params)),
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

    let result = if is_gui_action_method(method.as_str()) {
        dispatch_gui_action(app, &method, params).map_err(ControlCallError::from)
    } else {
        match method.as_str() {
            "ping" => Ok(json!({
                "pong": true,
                "bundled_fonts": crate::bundled_fonts_enabled(),
                "version": env!("CARGO_PKG_VERSION"),
                "gui_only": app.is_gui_only(),
            })),
            "command.list" => Ok(handle_command_list(app)),
            "command.run" => handle_command_run(app, &params),
            "command.batch" => handle_command_batch(app, &params),
            "doc.inspect" => handle_doc_inspect(app.session().document(), app.session().is_dirty(), &params).map_err(ControlCallError::from),
            "render.svg" => handle_render_svg(app, &params),
            "render.pdf" => handle_render_pdf(app, &params),
            "ui.state" | "state.snapshot" => Ok(handle_ui_state(app)),
            "engine.execute" => handle_engine_execute(app, &params),
            _ => dispatch_command_by_id(app, &method, params),
        }
    };

    let response = match result {
        Ok(value) => ControlResponse::ok(id, value),
        Err(err) => ControlResponse::err_with_detail(id, err),
    };
    (response, ControlLoopAction::Continue)
}

fn is_control_readonly_ui_command(command_id: &str) -> bool {
    command_id == "ui.canvas.preflight"
}

fn dispatch_command_by_id(app: &mut UiApp, command_id: &str, params: Value) -> Result<Value, ControlCallError> {
    if is_gui_action_method(command_id) {
        return dispatch_gui_action(app, command_id, params).map_err(ControlCallError::from);
    }
    if app.is_gui_only()
        && !is_control_readonly_ui_command(command_id)
        && (command_id.starts_with("ui.") || command_id == "app.about" || command_id == "file.print")
    {
        return Err(EngineError::with_code(
            ErrorCode::GuiOnlyViolation,
            format!("direct `{command_id}` control request is blocked under --gui-only"),
        )
        .with_remediation("Interact via the 8 GUI actions (`ui.invoke_menu`, `ui.pointer_gesture`, `ui.press_key`, `ui.answer_dialog`).")
        .into());
    }
    if let Some(ui_value) = dispatch_ui_command_detailed(app, command_id, &params)? {
        return Ok(ui_value);
    }
    let outcome = app.execute_control_command(command_id, params)?;
    serde_json::to_value(outcome).map_err(|err| ControlCallError::from(format!("failed to serialize command outcome: {err}")))
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
    let engine_specs: Vec<_> = cmd::registry().specs().collect();
    let mut commands = Vec::with_capacity(engine_specs.len() + UI_COMMANDS.len());
    for spec in &engine_specs {
        let active_shortcut = app.shortcuts.active_combo(spec.id).unwrap_or(spec.shortcut);
        commands.push(json!({
            "id": spec.id,
            "label": spec.label,
            "menu_path": spec.menu_path,
            "shortcut": active_shortcut,
            "params_doc": spec.summary,
            "summary": spec.summary,
            "beginner": spec.beginner,
            "group": format!("{:?}", spec.group),
            "effects": spec.effects.flag_names(),
            "read_only": spec.effects.read_only_hint(),
            "destructive": spec.effects.destructive_hint(),
            "needs_consent": spec.effects.needs_consent(),
            "enabled": (spec.enabled)(&app.session),
            "ui": false,
        }));
    }
    for spec in UI_COMMANDS {
        let active_shortcut = app.shortcuts.active_combo(spec.id).unwrap_or(spec.shortcut);
        commands.push(json!({
            "id": spec.id,
            "label": spec.label,
            "menu_path": "UI",
            "shortcut": active_shortcut,
            "params_doc": spec.params_doc,
            "summary": spec.summary,
            "beginner": spec.beginner,
            "ui": true,
        }));
    }
    json!({
        "commands": commands,
        "engine_count": engine_specs.len(),
        "ui_count": UI_COMMANDS.len(),
    })
}

fn handle_command_run(app: &mut UiApp, params: &Value) -> Result<Value, ControlCallError> {
    let command_id = extract_command_id(params)?;
    let cmd_params = extract_nested_params(params);
    if command_id == "app.quit" {
        return Err(ControlCallError::from("use top-level method 'app.quit' to shut down the control server"));
    }
    dispatch_command_by_id(app, &command_id, cmd_params)
}

fn handle_command_batch(app: &mut UiApp, params: &Value) -> Result<Value, ControlCallError> {
    let Some(items) = params.get("commands").and_then(Value::as_array) else {
        return Err(ControlCallError::from("command.batch requires a 'commands' array"));
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
                return Err(ControlCallError::from(format!("batch step {idx}: {err}")));
            }
        };
        let cmd_params = extract_nested_params(item);
        match dispatch_command_by_id(app, &cmd_id, cmd_params) {
            Ok(val) => results.push(json!({ "index": idx, "command": cmd_id, "result": val })),
            Err(err) => {
                if let Some(snapshot) = rollback_snapshot {
                    *app = snapshot;
                }
                return Err(ControlCallError {
                    message: format!("batch step {idx} ({cmd_id}) failed (rolled_back={atomic}): {}", err.message),
                    detail: err.detail,
                });
            }
        }
    }

    Ok(json!({
        "atomic": atomic,
        "count": results.len(),
        "results": results,
    }))
}

fn handle_engine_execute(app: &mut UiApp, params: &Value) -> Result<Value, ControlCallError> {
    let cmd_id = params
        .get("command_id")
        .or_else(|| params.get("command"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ControlCallError::from("engine.execute requires non-empty 'command_id'"))?;
    let cmd_params = params.get("params").cloned().unwrap_or(Value::Null);
    let outcome = app.execute_control_command(cmd_id, cmd_params)?;
    serde_json::to_value(outcome).map_err(|err| ControlCallError::from(format!("failed to serialize engine outcome: {err}")))
}

fn handle_ui_state(app: &mut UiApp) -> Value {
    app.sync_views();
    let doc = app.session().document();
    json!({
        "title": doc.title,
        "sheet_count": doc.total_sheet_count(),
        "dirty": app.session().is_dirty(),
        "gui_only": app.is_gui_only(),
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
        "split_enabled": app.split.enabled,
        "split_ratio": app.split.ratio,
        "preview_filter": app.preview_filter.as_str(),
        "presentation_mode": app.presentation_mode,
        "brushed_cell": app.brushed_cell,
        "brushed_points": app.brushed_points,
        "selected_rows": app.grid.selected_rows,
        "inline_editor_active": app.inline_editor.active,
        "preflight_issue_count": app.canvas_dialogs.preflight_issues.len(),
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

fn handle_render_svg(app: &UiApp, params: &Value) -> Result<Value, ControlCallError> {
    let sheet_id = resolve_renderable_sheet_id(app.document(), params)?;
    let scene = app.session().render_sheet_scene(&sheet_id)?;
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

fn handle_render_pdf(app: &UiApp, params: &Value) -> Result<Value, ControlCallError> {
    let doc = app.document();
    let all = params.get("all").and_then(Value::as_bool).unwrap_or(false);
    let sheet_ids = if all { collect_all_renderable_ids(doc)? } else { vec![resolve_renderable_sheet_id(doc, params)?] };

    let color_space = parse_pdf_color_space(params)?;
    let mut scenes = Vec::with_capacity(sheet_ids.len());
    for sheet_id in &sheet_ids {
        scenes.push(app.session().render_sheet_scene(sheet_id)?);
    }

    let opts = PdfExportOptions { color_space, title: Some(doc.title.clone()), ..PdfExportOptions::default() };
    let pdf_bytes = if scenes.len() == 1 {
        render_pdf(&scenes[0], &opts).map_err(|err| format!("PDF render failed: {err}"))?
    } else {
        render_pdf_pages(&scenes, &opts).map_err(|err| format!("PDF render failed: {err}"))?
    };

    if let Some(path) = params.get("path").and_then(Value::as_str).map(str::trim).filter(|p| !p.is_empty()) {
        if app.is_gui_only() {
            return Err(EngineError::with_code(
                ErrorCode::GuiOnlyViolation,
                "direct filesystem writes via `render.pdf` `path` are blocked under --gui-only",
            )
            .with_remediation("Inspect the returned base64 PDF payload or trigger Export through GUI dialogs.")
            .into());
        }
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
        let is_analyzable_result = doc
            .result_sheet(active)
            .is_some_and(|rs| rs.analyzable_data.is_some() || rs.tabs.iter().any(|t| !t.column_headers.is_empty() && !t.rows.is_empty()));
        if doc.data_sheet(active).is_some() || is_analyzable_result {
            active.clone()
        } else {
            doc.data_sheets.first().map(|ds| ds.id.clone()).ok_or_else(|| String::from("no data or analyzable sheet available for Analyze dialog"))?
        }
    } else {
        doc.data_sheets.first().map(|ds| ds.id.clone()).ok_or_else(|| String::from("no data or analyzable sheet available for Analyze dialog"))?
    };

    if !app.canvas_dialogs.open_analyze_dialog(&doc, &target_id) {
        return Err(format!("sheet `{}` is not a data sheet or tabular result sheet", target_id.as_str()));
    }
    Ok(json!({
        "dialog": "AnalyzeSheet",
        "source_sheet_id": target_id.as_str(),
        "extracted_result_tab_id": app.canvas_dialogs.analyze.extracted_result_tab_id,
    }))
}

fn dispatch_ui_power_wizard_open(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    app.canvas_dialogs.open_power_wizard();
    if let Some(fields) = params.as_object()
        && !fields.is_empty()
    {
        app.canvas_dialogs.power_wizard.apply_field_patch(fields)?;
    }
    Ok(json!({
        "dialog": "PowerAndSampleSize",
        "stage": app.canvas_dialogs.power_wizard.stage.label(),
        "stage_index": app.canvas_dialogs.power_wizard.stage.index(),
    }))
}

fn dispatch_ui_format_points_open(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let doc = app.document().clone();
    let graph = if let Some(explicit) = resolve_sheet_id_param(params)? {
        doc.graph_sheet(&explicit).ok_or_else(|| format!("graph sheet `{}` not found", explicit.as_str()))?
    } else if let Some(active) = doc.active_sheet_id.as_ref().and_then(|id| doc.graph_sheet(id)) {
        active
    } else {
        doc.graph_sheets.first().ok_or_else(|| String::from("no graph sheet available to format points"))?
    };

    let mut points: Vec<(usize, usize)> = if let Some(arr) = params.get("points").and_then(Value::as_array) {
        arr.iter()
            .filter_map(|item| {
                let s = item.get("series_index").and_then(Value::as_u64)? as usize;
                let p = item.get("point_index").and_then(Value::as_u64)? as usize;
                Some((s, p))
            })
            .collect()
    } else if !app.brushed_points.is_empty() {
        app.brushed_points.iter().map(|pt| (pt.col, pt.row)).collect()
    } else if let Some(ds) = doc.data_sheet(&graph.spec.primary_source_sheet_id) {
        app.grid.selected_graph_point_indices(ds, graph.spec.series.len().max(1))
    } else {
        Vec::new()
    };
    points.sort_unstable();
    points.dedup();

    let graph_id = graph.id.clone();
    app.canvas_dialogs.open_format_selected_points(&doc, &graph_id, points);
    if let Some(fields) = params.as_object()
        && !fields.is_empty()
    {
        app.canvas_dialogs.format_points.apply_field_patch(fields)?;
    }
    Ok(json!({
        "dialog": "FormatSelectedPoints",
        "graph_sheet_id": graph_id.as_str(),
        "selected_count": app.canvas_dialogs.format_points.selected_points.len(),
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

fn dispatch_ui_event_cmd(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let ev_val = params.get("event").unwrap_or(params);
    let event: UiInputEvent = serde_json::from_value(ev_val.clone()).map_err(|err| format!("invalid ui.event.dispatch payload: {err}"))?;
    let metrics: ViewportMetrics = if let Some(m) = params.get("metrics") {
        serde_json::from_value(m.clone()).map_err(|err| format!("invalid viewport metrics: {err}"))?
    } else {
        ViewportMetrics::default()
    };
    let outcome = dispatch_ui_event(app, event, metrics)?;
    serde_json::to_value(outcome).map_err(|err| format!("failed to serialize ui event outcome: {err}"))
}

fn dispatch_ui_split_toggle(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    if let Some(enabled) = params.get("enabled").and_then(Value::as_bool) {
        app.split.enabled = enabled;
    } else {
        app.split.enabled = !app.split.enabled;
    }
    if let Some(ratio) = params.get("ratio").and_then(Value::as_f64) {
        app.split.set_ratio(ratio);
    }
    if let Some(raw_p) = params.get("primary_sheet_id").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        app.split.primary_sheet_id = Some(SheetId::try_new(raw_p).map_err(|e| e.to_string())?);
    }
    if let Some(raw_s) = params.get("secondary_sheet_id").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        app.split.secondary_sheet_id = Some(SheetId::try_new(raw_s).map_err(|e| e.to_string())?);
    }
    let (primary, secondary) = app.split.resolve_sheets(app.document());
    Ok(json!({
        "enabled": app.split.enabled,
        "ratio": app.split.ratio,
        "primary_sheet_id": primary.as_ref().map(SheetId::as_str),
        "secondary_sheet_id": secondary.as_ref().map(SheetId::as_str),
    }))
}

fn dispatch_ui_brush_point(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    if params.get("clear").and_then(Value::as_bool) == Some(true) {
        app.clear_brush();
        return Ok(json!({ "brushed_cell": Value::Null }));
    }
    let doc = app.document();
    let sheet_id = if let Some(id) = resolve_sheet_id_param(params)? {
        id
    } else if let Some(active) = doc.active_sheet_id.as_ref().filter(|id| doc.data_sheet(id).is_some()) {
        active.clone()
    } else {
        doc.data_sheets.first().map(|d| d.id.clone()).ok_or_else(|| String::from("no data sheet available to brush"))?
    };
    let row = usize::try_from(params.get("row").and_then(Value::as_u64).unwrap_or(0)).map_err(|_| String::from("`row` exceeds usize"))?;
    let col = usize::try_from(params.get("col").and_then(Value::as_u64).unwrap_or(0)).map_err(|_| String::from("`col` exceeds usize"))?;
    let subcol = usize::try_from(params.get("subcol").and_then(Value::as_u64).unwrap_or(0)).map_err(|_| String::from("`subcol` exceeds usize"))?;
    app.brush_from_grid_cell(&sheet_id, GridCellCoord::new(row, col, subcol));
    Ok(json!({ "brushed_cell": app.brushed_cell }))
}

fn dispatch_ui_result_copy_methods_report(app: &mut UiApp, params: &Value) -> Result<Value, ControlCallError> {
    let doc = app.document();
    let rs_id = if let Some(raw) = params.get("result_sheet_id").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        SheetId::try_new(raw).map_err(|e| e.to_string())?
    } else if let Some(explicit) = resolve_sheet_id_param(params)? {
        explicit
    } else if let Some(active) = doc.active_sheet_id.as_ref().filter(|id| doc.result_sheet(id).is_some()) {
        active.clone()
    } else {
        doc.result_sheets.first().map(|r| r.id.clone()).ok_or_else(|| String::from("no result sheet available for methods report"))?
    };

    if let Some(fmt) = params.get("format").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        app.canvas_dialogs.result_view.report_format = fmt.to_owned();
    }
    let intent = app.canvas_dialogs.result_view.generate_report_intent(&rs_id);
    let outcome = app.execute_command(&intent.command_id, intent.params)?;
    let report = outcome.payload.get("report").and_then(Value::as_str).unwrap_or_default().to_owned();
    app.canvas_dialogs.result_view.last_methods_report = Some(report.clone());

    let mut clipboard_status = Value::Null;
    #[cfg(not(target_arch = "wasm32"))]
    if params.get("skip_os_clipboard").and_then(Value::as_bool) != Some(true)
        && let Ok(status) = write_os_pasteboard(&report)
    {
        clipboard_status = serde_json::to_value(status).unwrap_or(Value::Null);
    }

    Ok(json!({
        "sheet_id": rs_id.as_str(),
        "format": app.canvas_dialogs.result_view.report_format,
        "report": report,
        "clipboard_status": clipboard_status,
    }))
}

fn dispatch_ui_result_apply_remediation(app: &mut UiApp, params: &Value) -> Result<Value, ControlCallError> {
    let doc = app.document().clone();
    let rs_id = if let Some(raw) = params.get("result_sheet_id").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        SheetId::try_new(raw).map_err(|e| e.to_string())?
    } else if let Some(explicit) = resolve_sheet_id_param(params)? {
        explicit
    } else if let Some(active) = doc.active_sheet_id.as_ref().filter(|id| doc.result_sheet(id).is_some()) {
        active.clone()
    } else {
        doc.result_sheets.first().map(|r| r.id.clone()).ok_or_else(|| String::from("no result sheet available for remediation"))?
    };

    let rs = doc.result_sheet(&rs_id).ok_or_else(|| format!("result sheet `{}` not found", rs_id.as_str()))?;
    let mut banner = AssumptionDiagnosticBanner::from_result(rs).unwrap_or_else(|| AssumptionDiagnosticBanner {
        source_result_id: rs.id.clone(),
        severity: ChecklistStatus::Warn,
        summary: String::from("Manual remediation requested"),
        recommendation_label: String::from("Run robust non-parametric alternative"),
        remediation_kind: RemediationKind::WelchT,
    });
    if let Some(raw_kind) = params.get("kind").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        banner.remediation_kind = match raw_kind.to_ascii_lowercase().as_str() {
            "welch_t" | "welcht" => RemediationKind::WelchT,
            "mann_whitney" | "mannwhitney" => RemediationKind::MannWhitneyU,
            "wilcoxon" | "wilcoxon_signed_rank" => RemediationKind::WilcoxonSignedRank,
            "one_sample_wilcoxon" => RemediationKind::OneSampleWilcoxon,
            "welch_anova" => RemediationKind::WelchAnova,
            "kruskal_wallis" | "kruskalwallis" => RemediationKind::KruskalWallis,
            "friedman" => RemediationKind::Friedman,
            other => return Err(ControlCallError::from(format!("unsupported remediation kind `{other}`"))),
        };
    } else if AssumptionDiagnosticBanner::from_result(rs).is_none() {
        return Err(ControlCallError::from(format!("no assumption violation banner active for result sheet `{}`", rs_id.as_str())));
    }

    let sibling_id = next_sibling_result_id(&doc, rs_id.as_str())?;
    let intent = banner.build_sibling_analysis_intent(rs, &sibling_id);
    app.execute_command(&intent.command_id, intent.params)?;
    let doc_after = app.document().clone();
    app.canvas_dialogs.on_sheet_selected(&doc_after, &sibling_id);
    Ok(json!({
        "source_result_id": rs_id.as_str(),
        "sibling_result_id": sibling_id.as_str(),
        "remediation_kind": banner.remediation_kind.engine_kind(),
        "label": banner.remediation_kind.label(),
    }))
}

fn next_sibling_result_id(doc: &ProjectDocument, base: &str) -> Result<SheetId, String> {
    let mut suffix = 1usize;
    loop {
        let candidate_raw = format!("{base}-remediation-{suffix}");
        let candidate = SheetId::try_new(&candidate_raw).map_err(|err| err.to_string())?;
        if doc.sheet_section(&candidate).is_none() {
            return Ok(candidate);
        }
        suffix = suffix.saturating_add(1);
    }
}

fn dispatch_ui_canvas_apply_preset(app: &mut UiApp, params: &Value) -> Result<Value, ControlCallError> {
    let sheet_id = resolve_renderable_sheet_id(app.document(), params)?;
    let preset_raw = params
        .get("preset")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| String::from("missing required `preset` string parameter"))?;
    let preset = FigureCanvasPreset::from_name(preset_raw).ok_or_else(|| format!("unknown canvas preset `{preset_raw}`"))?;
    if let Some(mode_raw) = params.get("resize_mode").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        app.canvas_dialogs.canvas_resize_mode =
            CanvasResizeMode::from_name(mode_raw).ok_or_else(|| format!("unknown canvas resize_mode `{mode_raw}`"))?;
    }
    let is_layout = app.document().layout_sheet(&sheet_id).is_some();
    let intent = app.canvas_dialogs.apply_canvas_preset_intent(&sheet_id, is_layout, preset);
    app.execute_command(&intent.command_id, intent.params)?;
    if let Ok(scene) = app.session().render_sheet_scene(&sheet_id) {
        app.canvas_dialogs.refresh_preflight(&scene);
    }
    Ok(json!({
        "sheet_id": sheet_id.as_str(),
        "preset": format!("{preset:?}"),
        "resize_mode": format!("{:?}", app.canvas_dialogs.canvas_resize_mode),
        "preflight_issues": app.canvas_dialogs.preflight_issues,
    }))
}

fn dispatch_ui_canvas_preflight(app: &mut UiApp, params: &Value) -> Result<Value, ControlCallError> {
    let sheet_id = resolve_renderable_sheet_id(app.document(), params)?;
    let scene = app.session().render_sheet_scene(&sheet_id)?;
    app.canvas_dialogs.refresh_preflight(&scene);
    Ok(json!({
        "sheet_id": sheet_id.as_str(),
        "passed": app.canvas_dialogs.preflight_issues.is_empty(),
        "issue_count": app.canvas_dialogs.preflight_issues.len(),
        "issues": app.canvas_dialogs.preflight_issues,
    }))
}

fn dispatch_ui_preview_filter_set(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let raw = params
        .get("filter")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| String::from("missing non-empty `filter` string"))?;
    let filter = CanvasPreviewFilter::from_name(raw).ok_or_else(|| format!("unknown preview filter `{raw}`"))?;
    app.preview_filter = filter;
    Ok(json!({ "preview_filter": app.preview_filter.as_str() }))
}

fn dispatch_ui_presentation_toggle(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    if let Some(enabled) = params.get("enabled").and_then(Value::as_bool) {
        app.presentation_mode = enabled;
    } else {
        app.presentation_mode = !app.presentation_mode;
    }
    Ok(json!({ "presentation_mode": app.presentation_mode }))
}

fn dispatch_ui_grid_exclude_with_reason(app: &mut UiApp, params: &Value) -> Result<Value, ControlCallError> {
    let doc = app.document();
    let sheet_id = if let Some(id) = resolve_sheet_id_param(params)? {
        id
    } else if let Some(active) = doc.active_sheet_id.as_ref().filter(|id| doc.data_sheet(id).is_some()) {
        active.clone()
    } else {
        doc.data_sheets.first().map(|d| d.id.clone()).ok_or_else(|| String::from("no data sheet available for exclusion"))?
    };
    let ds = doc.data_sheet(&sheet_id).ok_or_else(|| format!("data sheet `{}` not found", sheet_id.as_str()))?;
    let mut coords = app.grid.selection_coords(ds);
    if coords.is_empty()
        && let Some(active) = app.grid.tool.active
    {
        coords.push(active);
    }
    app.canvas_dialogs.open_exclude_with_reason(sheet_id.clone(), coords.clone());
    if let Some(tag) = params.get("reason_tag").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        app.canvas_dialogs.exclude_with_reason.reason_tag = tag.to_owned();
    }
    if let Some(note) = params.get("note").and_then(Value::as_str) {
        app.canvas_dialogs.exclude_with_reason.note = note.to_owned();
    }
    if params.get("commit").and_then(Value::as_bool) == Some(true) {
        let intent = app.canvas_dialogs.exclude_with_reason.commit_intent()?;
        app.execute_command(&intent.command_id, intent.params)?;
        app.canvas_dialogs.close_dialog();
        return Ok(json!({
            "committed": true,
            "sheet_id": sheet_id.as_str(),
            "reason_tag": app.canvas_dialogs.exclude_with_reason.reason_tag,
            "count": coords.len(),
        }));
    }
    Ok(json!({
        "dialog": "ExcludeWithReason",
        "sheet_id": sheet_id.as_str(),
        "coords_count": app.canvas_dialogs.exclude_with_reason.coords.len(),
    }))
}

fn dispatch_ui_table_convert_archetype_open(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let doc = app.document().clone();
    let sheet_id = if let Some(id) = resolve_sheet_id_param(params)? {
        id
    } else if let Some(active) = doc.active_sheet_id.as_ref().filter(|id| doc.data_sheet(id).is_some()) {
        active.clone()
    } else {
        doc.data_sheets.first().map(|d| d.id.clone()).ok_or_else(|| String::from("no data sheet available to convert"))?
    };
    if !app.canvas_dialogs.open_convert_archetype(&doc, &sheet_id) {
        return Err(format!("data sheet `{}` not found", sheet_id.as_str()));
    }
    Ok(json!({
        "dialog": "ConvertArchetype",
        "sheet_id": sheet_id.as_str(),
    }))
}

fn dispatch_ui_import_reshape_open(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    if let Some(mode_raw) = params.get("mode").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        app.grid.import_reshape.mode = match mode_raw.to_ascii_lowercase().as_str() {
            "plate96" | "plate_96" | "96" => ImportReshapeMode::Plate96,
            "plate384" | "plate_384" | "384" => ImportReshapeMode::Plate384,
            "tidylongpivot" | "tidy_long_pivot" | "tidylong" | "tidy" => ImportReshapeMode::TidyLongPivot,
            other => return Err(format!("unknown import reshape mode `{other}`")),
        };
    }
    app.canvas_dialogs.open_import_reshape();
    Ok(json!({
        "dialog": "ImportReshape",
        "mode": format!("{:?}", app.grid.import_reshape.mode),
    }))
}

fn dispatch_ui_print_open(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let sheet_override = resolve_sheet_id_param(params)?;
    let active = app.session.document().active_sheet_id.clone();
    app.open_print_dialog_for(sheet_override.as_ref().or(active.as_ref()));

    if let Some(copies) = params.get("copies").and_then(Value::as_u64) {
        let clamped = u16::try_from(copies.clamp(1, 999)).unwrap_or(999);
        app.update_print_dialog_copies(clamped);
    }
    if let Some(printer_val) = params.get("printer") {
        let printer = printer_val.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
        app.select_print_dialog_printer(printer);
    }
    if let Some(scale) = params.get("scale_percent").and_then(Value::as_f64) {
        app.update_print_dialog_scale(scale);
    }
    if let Some(crop_marks) = params.get("crop_marks").and_then(Value::as_bool) {
        let bleed_pt = params.get("bleed_pt").and_then(Value::as_f64);
        app.update_print_dialog_crop_marks(crop_marks, bleed_pt);
    }

    if params.get("spool").and_then(Value::as_bool).unwrap_or(false) {
        let submit = params.get("submit_to_os").and_then(Value::as_bool).unwrap_or(false);
        let _ = app.execute_print_dialog_spool(submit)?;
    }

    let p = &app.canvas_dialogs.print;
    Ok(json!({
        "dialog": "Print",
        "sheet_ids": p.job.sheet_ids,
        "copies": p.copies,
        "scale_percent": p.job.scale_percent,
        "crop_marks": p.job.crop_marks,
        "selected_printer": p.selected_printer,
        "discovered_printers": p.discovered_printers,
        "last_spool_bytes": p.last_spool_bytes,
        "last_status": p.last_status,
    }))
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

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_host_file(path: &str, bytes: &[u8]) -> Result<(), String> {
    lumencraft_platform::fs::write_atomic(std::path::Path::new(path), bytes).map_err(|err| format!("failed to write file `{path}`: {err}"))
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn write_host_file(path: &str, _bytes: &[u8]) -> Result<(), String> {
    Err(format!("writing local filesystem path `{path}` is unsupported on wasm32"))
}

pub(crate) fn parse_bounded_u32(raw: Option<&Value>, default: u32, min: u32, max: u32, field: &str) -> Result<u32, String> {
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

fn scale_scene_for_print(scene: &ChartScene, factor: f64) -> ChartScene {
    if (factor - 1.0).abs() <= f64::EPSILON {
        return scene.clone();
    }
    let scale_pt = |pt: PointPt| PointPt::new(pt.x * factor, pt.y * factor);
    let scale_rect = |r: RectPt| RectPt::new(r.x * factor, r.y * factor, r.width * factor, r.height * factor);
    let primitives = scene.primitives.iter().map(|prim| transform_primitive(prim, factor, &scale_pt, &scale_rect)).collect();
    let plotted_points = scene
        .plotted_points
        .iter()
        .map(|p| {
            let mut next = p.clone();
            next.center_pt = scale_pt(p.center_pt);
            next.radius_pt *= factor;
            next
        })
        .collect();

    ChartScene {
        sheet_id: scene.sheet_id.clone(),
        title: scene.title.clone(),
        width_pt: (scene.width_pt * factor).max(1.0),
        height_pt: (scene.height_pt * factor).max(1.0),
        background: scene.background,
        plot_rect_pt: scale_rect(scene.plot_rect_pt),
        primitives,
        warnings: scene.warnings.clone(),
        plotted_points,
    }
}

fn decorate_scene_for_print(scene: &ChartScene, margin_pt: f64, crop_marks: bool) -> ChartScene {
    let shift_pt = |pt: PointPt| PointPt::new(pt.x + margin_pt, pt.y + margin_pt);
    let shift_rect = |r: RectPt| RectPt::new(r.x + margin_pt, r.y + margin_pt, r.width, r.height);
    let mut primitives: Vec<ScenePrimitive> = scene.primitives.iter().map(|prim| transform_primitive(prim, 1.0, &shift_pt, &shift_rect)).collect();

    if crop_marks {
        push_corner_crop_marks(&mut primitives, margin_pt, scene.width_pt, scene.height_pt);
    }
    let plotted_points = scene
        .plotted_points
        .iter()
        .map(|p| {
            let mut next = p.clone();
            next.center_pt = shift_pt(p.center_pt);
            next
        })
        .collect();

    ChartScene {
        sheet_id: scene.sheet_id.clone(),
        title: scene.title.clone(),
        width_pt: scene.width_pt + 2.0 * margin_pt,
        height_pt: scene.height_pt + 2.0 * margin_pt,
        background: scene.background,
        plot_rect_pt: shift_rect(scene.plot_rect_pt),
        primitives,
        warnings: scene.warnings.clone(),
        plotted_points,
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

pub(crate) fn encode_base64(input: &[u8]) -> String {
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
        ActiveDialog::ConvertArchetype { .. } => "ConvertArchetype",
        ActiveDialog::ImportReshape => "ImportReshape",
        ActiveDialog::ExcludeWithReason { .. } => "ExcludeWithReason",
        ActiveDialog::FormatSelectedPoints { .. } => "FormatSelectedPoints",
        ActiveDialog::PowerAndSampleSize => "PowerAndSampleSize",
        ActiveDialog::Print { .. } => "Print",
        ActiveDialog::CommandPalette => "CommandPalette",
        ActiveDialog::About => "About",
    }
}

pub use crate::os_integration::{
    OsAdapterAvailability, OsAdapterStatus, build_cf_html_payload, discover_os_printers, open_os_print_preview_dialog,
    open_os_print_preview_dialog_titled, read_os_clipboard_pdf_bytes, read_os_clipboard_text,
};
#[cfg(not(target_arch = "wasm32"))]
pub use crate::os_integration::{read_os_pasteboard, submit_os_print_job, submit_os_print_pdf_bytes, write_os_clipboard_bundle, write_os_pasteboard};

#[cfg(test)]
mod tests {
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

    #[test]
    fn control_response_includes_structured_error_detail_for_engine_failures() -> Result<(), Box<dyn std::error::Error>> {
        use super::ControlResponse;
        use serde_json::json;
        let mut app = crate::UiApp::new();

        // Unknown engine command via command.run emits backward-compatible `error` + structured `error_detail`.
        let unknown = dispatch_json(
            &mut app,
            &json!({
                "id": 10,
                "method": "command.run",
                "params": { "command": "no.such.command", "params": {} }
            }),
        )?;
        assert_eq!(unknown["ok"], json!(false));
        let err_str = unknown["error"].as_str().ok_or("missing legacy error string")?;
        let detail = &unknown["error_detail"];
        assert_eq!(detail["code"], json!("not_found"));
        assert_eq!(detail["category"], json!("state"));
        assert!(detail["message"].as_str().is_some_and(|m| err_str.contains(m)));
        assert_eq!(detail["retryable"], json!(false));
        assert!(detail.get("remediation").is_some());
        assert!(detail.get("details").is_some());

        // Frozen-sheet engine failure via engine.execute includes structured remediation/details.
        let created = dispatch_json(
            &mut app,
            &json!({
                "id": 11,
                "method": "sheet.createData",
                "params": { "kind": "xy", "title": "Frozen Data", "auto_family": false }
            }),
        )?;
        let sheet_id = created["result"]["payload"]["sheet_id"].as_str().ok_or("missing sheet_id")?.to_owned();
        let froze = dispatch_json(
            &mut app,
            &json!({
                "id": 12,
                "method": "sheet.freeze",
                "params": { "sheet_id": sheet_id }
            }),
        )?;
        assert_eq!(froze["ok"], json!(true));
        assert!(froze.as_object().is_some_and(|obj| !obj.contains_key("error_detail")));

        let frozen_rename = dispatch_json(
            &mut app,
            &json!({
                "id": 13,
                "method": "engine.execute",
                "params": { "command_id": "sheet.rename", "params": { "sheet_id": sheet_id, "title": "Blocked" } }
            }),
        )?;
        assert_eq!(frozen_rename["ok"], json!(false));
        let rename_err = frozen_rename["error"].as_str().ok_or("missing frozen error string")?;
        assert!(rename_err.contains("frozen") || rename_err.contains("Frozen"));
        assert_eq!(frozen_rename["error_detail"]["code"], json!("frozen_sheet"));
        assert_eq!(frozen_rename["error_detail"]["category"], json!("state"));
        assert_eq!(frozen_rename["error_detail"]["retryable"], json!(false));
        assert!(frozen_rename["error_detail"]["message"].as_str().is_some_and(|m| rename_err.contains(m)));

        // Command batch propagates engine failure error_detail too.
        let batch_err = dispatch_json(
            &mut app,
            &json!({
                "id": 14,
                "method": "command.batch",
                "params": {
                    "atomic": true,
                    "commands": [{ "command": "sheet.rename", "params": { "sheet_id": sheet_id, "title": "Still Blocked" } }]
                }
            }),
        )?;
        assert_eq!(batch_err["ok"], json!(false));
        assert_eq!(batch_err["error_detail"]["code"], json!("frozen_sheet"));

        // Invalid control JSON line omits `error_detail` and deserializes legacy ControlResponse payloads cleanly.
        let bad_json: ControlResponse = serde_json::from_str(&app.dispatch_control_line("{not-json"))?;
        assert!(!bad_json.ok);
        assert!(bad_json.error.is_some());
        assert_eq!(bad_json.error_detail, None);
        let legacy: ControlResponse = serde_json::from_str(r#"{"id":1,"ok":false,"result":null,"error":"legacy"}"#)?;
        assert_eq!(legacy.error.as_deref(), Some("legacy"));
        assert_eq!(legacy.error_detail, None);

        Ok(())
    }
}
