//! Local JSON-lines control channel and publication clipboard/print payload builders
//! (`docs/control-protocol.md`, `docs/evidence/platforms.md`).

pub mod host_routing;

use std::fmt;
use std::io::{self, BufRead, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::net::{SocketAddr, TcpListener};
use std::panic::{AssertUnwindSafe, catch_unwind};

use curvebook_chart::{ChartScene, Path2D, PathCmd, PointPt, RectPt, ScenePrimitive, StrokePaint};
use curvebook_doc::{
    AnalysisKind, CanvasResizeMode, ErrorCode, FigureCanvasPreset, MultipleVariablesTable, MultivariateSpec, ProjectDocument, ResultCell,
    ResultSheet, SheetFolderKind, SheetId, SheetSection, TableArchetype,
};
use curvebook_engine::services::{
    DEFAULT_PREVIEW_DPI, DocumentExportFormat, ExportArtifactFormat, ExportArtifactRequest, ExportOptionsSpec, ExportTarget, RasterSizingPolicy,
    compile_visual_scene_for_sheet, export_document_artifact, inspect_document_control_json, load_validated_ttf_font, parse_csv_delimiter_opt,
    parse_dpi, parse_pdf_color_space, parse_pdf_version, parse_tiff_compression, resolve_tidy_flag,
};
use curvebook_engine::{BatchCommandStep, Caller, EngineError, Session, cmd};
use curvebook_palette::{CanvasPreviewFilter, Rgba8};
use curvebook_pdf::{PdfColorSpace, PdfExportOptions, PdfVersion, render_pdf_pages};
use curvebook_render::{MAX_CAPPED_RASTER_DIM_PX, TiffCompression};
use curvebook_svg::SvgExportOptions;
use curvebook_tools::{GridCellCoord, PhysicalUnit};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, AssumptionDiagnosticBanner, AxisTarget};
use crate::grid_view::ImportReshapeMode;
use crate::i18n::{is_rtl_locale, locale_display_name, normalize_locale};
use crate::interaction::{UiInputEvent, ViewportMetrics, dispatch_ui_event};
use crate::menus::ui_commands::UI_COMMANDS;
pub use crate::os_integration::ClipboardBundle;
use crate::theme::{UiThemeMode, UiThemeSpec};
use crate::workspace_nav::{ActiveWorkspacePane, WelcomeMode};

/// Hard bound per JSON-lines control request/response frame (16 MiB).
pub const MAX_CONTROL_LINE_BYTES: usize = 16 * 1024 * 1024;

/// Hard bound on steps accepted in a single `command.batch` control request.
pub const MAX_CONTROL_BATCH_STEPS: usize = 1_000;

/// Idle read/write timeout applied to each accepted local control TCP connection so stalled clients
/// cannot starve subsequent callers (`R1-S10-04`).
#[cfg(not(target_arch = "wasm32"))]
pub const CONTROL_CLIENT_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Number of random bytes encoded into an auto-generated control authentication token (256 bits).
pub const CONTROL_AUTH_TOKEN_BYTES: usize = 32;

/// Hard upper bound on accepted control token length in bytes.
pub const MAX_CONTROL_TOKEN_BYTES: usize = 512;

/// HTTP 403 response emitted immediately when an HTTP client hits the raw control socket.
pub const CONTROL_HTTP_FORBIDDEN_RESPONSE: &str = "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nForbidden\n";

const HTTP_REQUEST_METHODS: &[&str] = &["POST", "GET", "PUT", "DELETE", "HEAD", "OPTIONS", "PATCH", "TRACE", "CONNECT", "PRI"];

/// Per-instance bearer secret enforcing authentication on local control sockets and streams.
#[derive(Clone, PartialEq, Eq)]
pub struct ControlAuth {
    token: String,
}

impl fmt::Debug for ControlAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ControlAuth([REDACTED])")
    }
}

impl ControlAuth {
    /// Generate a fresh 256-bit cryptographically random control authentication token.
    ///
    /// # Errors
    /// Returns an I/O error if OS entropy cannot be read, or `Unsupported` on `wasm32`.
    pub fn generate() -> io::Result<Self> {
        let bytes = generate_entropy_bytes()?;
        Ok(Self { token: encode_lower_hex(&bytes) })
    }

    /// Validate and wrap a caller-supplied control authentication token.
    ///
    /// # Errors
    /// Returns a descriptive error when `token` is empty, contains whitespace or control
    /// characters, or exceeds [`MAX_CONTROL_TOKEN_BYTES`].
    pub fn from_token(token: impl Into<String>) -> Result<Self, String> {
        let candidate = token.into();
        if candidate.is_empty() {
            return Err(String::from("control token must not be empty"));
        }
        if candidate.len() > MAX_CONTROL_TOKEN_BYTES {
            return Err(format!("control token exceeds {MAX_CONTROL_TOKEN_BYTES} bytes"));
        }
        if candidate.chars().any(|ch| ch.is_whitespace() || ch.is_control()) {
            return Err(String::from("control token must not contain whitespace or control characters"));
        }
        Ok(Self { token: candidate })
    }

    /// Borrow the raw secret token string for writing to a restricted local token file or client.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.token
    }

    /// Verify `candidate` against the secret token using constant-time byte comparison.
    #[must_use]
    pub fn verify(&self, candidate: &str) -> bool {
        constant_time_eq(self.token.as_bytes(), candidate.as_bytes())
    }
}

fn constant_time_eq(expected: &[u8], actual: &[u8]) -> bool {
    let max_len = expected.len().max(actual.len());
    let mut diff: u8 = u8::from(expected.len() != actual.len());
    for idx in 0..max_len {
        let a = expected.get(idx).copied().unwrap_or(0);
        let b = actual.get(idx).copied().unwrap_or(0);
        diff |= core::hint::black_box(a ^ b);
    }
    core::hint::black_box(diff) == 0
}

fn encode_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

#[cfg(unix)]
fn generate_entropy_bytes() -> io::Result<[u8; CONTROL_AUTH_TOKEN_BYTES]> {
    use std::io::Read;

    let mut file = std::fs::File::open("/dev/urandom")?;
    let mut buf = [0u8; CONTROL_AUTH_TOKEN_BYTES];
    file.read_exact(&mut buf)?;
    Ok(buf)
}

#[cfg(all(not(unix), not(target_arch = "wasm32")))]
fn generate_entropy_bytes() -> io::Result<[u8; CONTROL_AUTH_TOKEN_BYTES]> {
    use std::collections::hash_map::RandomState;
    use std::hash::BuildHasher;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |dur| dur.as_nanos());
    let pid = u64::from(std::process::id());
    let mut buf = [0u8; CONTROL_AUTH_TOKEN_BYTES];
    for (chunk_idx, chunk) in buf.chunks_exact_mut(8).enumerate() {
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let state = RandomState::new();
        let mixed = state.hash_one((nanos, pid, seq, chunk_idx as u64, b"curvebook-control-auth"));
        chunk.copy_from_slice(&mixed.to_le_bytes());
    }
    Ok(buf)
}

#[cfg(target_arch = "wasm32")]
fn generate_entropy_bytes() -> io::Result<[u8; CONTROL_AUTH_TOKEN_BYTES]> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "OS entropy generation for TCP control auth tokens is unavailable on wasm32"))
}

fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    let n = needle.as_bytes();
    !n.is_empty() && haystack.as_bytes().windows(n.len()).any(|win| win.eq_ignore_ascii_case(n))
}

fn has_http_method_prefix(trimmed: &str) -> bool {
    let bytes = trimmed.as_bytes();
    HTTP_REQUEST_METHODS.iter().any(|method| {
        let len = method.len();
        bytes.get(..len).is_some_and(|head| head.eq_ignore_ascii_case(method.as_bytes()))
            && bytes.get(len).is_some_and(|next| *next == b' ' || *next == b'\t')
    })
}

/// Detect whether `line` looks like an HTTP request line (e.g. `POST / HTTP/1.1` from browser `fetch`/form).
#[must_use]
pub fn is_http_request_line(line: &str) -> bool {
    let trimmed = line.trim();
    if has_http_method_prefix(trimmed) {
        return true;
    }
    if trimmed.starts_with('{') {
        return false;
    }
    contains_ascii_case_insensitive(trimmed, "HTTP/1.")
        || contains_ascii_case_insensitive(trimmed, "HTTP/2.")
        || contains_ascii_case_insensitive(trimmed, "HTTP/0.")
        || contains_ascii_case_insensitive(trimmed, "HTTP/2 ")
        || trimmed.eq_ignore_ascii_case("HTTP/2")
}

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

/// Recorded execution summary for a single engine command invoked by a control request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlRecordedStep {
    pub command_id: String,
    pub params: Value,
    pub mutated: bool,
}

/// Snapshot of recorded command steps and pre-request session state passed to post-request
/// persistence hooks (e.g. WAL journaling and snapshot checkpoints).
#[derive(Debug, Clone)]
pub struct ControlPostRequestContext<'a> {
    pub method: &'a str,
    pub steps: &'a [ControlRecordedStep],
    pub prev_undo_depth: usize,
    pub prev_redo_depth: usize,
    pub prev_dirty: bool,
    pub prev_mutation_seq: u64,
    pub rolled_back: bool,
    pub request_ok: bool,
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

fn map_ui_err(res: Result<Value, String>) -> Result<Value, ControlCallError> {
    res.map_err(ControlCallError::from)
}

/// Physical-size print spool specification (`docs/evidence/platforms.md` `G10`).
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
            creator: String::from("CurveBook Print Spooler"),
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
pub fn serve_control_listener(app: &mut UiApp, listener: &TcpListener, auth: &ControlAuth) -> io::Result<()> {
    serve_control_listener_with_handler(app, listener, auth, dispatch_request)
}

/// Serve authenticated loopback JSON-lines requests from `listener` while invoking `on_post_request`
/// before each response is flushed (for example to flush WAL entries or checkpoints).
///
/// # Errors
/// Propagates listener accept errors.
#[cfg(not(target_arch = "wasm32"))]
pub fn serve_control_listener_with_hook<F>(app: &mut UiApp, listener: &TcpListener, auth: &ControlAuth, mut on_post_request: F) -> io::Result<()>
where
    F: FnMut(&UiApp, &ControlPostRequestContext<'_>) -> Result<(), String>,
{
    serve_control_listener_with_handler(app, listener, auth, |app_ref, req| dispatch_request_with_hook(app_ref, req, &mut on_post_request))
}

#[cfg(not(target_arch = "wasm32"))]
const MAX_HEADLESS_CONTROL_CLIENTS: usize = 32;
#[cfg(not(target_arch = "wasm32"))]
const MAX_PRE_AUTH_HEADLESS_LINE_BYTES: usize = 64 * 1024;

#[cfg(not(target_arch = "wasm32"))]
struct HeadlessClientRequest {
    line: String,
    conn_authenticated: bool,
    reply_tx: std::sync::mpsc::Sender<HeadlessClientReply>,
}

#[cfg(not(target_arch = "wasm32"))]
struct HeadlessClientReply {
    response_json: String,
    conn_authenticated: bool,
    quit: bool,
}

#[cfg(not(target_arch = "wasm32"))]
struct HeadlessActiveGuard(std::sync::Arc<std::sync::atomic::AtomicUsize>);

#[cfg(not(target_arch = "wasm32"))]
impl Drop for HeadlessActiveGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct ListenerBlockingGuard<'a>(&'a TcpListener);

#[cfg(not(target_arch = "wasm32"))]
impl Drop for ListenerBlockingGuard<'_> {
    fn drop(&mut self) {
        let _ = self.0.set_nonblocking(false);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn serve_control_listener_with_handler<F>(app: &mut UiApp, listener: &TcpListener, auth: &ControlAuth, mut handler: F) -> io::Result<()>
where
    F: FnMut(&mut UiApp, ControlRequest) -> (ControlResponse, ControlLoopAction),
{
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    listener.set_nonblocking(true)?;
    let _restore_blocking = ListenerBlockingGuard(listener);
    let shutdown = Arc::new(AtomicBool::new(false));
    let active_clients = Arc::new(AtomicUsize::new(0));
    let (req_tx, req_rx) = mpsc::channel::<HeadlessClientRequest>();

    std::thread::scope(|scope| {
        let accept_shutdown = Arc::clone(&shutdown);
        let accept_active = Arc::clone(&active_clients);
        scope.spawn(move || run_headless_accept_loop(scope, listener, req_tx, accept_shutdown, accept_active));

        while !shutdown.load(Ordering::Acquire) {
            match req_rx.recv_timeout(Duration::from_millis(25)) {
                Ok(req) => {
                    let mut conn_auth = req.conn_authenticated;
                    let (response_json, action) = handle_authenticated_control_line(app, &req.line, auth, &mut conn_auth, &mut handler);
                    let quit = action == ControlLoopAction::Quit;
                    let _ = req.reply_tx.send(HeadlessClientReply { response_json, conn_authenticated: conn_auth, quit });
                    if quit {
                        shutdown.store(true, Ordering::Release);
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        drop(req_rx);
    });
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn run_headless_accept_loop<'scope>(
    scope: &'scope std::thread::Scope<'scope, '_>,
    listener: &'scope TcpListener,
    req_tx: std::sync::mpsc::Sender<HeadlessClientRequest>,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
    active_clients: std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    let mut last_err_log: Option<Instant> = None;
    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                let reserved =
                    active_clients.try_update(Ordering::AcqRel, Ordering::Acquire, |c| (c < MAX_HEADLESS_CONTROL_CLIENTS).then_some(c + 1)).is_ok();
                if reserved {
                    let guard = HeadlessActiveGuard(std::sync::Arc::clone(&active_clients));
                    let client_tx = req_tx.clone();
                    let client_shutdown = std::sync::Arc::clone(&shutdown);
                    scope.spawn(move || {
                        let _guard = guard;
                        if let Err(err) = serve_headless_tcp_client(stream, &client_tx, &client_shutdown) {
                            eprintln!("control connection closed after error: {err}");
                        }
                    });
                } else {
                    reject_busy_headless_tcp_client(stream);
                }
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::ConnectionAborted | io::ErrorKind::ConnectionReset | io::ErrorKind::TimedOut
                ) => {}
            Err(err) => {
                if !shutdown.load(Ordering::Acquire) {
                    if last_err_log.is_none_or(|prev| prev.elapsed() >= Duration::from_secs(1)) {
                        eprintln!("control listener transient accept error: {err}");
                        last_err_log = Some(Instant::now());
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn reject_busy_headless_tcp_client(mut stream: std::net::TcpStream) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(50)));
    let resp = ControlResponse::err(
        Value::String(String::new()),
        format!("control server busy: maximum concurrent clients ({MAX_HEADLESS_CONTROL_CLIENTS}) reached"),
    );
    let _ = stream.write_all(serialize_response(&resp).as_bytes());
    let _ = stream.write_all(b"\n");
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

#[cfg(not(target_arch = "wasm32"))]
fn serve_headless_tcp_client(
    mut stream: std::net::TcpStream,
    req_tx: &std::sync::mpsc::Sender<HeadlessClientRequest>,
    shutdown: &std::sync::atomic::AtomicBool,
) -> io::Result<()> {
    use std::sync::mpsc;
    use std::time::Duration;

    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_millis(100)))?;
    stream.set_write_timeout(Some(CONTROL_CLIENT_IDLE_TIMEOUT))?;
    let reader_stream = stream.try_clone()?;
    let mut reader = io::BufReader::new(reader_stream);
    let mut conn_authenticated = false;

    loop {
        let line_cap = if conn_authenticated { MAX_CONTROL_LINE_BYTES } else { MAX_PRE_AUTH_HEADLESS_LINE_BYTES };
        let raw_bytes = match read_headless_tcp_line(&mut reader, line_cap, shutdown)? {
            HeadlessLineRead::Line(bytes) => bytes,
            HeadlessLineRead::Done => return Ok(()),
            HeadlessLineRead::Oversized => {
                let resp = ControlResponse::err(Value::String(String::new()), format!("control line exceeds {line_cap} bytes"));
                let _ = stream.write_all(serialize_response(&resp).as_bytes());
                let _ = stream.write_all(b"\n");
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Write);
                drain_headless_reader_briefly(&mut reader);
                return Ok(());
            }
        };
        let line = match std::str::from_utf8(&raw_bytes) {
            Ok(text) => text,
            Err(err) => {
                let resp = ControlResponse::err(Value::String(String::new()), format!("control line is not valid UTF-8: {err}"));
                stream.write_all(serialize_response(&resp).as_bytes())?;
                stream.write_all(b"\n")?;
                stream.flush()?;
                continue;
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if is_http_request_line(trimmed) {
            stream.write_all(CONTROL_HTTP_FORBIDDEN_RESPONSE.as_bytes())?;
            stream.flush()?;
            return Ok(());
        }
        let (reply_tx, reply_rx) = mpsc::channel();
        let msg = HeadlessClientRequest { line: trimmed.to_owned(), conn_authenticated, reply_tx };
        if req_tx.send(msg).is_err() {
            return Ok(());
        }
        let Ok(reply) = reply_rx.recv() else {
            return Ok(());
        };
        conn_authenticated = reply.conn_authenticated;
        if reply.quit {
            let _ = stream.write_all(reply.response_json.as_bytes());
            let _ = stream.write_all(b"\n");
            let _ = stream.flush();
            return Ok(());
        }
        stream.write_all(reply.response_json.as_bytes())?;
        stream.write_all(b"\n")?;
        stream.flush()?;
    }
}

#[cfg(not(target_arch = "wasm32"))]
enum HeadlessLineRead {
    Line(Vec<u8>),
    Oversized,
    Done,
}

#[cfg(not(target_arch = "wasm32"))]
fn read_headless_tcp_line<R: BufRead>(reader: &mut R, max_bytes: usize, shutdown: &std::sync::atomic::AtomicBool) -> io::Result<HeadlessLineRead> {
    use std::sync::atomic::Ordering;
    use std::time::Instant;

    let mut line = Vec::new();
    let idle_start = Instant::now();
    let mut line_start: Option<Instant> = None;
    loop {
        if shutdown.load(Ordering::Acquire) {
            return Ok(HeadlessLineRead::Done);
        }
        let expired = line_start.map_or_else(|| idle_start.elapsed() >= CONTROL_CLIENT_IDLE_TIMEOUT, |s| s.elapsed() >= CONTROL_CLIENT_IDLE_TIMEOUT);
        if expired {
            return Ok(HeadlessLineRead::Done);
        }
        match reader.fill_buf() {
            Ok([]) => return Ok(if line.is_empty() { HeadlessLineRead::Done } else { HeadlessLineRead::Line(line) }),
            Ok(avail) => {
                let started = *line_start.get_or_insert_with(Instant::now);
                if let Some(nl_pos) = avail.iter().position(|&b| b == b'\n') {
                    let take = nl_pos + 1;
                    if line.len().saturating_add(take) > max_bytes {
                        return Ok(HeadlessLineRead::Oversized);
                    }
                    line.extend_from_slice(&avail[..take]);
                    reader.consume(take);
                    return Ok(HeadlessLineRead::Line(line));
                }
                if line.len().saturating_add(avail.len()) > max_bytes {
                    return Ok(HeadlessLineRead::Oversized);
                }
                line.extend_from_slice(avail);
                let consumed = avail.len();
                reader.consume(consumed);
                if started.elapsed() >= CONTROL_CLIENT_IDLE_TIMEOUT {
                    return Ok(HeadlessLineRead::Done);
                }
            }
            Err(err) if matches!(err.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => {}
            Err(err) => return Err(err),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn drain_headless_reader_briefly<R: BufRead>(reader: &mut R) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
    let mut drained = 0usize;
    while drained <= MAX_CONTROL_LINE_BYTES && std::time::Instant::now() < deadline {
        match reader.fill_buf() {
            Ok([]) | Err(_) => break,
            Ok(avail) => {
                if let Some(nl_pos) = avail.iter().position(|&b| b == b'\n') {
                    reader.consume(nl_pos + 1);
                    break;
                }
                let n = avail.len();
                drained = drained.saturating_add(n);
                reader.consume(n);
            }
        }
    }
}

/// Bind `addr` (loopback-only) and run the authenticated JSON-lines TCP control server until `app.quit`.
///
/// # Errors
/// Returns an error if `addr` is non-loopback or socket bind/accept fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_control_tcp_server(app: &mut UiApp, addr: SocketAddr, auth: &ControlAuth) -> Result<(), String> {
    let checked = validate_loopback_addr(addr)?;
    let listener = bind_control_tcp_listener(checked).map_err(|err| format!("failed to bind control socket {checked}: {err}"))?;
    serve_control_listener(app, &listener, auth).map_err(|err| format!("control server I/O error: {err}"))
}

/// Serve authenticated JSON-lines control requests from `reader` and emit serialized responses to `writer`,
/// stopping cleanly on EOF, HTTP `403` rejection, or `app.quit`.
///
/// # Errors
/// Returns an I/O error if reading, exceeding [`MAX_CONTROL_LINE_BYTES`], or writing fails.
pub fn serve_control_stream<R: BufRead, W: Write>(app: &mut UiApp, reader: &mut R, writer: &mut W, auth: &ControlAuth) -> io::Result<()> {
    let _ = serve_control_stream_with_quit(app, reader, writer, auth)?;
    Ok(())
}

/// Serve authenticated JSON-lines requests over `reader`/`writer` and return `Ok(true)` when `app.quit` stopped the stream.
///
/// # Errors
/// Returns an I/O error on underlying stream failure or oversized lines exceeding [`MAX_CONTROL_LINE_BYTES`].
pub fn serve_control_stream_with_quit<R: BufRead, W: Write>(app: &mut UiApp, reader: &mut R, writer: &mut W, auth: &ControlAuth) -> io::Result<bool> {
    serve_control_stream_with_handler(app, reader, writer, auth, dispatch_request)
}

/// Serve authenticated JSON-lines requests over `reader`/`writer` while invoking `on_post_request`
/// before emitting each response, returning `Ok(true)` when `app.quit` stopped the stream.
///
/// # Errors
/// Returns an I/O error on underlying stream failure or oversized lines exceeding [`MAX_CONTROL_LINE_BYTES`].
pub fn serve_control_stream_with_quit_and_hook<R: BufRead, W: Write, F>(
    app: &mut UiApp,
    reader: &mut R,
    writer: &mut W,
    auth: &ControlAuth,
    mut on_post_request: F,
) -> io::Result<bool>
where
    F: FnMut(&UiApp, &ControlPostRequestContext<'_>) -> Result<(), String>,
{
    serve_control_stream_with_handler(app, reader, writer, auth, |app_ref, req| dispatch_request_with_hook(app_ref, req, &mut on_post_request))
}

/// Convenience wrapper around [`serve_control_stream_with_quit`] that validates a raw token string.
///
/// # Errors
/// Returns `InvalidInput` if `token` is invalid, or any underlying stream I/O error.
pub fn serve_control_stream_with_quit_and_token<R: BufRead, W: Write>(
    app: &mut UiApp,
    reader: &mut R,
    writer: &mut W,
    token: &str,
) -> io::Result<bool> {
    let auth = ControlAuth::from_token(token).map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    serve_control_stream_with_quit(app, reader, writer, &auth)
}

fn serve_control_stream_with_handler<R, W, F>(app: &mut UiApp, reader: &mut R, writer: &mut W, auth: &ControlAuth, handler: F) -> io::Result<bool>
where
    R: BufRead,
    W: Write,
    F: FnMut(&mut UiApp, ControlRequest) -> (ControlResponse, ControlLoopAction),
{
    match serve_control_stream_loop(app, reader, writer, auth, handler) {
        Ok(quit) => Ok(quit),
        // Drop only the stalled/idle client connection so subsequent clients can connect promptly.
        Err(err) if matches!(err.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => Ok(false),
        Err(err) => Err(err),
    }
}

fn serve_control_stream_loop<R, W, F>(app: &mut UiApp, reader: &mut R, writer: &mut W, auth: &ControlAuth, mut handler: F) -> io::Result<bool>
where
    R: BufRead,
    W: Write,
    F: FnMut(&mut UiApp, ControlRequest) -> (ControlResponse, ControlLoopAction),
{
    let mut connection_authenticated = false;
    loop {
        let raw_bytes = match read_bounded_control_line(reader, MAX_CONTROL_LINE_BYTES) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return Ok(false),
            Err(err) => {
                if err.kind() == io::ErrorKind::InvalidData {
                    let resp = ControlResponse::err(Value::String(String::new()), err.to_string());
                    let _ = writer.write_all(serialize_response(&resp).as_bytes());
                    let _ = writer.write_all(b"\n");
                    let _ = writer.flush();
                }
                return Err(err);
            }
        };
        let line = match std::str::from_utf8(&raw_bytes) {
            Ok(line) => line,
            Err(err) => {
                let resp = ControlResponse::err(Value::String(String::new()), format!("control line is not valid UTF-8: {err}"));
                writer.write_all(serialize_response(&resp).as_bytes())?;
                writer.write_all(b"\n")?;
                writer.flush()?;
                continue;
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if is_http_request_line(trimmed) {
            writer.write_all(CONTROL_HTTP_FORBIDDEN_RESPONSE.as_bytes())?;
            writer.flush()?;
            return Ok(false);
        }
        let (response_json, action) = handle_authenticated_control_line(app, trimmed, auth, &mut connection_authenticated, &mut handler);
        writer.write_all(response_json.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        if action == ControlLoopAction::Quit {
            return Ok(true);
        }
    }
}

fn handle_authenticated_control_line<F>(
    app: &mut UiApp,
    line: &str,
    auth: &ControlAuth,
    connection_authenticated: &mut bool,
    handler: &mut F,
) -> (String, ControlLoopAction)
where
    F: FnMut(&mut UiApp, ControlRequest) -> (ControlResponse, ControlLoopAction),
{
    let best_effort_id = extract_best_effort_request_id(line);
    let guarded = catch_unwind(AssertUnwindSafe(|| process_authenticated_line(app, line, auth, connection_authenticated, handler)));
    match guarded {
        Ok(output) => output,
        Err(payload) => (serialize_response(&panic_control_response(best_effort_id, &*payload)), ControlLoopAction::Continue),
    }
}

fn process_authenticated_line<F>(
    app: &mut UiApp,
    line: &str,
    auth: &ControlAuth,
    connection_authenticated: &mut bool,
    handler: &mut F,
) -> (String, ControlLoopAction)
where
    F: FnMut(&mut UiApp, ControlRequest) -> (ControlResponse, ControlLoopAction),
{
    let raw: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(err) => {
            let resp = if *connection_authenticated {
                ControlResponse::err(Value::String(String::new()), format!("invalid control JSON: {err}"))
            } else {
                unauthorized_control_response(Value::String(String::new()), "authentication required")
            };
            return (serialize_response(&resp), ControlLoopAction::Continue);
        }
    };
    let Some(obj) = raw.as_object() else {
        let resp = if *connection_authenticated {
            ControlResponse::err(Value::String(String::new()), "control request must be a JSON object")
        } else {
            unauthorized_control_response(Value::String(String::new()), "authentication required")
        };
        return (serialize_response(&resp), ControlLoopAction::Continue);
    };

    let id = obj.get("id").cloned().unwrap_or_else(|| Value::String(String::new()));
    let raw_method = obj.get("method").or_else(|| obj.get("op")).and_then(Value::as_str).map(str::trim);

    if raw_method == Some("auth") {
        let valid = extract_auth_handshake_token(obj).is_some_and(|tok| auth.verify(tok));
        *connection_authenticated = valid;
        let resp = if valid {
            ControlResponse::ok(id, json!({ "authenticated": true }))
        } else {
            unauthorized_control_response(id, "invalid or missing control authentication token")
        };
        return (serialize_response(&resp), ControlLoopAction::Continue);
    }

    if let Err(reason) = authorize_request_envelope(obj, auth, *connection_authenticated) {
        return (serialize_response(&unauthorized_control_response(id, reason)), ControlLoopAction::Continue);
    }

    let Some(method) = raw_method.filter(|s| !s.is_empty()) else {
        let resp = ControlResponse::err(id, "missing non-empty 'method' or 'op' field");
        return (serialize_response(&resp), ControlLoopAction::Continue);
    };

    let params = host_routing::normalize_control_request_params(method, obj);
    let req = ControlRequest { id, method: method.to_owned(), params };
    let (response, action) = handler(app, req);
    (serialize_response(&response), action)
}

fn extract_auth_handshake_token(obj: &Map<String, Value>) -> Option<&str> {
    let top = obj.get("token");
    let nested = obj.get("params").and_then(Value::as_object).and_then(|params| params.get("token"));
    match (top, nested) {
        (Some(a), Some(b)) => {
            let sa = a.as_str()?;
            let sb = b.as_str()?;
            if sa == sb { Some(sa) } else { None }
        }
        (Some(val), None) | (None, Some(val)) => val.as_str(),
        (None, None) => None,
    }
}

fn authorize_request_envelope(obj: &Map<String, Value>, auth: &ControlAuth, connection_authenticated: bool) -> Result<(), &'static str> {
    if let Some(token_val) = obj.get("token") {
        let matched = token_val.as_str().is_some_and(|candidate| auth.verify(candidate));
        if matched {
            return Ok(());
        }
        return Err("invalid control authentication token");
    }
    if connection_authenticated {
        return Ok(());
    }
    Err("authentication required")
}

fn unauthorized_control_response(id: Value, detail: &str) -> ControlResponse {
    let message = format!("unauthorized: {detail}");
    ControlResponse {
        id,
        ok: false,
        result: Value::Null,
        error: Some(message.clone()),
        error_detail: Some(json!({
            "code": "unauthorized",
            "category": "policy",
            "message": message,
            "retryable": false,
            "remediation": "Authenticate with `{\"method\":\"auth\",\"params\":{\"token\":\"<token>\"}}` or include top-level `\"token\"`.",
        })),
    }
}

fn extract_panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(msg) = payload.downcast_ref::<&str>() {
        if msg.trim().is_empty() { String::from("empty panic payload") } else { (*msg).to_owned() }
    } else if let Some(msg) = payload.downcast_ref::<String>() {
        if msg.trim().is_empty() { String::from("empty panic payload") } else { msg.clone() }
    } else {
        String::from("unknown panic payload")
    }
}

fn panic_control_response(id: Value, payload: &(dyn std::any::Any + Send)) -> ControlResponse {
    let panic_msg = extract_panic_message(payload);
    let err = EngineError::with_code(ErrorCode::InternalPanic, format!("internal control panic: {panic_msg}"))
        .with_remediation("Inspect application state before retrying the control command.");
    ControlResponse::err_with_detail(id, err.into())
}

fn extract_best_effort_request_id(line: &str) -> Value {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|val| val.as_object().and_then(|obj| obj.get("id")).cloned())
        .unwrap_or_else(|| Value::String(String::new()))
}

/// Handle a single in-process JSON-lines request against `app` and return serialized JSON response text.
///
/// Always emits a structured JSON [`ControlResponse`] even on parse, validation, or handler-panic failures.
#[must_use]
pub fn handle_control_line(app: &mut UiApp, line: &str) -> String {
    handle_control_line_with_action(app, line).0
}

/// Handle a single in-process JSON-lines request against `app` and return `(response_json, loop_action)`.
#[must_use]
pub fn handle_control_line_with_action(app: &mut UiApp, line: &str) -> (String, ControlLoopAction) {
    let best_effort_id = extract_best_effort_request_id(line);
    let guarded = catch_unwind(AssertUnwindSafe(|| {
        let req = match parse_control_request(line) {
            Ok(parsed) => parsed,
            Err((id, message)) => {
                let resp = ControlResponse::err(id, message);
                return (serialize_response(&resp), ControlLoopAction::Continue);
            }
        };

        let (response, action) = dispatch_request(app, req);
        (serialize_response(&response), action)
    }));
    match guarded {
        Ok(output) => output,
        Err(payload) => (serialize_response(&panic_control_response(best_effort_id, &*payload)), ControlLoopAction::Continue),
    }
}

/// Dispatch one front-end UI command from [`UI_COMMANDS`].
///
/// Returns `Ok(None)` when `command_id` is not a front-end UI command so callers can delegate
/// to engine command execution cleanly.
///
/// Returns `true` when `method` is one of the 8 interactive GUI action entry points allowed
/// under strict `--gui-only`.
#[must_use]
pub(crate) fn is_gui_action_method(method: &str) -> bool {
    matches!(
        method,
        "ui.inspect"
            | "ui.screenshot"
            | "ui.pointer_gesture"
            | "ui.type_text"
            | "ui.press_key"
            | "ui.invoke_menu"
            | "ui.switch_tab"
            | "ui.answer_dialog"
    )
}

/// Dispatch one of the 8 interactive GUI actions (`ui.inspect`, `ui.screenshot`,
/// `ui.pointer_gesture`, `ui.type_text`, `ui.press_key`, `ui.invoke_menu`, `ui.switch_tab`,
/// `ui.answer_dialog`) against `app` for in-process callers (`curvebook-cli` MCP bridge, `R8.7`).
///
/// All workspace mutations invoked via these GUI interactions run as `Caller::Ui`.
///
/// # Errors
/// Returns a structured [`EngineError`] on invalid parameters, disabled UI controls/tabs/menus
/// (`ErrorCode::Conflict`), missing widgets (`ErrorCode::NotFound`), or blocked disk writes under
/// strict `--gui-only` (`ErrorCode::GuiOnlyViolation`).
pub fn dispatch_gui_action(app: &mut UiApp, method: &str, params: Value) -> Result<Value, EngineError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::egui_shell::inspect_bridge::dispatch_gui_action(app, method, params)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (app, params);
        Err(EngineError::with_code(ErrorCode::Conflict, format!("GUI action `{method}` requires desktop egui_shell")))
    }
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

pub(crate) fn require_params_object_or_null(params: &Value, context: &str) -> Result<(), String> {
    if params.is_null() || params.is_object() { Ok(()) } else { Err(format!("{context} expects a JSON object `params`")) }
}

fn opt_field_entry<'a, 'b>(params: &'a Value, fields: &[&'b str]) -> Option<(&'b str, &'a Value)> {
    fields.iter().find_map(|&key| params.get(key).map(|val| (key, val)))
}

pub(crate) fn opt_bool_param(params: &Value, fields: &[&str]) -> Result<Option<bool>, String> {
    match opt_field_entry(params, fields) {
        None | Some((_, Value::Null)) => Ok(None),
        Some((_, Value::Bool(b))) => Ok(Some(*b)),
        Some((field, _)) => Err(format!("`{field}` must be a boolean")),
    }
}

pub(crate) fn opt_usize_param(params: &Value, fields: &[&str]) -> Result<Option<usize>, String> {
    match opt_field_entry(params, fields) {
        None | Some((_, Value::Null)) => Ok(None),
        Some((field, val)) => {
            let n = val.as_u64().ok_or_else(|| format!("`{field}` must be a non-negative integer"))?;
            usize::try_from(n).map(Some).map_err(|_| format!("`{field}` exceeds supported index range"))
        }
    }
}

pub(crate) fn opt_finite_f64_param(params: &Value, fields: &[&str]) -> Result<Option<f64>, String> {
    match opt_field_entry(params, fields) {
        None | Some((_, Value::Null)) => Ok(None),
        Some((field, val)) => {
            let n = val.as_f64().filter(|x| x.is_finite()).ok_or_else(|| format!("`{field}` must be a finite number"))?;
            Ok(Some(n))
        }
    }
}

pub(crate) fn opt_str_param<'a>(params: &'a Value, fields: &[&str]) -> Result<Option<&'a str>, String> {
    match opt_field_entry(params, fields) {
        None | Some((_, Value::Null)) => Ok(None),
        Some((field, val)) => val.as_str().map(Some).ok_or_else(|| format!("`{field}` must be a string")),
    }
}

pub(crate) fn opt_non_empty_str_param<'a>(params: &'a Value, fields: &[&str]) -> Result<Option<&'a str>, String> {
    match opt_str_param(params, fields)? {
        None => Ok(None),
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                let field = opt_field_entry(params, fields).map_or("param", |(name, _)| name);
                Err(format!("`{field}` must be a non-empty string"))
            } else {
                Ok(Some(trimmed))
            }
        }
    }
}

pub(crate) fn parse_selected_points_with_subcols(val: &Value, field: &str) -> Result<Vec<(usize, usize, Option<usize>)>, String> {
    let arr = val.as_array().ok_or_else(|| format!("`{field}` must be an array of `{{series_index, point_index}}` objects"))?;
    let mut pts = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        let Some(obj) = item.as_object() else {
            return Err(format!("`{field}[{idx}]` must be an object with `series_index` and `point_index`"));
        };
        let s_u64 =
            obj.get("series_index").and_then(Value::as_u64).ok_or_else(|| format!("`{field}[{idx}].series_index` must be a non-negative integer"))?;
        let p_u64 =
            obj.get("point_index").and_then(Value::as_u64).ok_or_else(|| format!("`{field}[{idx}].point_index` must be a non-negative integer"))?;
        let s = usize::try_from(s_u64).map_err(|_| format!("`{field}[{idx}].series_index` exceeds usize"))?;
        let p = usize::try_from(p_u64).map_err(|_| format!("`{field}[{idx}].point_index` exceeds usize"))?;
        let subcol = match obj.get("subcol") {
            None | Some(Value::Null) => None,
            Some(v) => {
                let sc_u64 = v.as_u64().ok_or_else(|| format!("`{field}[{idx}].subcol` must be a non-negative integer or null"))?;
                Some(usize::try_from(sc_u64).map_err(|_| format!("`{field}[{idx}].subcol` exceeds usize"))?)
            }
        };
        pts.push((s, p, subcol));
    }
    Ok(pts)
}

type UiCommandHandler = fn(&mut UiApp, &Value) -> Result<Value, ControlCallError>;

fn lookup_ui_command_handler(command_id: &str) -> Option<UiCommandHandler> {
    match command_id {
        "ui.navigator.toggle" => Some(|app, _| Ok(json!({ "navigator_open": app.toggle_navigator_pane() }))),
        "ui.inspector.toggle" => Some(|app, _| Ok(json!({ "inspector_open": app.toggle_inspector_pane() }))),
        "ui.family.toggle" => Some(|app, _| Ok(json!({ "family_open": app.toggle_family_pane() }))),
        "ui.palette.open" => Some(|app, _| {
            app.canvas_dialogs.open_command_palette();
            app.canvas_dialogs.command_palette.sync_shortcuts(&app.shortcuts);
            Ok(json!({
                "palette_open": matches!(app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette),
                "dialog": "CommandPalette",
            }))
        }),
        "ui.sheet.find" => Some(|app, p| map_ui_err(dispatch_ui_sheet_find(app, p))),
        "ui.analyze.open" => Some(|app, p| map_ui_err(dispatch_ui_analyze_open(app, p))),
        "ui.editParams.open" => Some(|app, p| map_ui_err(dispatch_ui_edit_params_open(app, p))),
        "ui.powerWizard.open" => Some(|app, p| map_ui_err(dispatch_ui_power_wizard_open(app, p))),
        "ui.formatPoints.open" => Some(|app, p| map_ui_err(dispatch_ui_format_points_open(app, p))),
        "ui.clonePipeline.open" => Some(|app, p| map_ui_err(dispatch_ui_clone_pipeline_open(app, p))),
        "ui.zoom.fitSheet" => Some(|app, _| {
            app.canvas_dialogs.reset_zoom_to_fit();
            app.egui_views.canvas_view.fit_zoom_active = true;
            app.egui_views.canvas_view.pan_pt = [0.0, 0.0];
            app.egui_views.center_stage.secondary_canvas_view.fit_zoom_active = true;
            app.egui_views.center_stage.secondary_canvas_view.pan_pt = [0.0, 0.0];
            Ok(json!({ "zoom": app.canvas_dialogs.canvas.zoom, "fit": true }))
        }),
        "ui.gallery.toggle" => Some(|app, p| map_ui_err(dispatch_ui_gallery_toggle(app, p))),
        "ui.pane.cycle" => Some(|app, p| map_ui_err(dispatch_ui_pane_cycle(app, p))),
        "ui.rulers.toggle" => Some(|app, p| map_ui_err(dispatch_ui_rulers_toggle(app, p))),
        "ui.shortcut.set" => Some(|app, p| map_ui_err(dispatch_ui_shortcut_set(app, p))),
        "ui.shortcut.reset" => Some(|app, p| map_ui_err(dispatch_ui_shortcut_reset(app, p))),
        "ui.locale.set" => Some(|app, p| map_ui_err(dispatch_ui_locale_set(app, p))),
        "ui.theme.set" => Some(|app, p| map_ui_err(dispatch_ui_theme_set(app, p))),
        "ui.screenshot" => Some(|app, p| dispatch_gui_action(app, "ui.screenshot", p.clone()).map_err(ControlCallError::from)),
        "ui.event.dispatch" => Some(|app, p| map_ui_err(dispatch_ui_event_cmd(app, p))),
        "ui.split.toggle" => Some(|app, p| map_ui_err(dispatch_ui_split_toggle(app, p))),
        "ui.brush.point" => Some(|app, p| map_ui_err(dispatch_ui_brush_point(app, p))),
        "ui.result.copyMethodsReport" => Some(dispatch_ui_result_copy_methods_report),
        "ui.result.applyRemediation" => Some(|app, p| dispatch_ui_result_apply_remediation(app, p)),
        "ui.result.inspectDiagnostics" => Some(|app, p| map_ui_err(dispatch_ui_result_inspect_diagnostics(app, p))),
        "ui.canvas.applyPreset" => Some(dispatch_ui_canvas_apply_preset),
        "ui.canvas.preflight" => Some(dispatch_ui_canvas_preflight),
        "ui.previewFilter.set" => Some(|app, p| map_ui_err(dispatch_ui_preview_filter_set(app, p))),
        "ui.workspacePreset.set" => Some(|app, p| map_ui_err(dispatch_ui_workspace_preset_set(app, p))),
        "ui.grid.excludeWithReason" => Some(dispatch_ui_grid_exclude_with_reason),
        "ui.table.randomizeGroups" => Some(|app, p| map_ui_err(dispatch_ui_randomize_groups_open(app, p))),
        "ui.table.randomSampleRows" => Some(|app, p| map_ui_err(dispatch_ui_random_sample_rows_open(app, p))),
        "ui.table.simulateDistributions" => Some(|app, p| map_ui_err(dispatch_ui_simulate_distributions_open(app, p))),
        "ui.table.openFormulaBar" => Some(|app, p| map_ui_err(dispatch_ui_open_formula_bar(app, p))),
        "ui.result.appendClusterColumn" => Some(dispatch_ui_append_cluster_column),
        "ui.graph.insertModelEquation" => Some(|app, p| map_ui_err(dispatch_ui_insert_model_equation_open(app, p))),
        "ui.graph.axes.open" => Some(|app, p| map_ui_err(dispatch_ui_format_axes_open(app, p))),
        "ui.applyStyleFrom.open" => Some(|app, p| map_ui_err(dispatch_ui_apply_style_from_open(app, p))),
        "ui.export.figure.open" => Some(|app, p| map_ui_err(dispatch_ui_export_figure_open(app, p))),
        "ui.export.tableCsv.open" => Some(|app, p| map_ui_err(dispatch_ui_csv_export_open(app, p, CsvExportOpenMode::DataTable))),
        "ui.export.resultsCsv.open" => Some(|app, p| map_ui_err(dispatch_ui_csv_export_open(app, p, CsvExportOpenMode::ResultSheet))),
        "ui.export.allTablesCsv.open" => Some(|app, p| map_ui_err(dispatch_ui_csv_export_open(app, p, CsvExportOpenMode::AllDataTables))),
        "ui.export.allTablesCsv" => Some(|app, p| map_ui_err(dispatch_ui_export_all_tables_csv_fallback(app, p))),
        "ui.import.warnings.open" => Some(|app, p| map_ui_err(dispatch_ui_import_warnings_open(app, p))),
        "ui.import.reshape.open" => Some(|app, p| map_ui_err(dispatch_ui_import_reshape_open(app, p))),
        "ui.print.open" | "file.print" => Some(|app, p| map_ui_err(dispatch_ui_print_open(app, p))),
        "app.about" => Some(|app, _| {
            app.canvas_dialogs.active_dialog = ActiveDialog::About;
            Ok(json!({ "dialog": "About", "version": env!("CARGO_PKG_VERSION") }))
        }),
        "app.feedback" => Some(dispatch_ui_app_feedback),
        "ui.view.rowTitles.toggle" => Some(|app, p| map_ui_err(dispatch_ui_row_titles_toggle(app, p))),
        "ui.graph.create.open" => Some(|app, p| map_ui_err(dispatch_ui_create_graph_open(app, p))),
        "ui.gridOverlay.toggle" => Some(|app, p| map_ui_err(dispatch_ui_grid_overlay_toggle(app, p))),
        "ui.embeddedTable.format.open" => Some(|app, p| map_ui_err(dispatch_ui_embedded_object_format_open(app, p, false))),
        "ui.image.format.open" => Some(|app, p| map_ui_err(dispatch_ui_embedded_object_format_open(app, p, true))),
        "ui.clipboard.copyForSlides" => Some(dispatch_ui_copy_for_slides),
        _ => None,
    }
}

fn dispatch_ui_app_feedback(app: &mut UiApp, params: &Value) -> Result<Value, ControlCallError> {
    if let Some(raw_cat) = params.get("category").and_then(Value::as_str) {
        let Some(parsed) = crate::feedback::FeedbackCategory::parse(raw_cat) else {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                format!("unknown feedback category `{raw_cat}` (expected `bug`, `feature`, or `numerical`)"),
            )
            .into());
        };
        app.canvas_dialogs.feedback.category = parsed;
    }
    app.canvas_dialogs.feedback.status_message = None;
    app.canvas_dialogs.feedback.copied_markdown = false;
    app.canvas_dialogs.active_dialog = ActiveDialog::Feedback;
    let preview = app.build_feedback_issue_url();
    Ok(json!({
        "dialog": "Feedback",
        "category": app.canvas_dialogs.feedback.category.slug(),
        "template": app.canvas_dialogs.feedback.category.template_filename(),
        "preview_url": preview.url,
        "truncated": preview.truncated,
    }))
}

fn dispatch_ui_command_detailed(app: &mut UiApp, command_id: &str, params: &Value) -> Result<Option<Value>, ControlCallError> {
    if command_id == "analysis.updateParams" && (params.is_null() || params.as_object().is_some_and(Map::is_empty)) {
        return map_ui_err(dispatch_ui_edit_params_open(app, params)).map(Some);
    }
    if let Some(spec) = lookup_patched_sheet_dialog(command_id) {
        require_params_object_or_null(params, command_id).map_err(ControlCallError::from)?;
        return map_ui_err(open_patched_sheet_dialog_spec(app, params, spec)).map(Some);
    }
    let Some(handler) = lookup_ui_command_handler(command_id) else {
        return Ok(None);
    };
    require_params_object_or_null(params, command_id).map_err(ControlCallError::from)?;
    handler(app, params).map(Some)
}

fn dispatch_request(app: &mut UiApp, req: ControlRequest) -> (ControlResponse, ControlLoopAction) {
    dispatch_request_inner(app, req, None, None)
}

fn dispatch_request_with_hook<F>(app: &mut UiApp, req: ControlRequest, on_post_request: &mut F) -> (ControlResponse, ControlLoopAction)
where
    F: FnMut(&UiApp, &ControlPostRequestContext<'_>) -> Result<(), String>,
{
    let prev_undo_depth = app.session().undo_depth();
    let prev_redo_depth = app.session().redo_depth();
    let prev_dirty = app.session().is_dirty();
    let prev_mutation_seq = app.mutation_seq();
    let method = req.method.clone();
    let id = req.id.clone();
    let mut steps = Vec::new();
    let mut rolled_back = false;
    let (mut response, action) = dispatch_request_inner(app, req, Some(&mut steps), Some(&mut rolled_back));
    if action != ControlLoopAction::Quit {
        let hook_ctx = ControlPostRequestContext {
            method: &method,
            steps: &steps,
            prev_undo_depth,
            prev_redo_depth,
            prev_dirty,
            prev_mutation_seq,
            rolled_back,
            request_ok: response.ok,
        };
        if let Err(err) = on_post_request(app, &hook_ctx) {
            response = ControlResponse::err(id, err);
        }
    }
    (response, action)
}

fn dispatch_request_inner(
    app: &mut UiApp,
    req: ControlRequest,
    mut recorded: Option<&mut Vec<ControlRecordedStep>>,
    rolled_back: Option<&mut bool>,
) -> (ControlResponse, ControlLoopAction) {
    let ControlRequest { id, method, params } = req;
    if method == "app.quit" {
        if let Err(err) = reject_control_if_gui_only(app, "app.quit") {
            return (ControlResponse::err_with_detail(id, err), ControlLoopAction::Continue);
        }
        return (ControlResponse::ok(id, json!({ "quit": true })), ControlLoopAction::Quit);
    }

    let result = if is_gui_action_method(method.as_str()) {
        dispatch_gui_action_and_record(app, &method, params, recorded)
    } else {
        match method.as_str() {
            "ping" => Ok(json!({
                "pong": true,
                "bundled_fonts": crate::bundled_fonts_enabled(),
                "version": env!("CARGO_PKG_VERSION"),
                "gui_only": app.is_gui_only(),
            })),
            "command.list" => Ok(handle_command_list(app)),
            "command.run" => handle_command_run(app, &params, recorded.as_deref_mut()),
            "command.batch" => handle_command_batch(app, &params, recorded, rolled_back),
            "doc.inspect" => reject_control_if_gui_only(app, "doc.inspect")
                .and_then(|()| handle_doc_inspect(app.session().document(), app.session().is_dirty(), app.session().file_path(), &params)),
            "doc.export" => handle_doc_export(app, &params),
            "render.svg" => handle_render_svg(app, &params),
            "render.pdf" => handle_render_pdf(app, &params),
            "render.preview" => handle_render_preview(app, &params),
            "ui.state" | "state.snapshot" => Ok(handle_ui_state(app)),
            "engine.execute" => handle_engine_execute(app, &params, recorded),
            _ => dispatch_command_by_id(app, &method, params, recorded),
        }
    };

    let response = match result {
        Ok(value) => ControlResponse::ok(id, value),
        Err(err) => ControlResponse::err_with_detail(id, err),
    };
    (response, ControlLoopAction::Continue)
}

fn dispatch_gui_action_and_record(
    app: &mut UiApp,
    command_id: &str,
    params: Value,
    recorded: Option<&mut Vec<ControlRecordedStep>>,
) -> Result<Value, ControlCallError> {
    let seq_before = app.mutation_seq();
    let undo_before = app.session().undo_depth();
    let redo_before = app.session().redo_depth();
    let dirty_before = app.session().is_dirty();
    let value = dispatch_gui_action(app, command_id, params.clone()).map_err(ControlCallError::from)?;
    if let Some(steps) = recorded {
        if command_id == "ui.invoke_menu"
            && value.get("command_id").and_then(Value::as_str) == Some("project.save")
            && !value.get("ui").and_then(Value::as_bool).unwrap_or(false)
        {
            steps.push(ControlRecordedStep {
                command_id: String::from("project.save"),
                params: params.get("params").cloned().unwrap_or(Value::Null),
                mutated: false,
            });
        } else {
            let mutated = app.mutation_seq() != seq_before
                || app.session().undo_depth() != undo_before
                || app.session().redo_depth() != redo_before
                || (!dirty_before && app.session().is_dirty())
                || value.get("mutated").and_then(Value::as_bool).unwrap_or(false);
            if mutated {
                steps.push(ControlRecordedStep { command_id: command_id.to_owned(), params, mutated: true });
            }
        }
    }
    Ok(value)
}

fn is_control_readonly_ui_command(command_id: &str) -> bool {
    command_id == "ui.canvas.preflight"
}

fn dispatch_command_by_id(
    app: &mut UiApp,
    command_id: &str,
    params: Value,
    recorded: Option<&mut Vec<ControlRecordedStep>>,
) -> Result<Value, ControlCallError> {
    if is_gui_action_method(command_id) {
        return dispatch_gui_action_and_record(app, command_id, params, recorded);
    }
    if app.is_gui_only()
        && !is_control_readonly_ui_command(command_id)
        && (command_id.starts_with("ui.")
            || command_id == "app.about"
            || command_id == "app.feedback"
            || command_id == "file.print"
            || command_id == "analysis.updateParams")
    {
        return Err(EngineError::with_code(
            ErrorCode::GuiOnlyViolation,
            format!("direct `{command_id}` control request is blocked under --gui-only"),
        )
        .with_remediation("Interact via the 8 GUI actions (`ui.invoke_menu`, `ui.pointer_gesture`, `ui.press_key`, `ui.answer_dialog`).")
        .into());
    }
    let seq_before = app.mutation_seq();
    if let Some(ui_value) = dispatch_ui_command_detailed(app, command_id, &params)? {
        if let Some(steps) = recorded
            && app.mutation_seq() != seq_before
        {
            steps.push(ControlRecordedStep { command_id: command_id.to_owned(), params, mutated: true });
        }
        return Ok(ui_value);
    }
    let outcome = app.execute_control_command(command_id, params.clone())?;
    if let Some(steps) = recorded {
        steps.push(ControlRecordedStep { command_id: command_id.to_owned(), params, mutated: outcome.mutated });
    }
    serde_json::to_value(outcome).map_err(|err| ControlCallError::from(format!("failed to serialize command outcome: {err}")))
}

/// Parse a raw JSON-lines control request into a normalized [`ControlRequest`].
///
/// # Errors
/// Returns `(request_id, error_message)` if `line` is not valid JSON, not a JSON object,
/// or lacks a non-empty `method`/`op` field.
pub fn parse_control_request(line: &str) -> Result<ControlRequest, (Value, String)> {
    let raw: Value = serde_json::from_str(line).map_err(|err| (Value::String(String::new()), format!("invalid control JSON: {err}")))?;
    let Some(obj) = raw.as_object() else {
        return Err((Value::String(String::new()), String::from("control request must be a JSON object")));
    };
    let id = obj.get("id").cloned().unwrap_or_else(|| Value::String(String::new()));
    let Some(method) = obj.get("method").or_else(|| obj.get("op")).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) else {
        return Err((id, String::from("missing non-empty 'method' or 'op' field")));
    };

    let params = host_routing::normalize_control_request_params(method, obj);
    Ok(ControlRequest { id, method: method.to_owned(), params })
}

fn serialize_response(resp: &ControlResponse) -> String {
    const FALLBACK: &str = "{\"id\":\"\",\"ok\":false,\"result\":null,\"error\":\"serialization failure\"}";
    let max_json_bytes = MAX_CONTROL_LINE_BYTES.saturating_sub(1);
    let Ok(encoded) = serde_json::to_string(resp) else {
        return String::from(FALLBACK);
    };
    if encoded.len() <= max_json_bytes {
        return encoded;
    }

    let bounded_id = match serde_json::to_string(&resp.id) {
        Ok(id_json) if id_json.len() <= 1024 => resp.id.clone(),
        _ => Value::String(String::new()),
    };
    let overflow = ControlResponse::err(
        bounded_id,
        format!("control response ({} bytes) exceeds maximum control line size of {MAX_CONTROL_LINE_BYTES} bytes", encoded.len()),
    );
    match serde_json::to_string(&overflow) {
        Ok(text) if text.len() <= max_json_bytes => text,
        _ => String::from(FALLBACK),
    }
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

fn handle_command_run(app: &mut UiApp, params: &Value, recorded: Option<&mut Vec<ControlRecordedStep>>) -> Result<Value, ControlCallError> {
    let command_id = extract_command_id(params)?;
    let cmd_params = extract_nested_params(params);
    if command_id == "app.quit" {
        return Err(ControlCallError::from("use top-level method 'app.quit' to shut down the control server"));
    }
    dispatch_command_by_id(app, &command_id, cmd_params, recorded)
}

struct AtomicBatchRollbackState {
    active: bool,
    prev_tracker: Option<crate::BatchUndoTracker>,
    baseline_session: Option<Session>,
    ui_shell: Option<crate::UiShellSnapshot>,
}

fn is_batch_readonly_step(cmd_id: &str) -> bool {
    matches!(
        cmd_id,
        "ping"
            | "command.list"
            | "doc.inspect"
            | "doc.export"
            | "ui.state"
            | "state.snapshot"
            | "render.svg"
            | "render.pdf"
            | "render.preview"
            | "ui.canvas.preflight"
            | "ui.result.inspectDiagnostics"
            | "result.inspect_diagnostics"
            | "project.inspect"
            | "table.exportCsv"
    )
}

fn is_pure_engine_batch_step(cmd_id: &str) -> bool {
    !is_gui_action_method(cmd_id)
        && lookup_patched_sheet_dialog(cmd_id).is_none()
        && lookup_ui_command_handler(cmd_id).is_none()
        && !cmd_id.starts_with("ui.")
        && !matches!(cmd_id, "app.about" | "app.feedback" | "file.print" | "app.quit")
        && cmd_id != "analysis.updateParams"
}

fn rollback_or_commit_partial_batch(
    app: &mut UiApp,
    rollback: AtomicBatchRollbackState,
    recorded: Option<&mut Vec<ControlRecordedStep>>,
    batch_steps: Vec<ControlRecordedStep>,
    rolled_back: Option<&mut bool>,
) -> bool {
    if rollback.active {
        let _child_tracker = app.batch_undo_log.take();
        app.batch_undo_log = rollback.prev_tracker;
        if let Some(baseline) = rollback.baseline_session {
            app.session = baseline;
        }
        if let Some(ui_snap) = rollback.ui_shell {
            app.restore_ui_shell(ui_snap);
        } else {
            app.sync_views();
        }
        if let Some(flag) = rolled_back {
            *flag = true;
        }
        true
    } else {
        if let Some(out_steps) = recorded {
            out_steps.extend(batch_steps);
        }
        false
    }
}

fn finish_committed_atomic_batch(app: &mut UiApp, rollback: AtomicBatchRollbackState) {
    if !rollback.active {
        return;
    }
    let child_tracker = app.batch_undo_log.take();
    let is_outermost = rollback.prev_tracker.is_none();
    app.batch_undo_log = rollback.prev_tracker;
    if let (Some(parent), Some(child)) = (app.batch_undo_log.as_mut(), child_tracker) {
        parent.merge_committed_child(child);
    } else if is_outermost && let Some(baseline) = rollback.baseline_session {
        let _ = app.session.collapse_atomic_group(baseline);
        app.sync_views();
    }
}

fn try_execute_pure_engine_atomic_batch(
    app: &mut UiApp,
    items: &[Value],
    recorded: Option<&mut Vec<ControlRecordedStep>>,
    rolled_back: Option<&mut bool>,
) -> Option<Result<Value, ControlCallError>> {
    if app.batch_undo_log.is_some() {
        return None;
    }
    let mut steps = Vec::with_capacity(items.len());
    for item in items {
        let cmd_id = extract_command_id(item).ok()?;
        if !is_pure_engine_batch_step(&cmd_id) {
            return None;
        }
        steps.push(BatchCommandStep::new(cmd_id, extract_nested_params(item)));
    }
    let baseline_for_frame_overflow = app.session.clone();
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    match app.session.execute_batch_atomic(&steps, Caller::Control, &cancelled) {
        Ok(outcomes) => {
            let max_payload_bytes = MAX_CONTROL_LINE_BYTES.saturating_sub(512);
            let mut accumulated_bytes: usize = 64;
            let mut results = Vec::with_capacity(outcomes.len());
            let mut any_mutated = false;
            for (idx, (step, outcome)) in steps.iter().zip(outcomes.iter()).enumerate() {
                any_mutated |= outcome.mutated;
                let val = match serde_json::to_value(outcome) {
                    Ok(v) => v,
                    Err(err) => {
                        app.session = baseline_for_frame_overflow;
                        app.sync_views();
                        if let Some(flag) = rolled_back {
                            *flag = true;
                        }
                        return Some(Err(ControlCallError::from(format!("failed to serialize command outcome: {err}"))));
                    }
                };
                let entry = json!({ "index": idx, "command": step.command_id, "result": val });
                let entry_bytes = serde_json::to_string(&entry).map(|s| s.len().saturating_add(1)).unwrap_or(MAX_CONTROL_LINE_BYTES);
                accumulated_bytes = accumulated_bytes.saturating_add(entry_bytes);
                if accumulated_bytes > max_payload_bytes {
                    app.session = baseline_for_frame_overflow;
                    app.sync_views();
                    if let Some(flag) = rolled_back {
                        *flag = true;
                    }
                    return Some(Err(ControlCallError::from(format!(
                        "command.batch serialized payload exceeds maximum control line size of {MAX_CONTROL_LINE_BYTES} bytes (rolled_back=true)"
                    ))));
                }
                results.push(entry);
            }
            if any_mutated {
                app.mutation_seq.0 = app.mutation_seq.0.wrapping_add(1);
            }
            app.sync_views();
            if let Some(out_steps) = recorded {
                for (step, outcome) in steps.into_iter().zip(outcomes) {
                    out_steps.push(ControlRecordedStep { command_id: step.command_id, params: step.params, mutated: outcome.mutated });
                }
            }
            Some(Ok(json!({ "atomic": true, "count": results.len(), "results": results })))
        }
        Err(failure) => {
            app.sync_views();
            if let Some(flag) = rolled_back {
                *flag = true;
            }
            Some(Err(ControlCallError {
                message: format!("batch step {} ({}) failed (rolled_back=true): {}", failure.failed_index, failure.failed_command_id, failure.error),
                detail: Some(engine_error_detail(&failure.error)),
            }))
        }
    }
}

fn handle_command_batch(
    app: &mut UiApp,
    params: &Value,
    mut recorded: Option<&mut Vec<ControlRecordedStep>>,
    mut rolled_back: Option<&mut bool>,
) -> Result<Value, ControlCallError> {
    let Some(items) = params.get("commands").and_then(Value::as_array) else {
        return Err(ControlCallError::from("command.batch requires a 'commands' array"));
    };
    if items.len() > MAX_CONTROL_BATCH_STEPS {
        return Err(ControlCallError::from(format!("command.batch exceeds maximum step limit of {MAX_CONTROL_BATCH_STEPS} (got {})", items.len())));
    }
    let atomic =
        opt_bool_param(params, &["atomic"]).map_err(|_| ControlCallError::from("command.batch 'atomic' must be a boolean"))?.unwrap_or(false);
    if atomic && let Some(res) = try_execute_pure_engine_atomic_batch(app, items, recorded.as_deref_mut(), rolled_back.as_deref_mut()) {
        return res;
    }

    let mut rollback = if atomic {
        let prev_tracker = app.batch_undo_log.take();
        app.batch_undo_log = Some(crate::BatchUndoTracker::for_session(app.session()));
        AtomicBatchRollbackState { active: true, prev_tracker, baseline_session: None, ui_shell: None }
    } else {
        AtomicBatchRollbackState { active: false, prev_tracker: None, baseline_session: None, ui_shell: None }
    };
    let mut batch_steps: Vec<ControlRecordedStep> = Vec::new();
    let max_payload_bytes = MAX_CONTROL_LINE_BYTES.saturating_sub(512);
    let mut accumulated_bytes: usize = 64;

    let mut results = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let cmd_id = match extract_command_id(item) {
            Ok(id) => id,
            Err(err) => {
                let _ = rollback_or_commit_partial_batch(app, rollback, recorded, batch_steps, rolled_back);
                return Err(ControlCallError::from(format!("batch step {idx}: {err}")));
            }
        };
        if rollback.active && !is_batch_readonly_step(&cmd_id) {
            if rollback.baseline_session.is_none() {
                rollback.baseline_session = Some(app.session.clone());
            }
            if rollback.ui_shell.is_none() {
                rollback.ui_shell = Some(app.snapshot_ui_shell());
            }
        }
        let cmd_params = extract_nested_params(item);
        let step_sink = recorded.as_ref().map(|_| &mut batch_steps);
        match dispatch_command_by_id(app, &cmd_id, cmd_params, step_sink) {
            Ok(val) => {
                let entry = json!({ "index": idx, "command": cmd_id, "result": val });
                let entry_bytes = serde_json::to_string(&entry).map(|s| s.len().saturating_add(1)).unwrap_or(MAX_CONTROL_LINE_BYTES);
                accumulated_bytes = accumulated_bytes.saturating_add(entry_bytes);
                if accumulated_bytes > max_payload_bytes {
                    let did_rollback = rollback_or_commit_partial_batch(app, rollback, recorded, batch_steps, rolled_back);
                    return Err(ControlCallError::from(format!(
                        "command.batch serialized payload exceeds maximum control line size of {MAX_CONTROL_LINE_BYTES} bytes (rolled_back={did_rollback})"
                    )));
                }
                results.push(entry);
            }
            Err(err) => {
                let did_rollback = rollback_or_commit_partial_batch(app, rollback, recorded, batch_steps, rolled_back);
                return Err(ControlCallError {
                    message: format!("batch step {idx} ({cmd_id}) failed (rolled_back={did_rollback}): {}", err.message),
                    detail: err.detail,
                });
            }
        }
    }

    finish_committed_atomic_batch(app, rollback);
    if let Some(out_steps) = recorded {
        out_steps.extend(batch_steps);
    }

    Ok(json!({ "atomic": atomic, "count": results.len(), "results": results }))
}

fn handle_engine_execute(app: &mut UiApp, params: &Value, recorded: Option<&mut Vec<ControlRecordedStep>>) -> Result<Value, ControlCallError> {
    require_params_object_or_null(params, "engine.execute")?;
    let cmd_id = opt_non_empty_str_param(params, &["command_id", "command"])?
        .ok_or_else(|| ControlCallError::from("engine.execute requires non-empty 'command_id'"))?;
    let cmd_params = match params.get("params") {
        None | Some(Value::Null) => Value::Null,
        Some(other @ Value::Object(_)) => other.clone(),
        Some(_) => return Err(ControlCallError::from("engine.execute 'params' must be a JSON object or null")),
    };
    let outcome = app.execute_control_command(cmd_id, cmd_params.clone())?;
    if let Some(steps) = recorded {
        steps.push(ControlRecordedStep { command_id: cmd_id.to_owned(), params: cmd_params, mutated: outcome.mutated });
    }
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
        "brushed_cell": app.brushed_cell,
        "brushed_points": app.brushed_points,
        "selected_rows": app.grid.selected_rows,
        "inline_editor_active": app.inline_editor.active,
        "preflight_issue_count": app.canvas_dialogs.preflight_issues.len(),
    })
}

fn handle_doc_inspect(doc: &ProjectDocument, dirty: bool, file_path: Option<&std::path::Path>, params: &Value) -> Result<Value, ControlCallError> {
    require_params_object_or_null(params, "doc.inspect")?;
    let include_cells = opt_bool_param(params, &["include_cells"])?.unwrap_or(false);
    let sheet_id = resolve_sheet_id_param(params)?;
    Ok(inspect_document_control_json(doc, dirty, file_path, sheet_id.as_ref(), include_cells)?)
}

fn visual_sheet_title(doc: &ProjectDocument, sheet_id: &SheetId) -> String {
    if let Some(graph) = doc.graph_sheet(sheet_id) {
        return if graph.spec.title.is_empty() { graph.title.clone() } else { graph.spec.title.clone() };
    }
    doc.layout_sheet(sheet_id).map(|layout| layout.title.clone()).unwrap_or_default()
}

fn handle_render_svg(app: &UiApp, params: &Value) -> Result<Value, ControlCallError> {
    reject_control_if_gui_only(app, "render.svg")?;
    require_params_object_or_null(params, "render.svg")?;
    let sheet_id = resolve_renderable_sheet_id(app.document(), params)?;
    let options = SvgExportOptions {
        pretty: opt_bool_param(params, &["pretty"])?.unwrap_or(true),
        outline_text: opt_bool_param(params, &["outline_text"])?.unwrap_or(false),
        ..SvgExportOptions::default()
    };
    let artifact = export_document_artifact(
        app.document(),
        &ExportArtifactRequest { target: ExportTarget::Explicit(sheet_id.clone()), format: ExportArtifactFormat::Svg { options } },
        None,
        "render.svg",
    )?;
    let title = visual_sheet_title(app.document(), &sheet_id);
    let width_pt = artifact.width_pt.unwrap_or(0.0);
    let height_pt = artifact.height_pt.unwrap_or(0.0);
    let svg = artifact.into_utf8_string()?;
    Ok(json!({ "sheet_id": sheet_id.as_str(), "title": title, "width_pt": width_pt, "height_pt": height_pt, "svg": svg }))
}

fn handle_render_pdf(app: &UiApp, params: &Value) -> Result<Value, ControlCallError> {
    reject_control_if_gui_only(app, "render.pdf")?;
    require_params_object_or_null(params, "render.pdf")?;
    let opt_path = opt_str_param(params, &["path"])?.map(str::trim).filter(|p| !p.is_empty());
    if let Some(path) = opt_path {
        let _ = app.session().validate_host_write_path(path)?;
    }

    let doc = app.document();
    let all = opt_bool_param(params, &["all"])?.unwrap_or(false);
    let sheet_ids = if all { collect_all_renderable_ids(doc)? } else { vec![resolve_renderable_sheet_id(doc, params)?] };
    let color_space = opt_non_empty_str_param(params, &["color_space"])?.map(parse_pdf_color_space).transpose()?.unwrap_or(PdfColorSpace::Srgb);
    let options = PdfExportOptions { color_space, title: Some(doc.title.clone()), ..PdfExportOptions::default() };
    let target = match sheet_ids.as_slice() {
        [only] => ExportTarget::Explicit(only.clone()),
        _ => ExportTarget::AutoOrExplicit(None),
    };
    let request = ExportArtifactRequest {
        target,
        format: ExportArtifactFormat::Pdf { options, all_graphs: false, sheet_ids: (sheet_ids.len() > 1).then(|| sheet_ids.clone()) },
    };
    let artifact = export_document_artifact(doc, &request, None, "render.pdf")?;
    if let Some(path) = opt_path {
        app.session().write_host_file_atomic(path, &artifact.bytes)?;
    }

    let id_strings: Vec<&str> = sheet_ids.iter().map(SheetId::as_str).collect();
    Ok(json!({
        "sheet_id": id_strings.first().copied(),
        "sheet_ids": id_strings,
        "page_count": artifact.page_count,
        "encoding": "base64",
        "bytes_len": artifact.bytes.len(),
        "data_base64": curvebook_platform::base64::encode(&artifact.bytes),
    }))
}

fn reject_control_if_gui_only(app: &UiApp, method: &str) -> Result<(), ControlCallError> {
    if app.is_gui_only() {
        return Err(EngineError::with_code(ErrorCode::GuiOnlyViolation, format!("direct `{method}` control request is blocked under --gui-only"))
            .with_remediation("Inspect visible UI via GUI tools/screenshot or restart without --gui-only.")
            .into());
    }
    Ok(())
}

fn opt_u32_bounded_param(params: &Value, key: &str, min: u32, max: u32) -> Result<Option<u32>, String> {
    let Some(raw) = opt_usize_param(params, &[key])? else {
        return Ok(None);
    };
    let val = u32::try_from(raw).map_err(|_| format!("`{key}` exceeds u32 range"))?;
    if val < min || val > max {
        return Err(format!("`{key}` must be in {min}..={max} (got {val})"));
    }
    Ok(Some(val))
}

fn opt_u16_bounded_param(params: &Value, key: &str, min: u16, max: u16) -> Result<Option<u16>, String> {
    let Some(raw) = opt_u32_bounded_param(params, key, u32::from(min), u32::from(max))? else {
        return Ok(None);
    };
    u16::try_from(raw).map(Some).map_err(|_| format!("`{key}` out of range"))
}

fn enforce_control_payload_budget(bytes_len: usize) -> Result<(), ControlCallError> {
    let max_raw_bytes = (MAX_CONTROL_LINE_BYTES.saturating_mul(3)) / 4 - 4096;
    if bytes_len > max_raw_bytes {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("encoded payload ({bytes_len} bytes) exceeds control frame limit"))
            .with_remediation("Specify a lower DPI/resolution or export smaller targets.")
            .into());
    }
    Ok(())
}

fn handle_render_preview(app: &UiApp, params: &Value) -> Result<Value, ControlCallError> {
    reject_control_if_gui_only(app, "render.preview")?;
    require_params_object_or_null(params, "render.preview")?;
    let doc = app.document();
    let sheet_id = resolve_renderable_sheet_id(doc, params)?;
    let dpi = parse_dpi(params.get("dpi"), DEFAULT_PREVIEW_DPI)?;
    let width_px = opt_u32_bounded_param(params, "width_px", 1, MAX_CAPPED_RASTER_DIM_PX)?;
    let height_px = opt_u32_bounded_param(params, "height_px", 1, MAX_CAPPED_RASTER_DIM_PX)?;

    let sheet_kind = if doc.graph_sheet(&sheet_id).is_some() { "graph" } else { "layout" };
    let scene = compile_visual_scene_for_sheet(doc, &sheet_id)?;
    let artifact = export_document_artifact(
        doc,
        &ExportArtifactRequest {
            target: ExportTarget::Explicit(sheet_id.clone()),
            format: ExportArtifactFormat::Png {
                width_px,
                height_px,
                dpi: Some(dpi),
                sizing: RasterSizingPolicy::CapToLimits { default_dpi: DEFAULT_PREVIEW_DPI },
            },
        },
        None,
        "render.preview",
    )?;
    enforce_control_payload_budget(artifact.bytes.len())?;
    let raster = artifact.raster.ok_or_else(|| ControlCallError::from("missing raster metadata for preview"))?;

    Ok(json!({
        "sheet_id": sheet_id.as_str(),
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
        "encoding": "base64",
        "data_base64": curvebook_platform::base64::encode(&artifact.bytes),
    }))
}

fn handle_doc_export(app: &UiApp, params: &Value) -> Result<Value, ControlCallError> {
    reject_control_if_gui_only(app, "doc.export")?;
    require_params_object_or_null(params, "doc.export")?;
    let raw_format =
        opt_non_empty_str_param(params, &["format"])?.ok_or_else(|| ControlCallError::from("doc.export requires a non-empty string 'format'"))?;
    let format = DocumentExportFormat::parse(raw_format)?;

    let validated_out_path =
        opt_non_empty_str_param(params, &["path", "output_path"])?.map(|raw| app.session().validate_host_write_path(raw)).transpose()?;

    let opt_ttf_font = opt_non_empty_str_param(params, &["ttf_font"])?;
    if let Some(raw) = opt_ttf_font
        && format != DocumentExportFormat::Pdf
    {
        let _ = app.session().validate_host_write_path(raw)?;
    }

    let tab_id = opt_non_empty_str_param(params, &["tab_id", "tabId"])?;
    let mut spec = ExportOptionsSpec::new(format);
    spec.sheet_id = opt_non_empty_str_param(params, &["sheet_id", "sheetId"])?.map(SheetId::new);
    spec.pretty = opt_bool_param(params, &["pretty"])?.unwrap_or(true);
    spec.tidy = resolve_tidy_flag(opt_bool_param(params, &["tidy"])?, tab_id)?;
    spec.tab_id = tab_id.map(str::to_owned);
    spec.csv_options = parse_csv_delimiter_opt(opt_str_param(params, &["csv_delimiter"])?, "csv_delimiter")?;
    spec.svg_precision = opt_u16_bounded_param(params, "svg_precision", 1, 8)?
        .map(|p| u8::try_from(p).map_err(|_| ControlCallError::from("svg_precision out of range")))
        .transpose()?;
    spec.pdf_color_space =
        opt_non_empty_str_param(params, &["pdf_color_space", "color_space"])?.map(parse_pdf_color_space).transpose()?.unwrap_or(PdfColorSpace::Srgb);
    spec.pdf_version = opt_non_empty_str_param(params, &["pdf_version"])?.map(parse_pdf_version).transpose()?.unwrap_or(PdfVersion::Pdf17);
    spec.all_graphs = opt_bool_param(params, &["all_graphs", "all"])?.unwrap_or(false);
    spec.dpi = Some(parse_dpi(params.get("dpi"), DEFAULT_PREVIEW_DPI)?);
    spec.width_px = opt_u32_bounded_param(params, "width_px", 1, MAX_CAPPED_RASTER_DIM_PX)?;
    spec.height_px = opt_u32_bounded_param(params, "height_px", 1, MAX_CAPPED_RASTER_DIM_PX)?;
    spec.tiff_compression =
        opt_non_empty_str_param(params, &["tiff_compression"])?.map(parse_tiff_compression).transpose()?.unwrap_or(TiffCompression::Lzw);
    spec.embedded_ttf_font = if format == DocumentExportFormat::Pdf {
        opt_ttf_font.map(|raw| load_validated_ttf_font(app.session().path_policy(), raw)).transpose()?
    } else {
        None
    };

    let request = spec.into_artifact_request(app.document(), true)?;
    let is_delimited = matches!(format, DocumentExportFormat::Csv | DocumentExportFormat::Tsv);
    let artifact = export_document_artifact(app.document(), &request, None, "doc.export").map_err(|err| {
        if is_delimited {
            ControlCallError { message: format!("CSV export failed: {err}"), detail: Some(engine_error_detail(&err)) }
        } else {
            ControlCallError::from(err)
        }
    })?;

    if let Some(dest_path) = validated_out_path {
        app.session().write_host_file_atomic(&dest_path, &artifact.bytes)?;
        return Ok(json!({
            "exported": true,
            "format": format.as_str(),
            "sheet_id": artifact.target_id,
            "path": dest_path,
            "bytes_written": artifact.bytes.len(),
            "width_px": artifact.raster.map(|r| r.width_px),
            "height_px": artifact.raster.map(|r| r.height_px),
            "dpi": artifact.raster.map(|r| r.dpi),
        }));
    }

    enforce_control_payload_budget(artifact.bytes.len())?;
    Ok(json!({
        "format": format.as_str(),
        "sheet_id": artifact.target_id,
        "width_px": artifact.raster.map(|r| r.width_px),
        "height_px": artifact.raster.map(|r| r.height_px),
        "dpi": artifact.raster.map(|r| r.dpi),
        "bytes_len": artifact.bytes.len(),
        "encoding": "base64",
        "data_base64": curvebook_platform::base64::encode(&artifact.bytes),
    }))
}

const SHEET_ID_KEYS: &[&str] = &["sheet_id", "sheetId"];
const EMBEDDED_OPEN_IGNORE_KEYS: &[&str] = &["sheet_id", "sheetId", "annotation_index", "panel_index", "item_index"];

fn resolve_target_sheet_matching(
    doc: &ProjectDocument,
    params: &Value,
    extra_keys: &[&str],
    kind_label: &str,
    validate_explicit: bool,
    matches_kind: impl Fn(&ProjectDocument, &SheetId) -> bool,
    first_fallback: impl Fn(&ProjectDocument) -> Option<SheetId>,
) -> Result<SheetId, String> {
    let explicit = if let Some(raw) = opt_non_empty_str_param(params, extra_keys)? {
        Some(SheetId::try_new(raw).map_err(|e| e.to_string())?)
    } else {
        resolve_sheet_id_param(params)?
    };
    if let Some(id) = explicit {
        if !validate_explicit || matches_kind(doc, &id) {
            return Ok(id);
        }
        return Err(format!("{kind_label} `{}` not found", id.as_str()));
    }
    if let Some(active) = doc.active_sheet_id.as_ref().filter(|id| matches_kind(doc, id)) {
        return Ok(active.clone());
    }
    first_fallback(doc).ok_or_else(|| format!("no {kind_label} available"))
}

fn resolve_target_data_id(doc: &ProjectDocument, params: &Value) -> Result<SheetId, String> {
    resolve_target_sheet_matching(
        doc,
        params,
        &["source_sheet_id"],
        "data sheet",
        true,
        |d, id| d.data_sheet(id).is_some(),
        |d| d.data_sheets.first().map(|s| s.id.clone()),
    )
}

fn resolve_target_graph_id(doc: &ProjectDocument, params: &Value) -> Result<SheetId, String> {
    resolve_target_sheet_matching(
        doc,
        params,
        &[],
        "graph sheet",
        true,
        |d, id| d.graph_sheet(id).is_some(),
        |d| d.graph_sheets.first().map(|g| g.id.clone()),
    )
}

fn resolve_target_layout_id(doc: &ProjectDocument, params: &Value) -> Result<SheetId, String> {
    resolve_target_sheet_matching(
        doc,
        params,
        &[],
        "layout sheet",
        true,
        |d, id| d.layout_sheet(id).is_some(),
        |d| d.layout_sheets.first().map(|l| l.id.clone()),
    )
}

fn resolve_target_result_sheet_id(doc: &ProjectDocument, params: &Value) -> Result<SheetId, String> {
    resolve_target_sheet_matching(
        doc,
        params,
        &["result_sheet_id"],
        "result sheet",
        false,
        |d, id| d.result_sheet(id).is_some(),
        |d| d.result_sheets.first().map(|r| r.id.clone()),
    )
}

fn resolve_renderable_sheet_id(doc: &ProjectDocument, params: &Value) -> Result<SheetId, String> {
    resolve_target_sheet_matching(
        doc,
        params,
        &[],
        "graph or layout sheet",
        false,
        |d, id| matches!(d.sheet_section(id), Some(SheetSection::Graphs | SheetSection::Layouts)),
        |d| d.graph_sheets.first().map(|g| g.id.clone()).or_else(|| d.layout_sheets.first().map(|l| l.id.clone())),
    )
}

fn apply_open_patch(params: &Value, ignore_keys: &[&str], apply: impl FnOnce(&Map<String, Value>) -> Result<(), String>) -> Result<(), String> {
    let Some(obj) = params.as_object() else {
        return Ok(());
    };
    let fields: Map<String, Value> = obj.iter().filter(|(k, _)| !ignore_keys.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
    if !fields.is_empty() {
        apply(&fields)?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum SheetDialogTarget {
    Data,
    Graph,
    Layout,
}

impl SheetDialogTarget {
    const fn kind_label(self) -> &'static str {
        match self {
            Self::Data => "data sheet",
            Self::Graph => "graph sheet",
            Self::Layout => "layout sheet",
        }
    }

    const fn response_id_key(self) -> &'static str {
        match self {
            Self::Data => "sheet_id",
            Self::Graph => "graph_sheet_id",
            Self::Layout => "layout_sheet_id",
        }
    }

    fn resolve(self, doc: &ProjectDocument, params: &Value) -> Result<SheetId, String> {
        match self {
            Self::Data => resolve_target_data_id(doc, params),
            Self::Graph => resolve_target_graph_id(doc, params),
            Self::Layout => resolve_target_layout_id(doc, params),
        }
    }
}

type SheetDialogOpenFn = fn(&mut UiApp, &ProjectDocument, &SheetId) -> bool;
type SheetDialogPatchFn = fn(&mut UiApp, &ProjectDocument, &Map<String, Value>) -> Result<(), String>;

struct PatchedSheetDialogSpec {
    command_id: &'static str,
    dialog_name: &'static str,
    target: SheetDialogTarget,
    open: SheetDialogOpenFn,
    patch: Option<SheetDialogPatchFn>,
}

const PATCHED_SHEET_DIALOGS: &[PatchedSheetDialogSpec] = &[
    PatchedSheetDialogSpec {
        command_id: "ui.changeGraphType.open",
        dialog_name: "ChangeGraphType",
        target: SheetDialogTarget::Graph,
        open: |a, d, id| {
            let Some(graph) = d.graph_sheet(id) else {
                return false;
            };
            a.canvas_dialogs.change_graph_type.open_for_graph(graph, d);
            a.canvas_dialogs.active_dialog = ActiveDialog::ChangeGraphType { graph_sheet_id: id.clone() };
            true
        },
        patch: None,
    },
    PatchedSheetDialogSpec {
        command_id: "ui.table.convertArchetype.open",
        dialog_name: "ConvertArchetype",
        target: SheetDialogTarget::Data,
        open: |a, d, id| a.canvas_dialogs.open_convert_archetype(d, id),
        patch: None,
    },
    PatchedSheetDialogSpec {
        command_id: "ui.graph.bubblePlot.open",
        dialog_name: "BubblePlotInspector",
        target: SheetDialogTarget::Graph,
        open: |a, d, id| a.canvas_dialogs.open_bubble_plot(d, id),
        patch: Some(|a, _, f| crate::dialogs_canvas::dialog_patch::patch_bubble_plot(&mut a.canvas_dialogs.bubble_plot, f)),
    },
    PatchedSheetDialogSpec {
        command_id: "ui.graph.format.open",
        dialog_name: "FormatGraph",
        target: SheetDialogTarget::Graph,
        open: |a, d, id| a.canvas_dialogs.open_format_graph(d, id),
        patch: Some(|a, _, f| {
            let preset = a.locale_preset.clone();
            crate::dialogs_canvas::dialog_patch::patch_format_graph(&mut a.canvas_dialogs.format_graph, f, &preset)
        }),
    },
    PatchedSheetDialogSpec {
        command_id: "ui.graph.pairwise.open",
        dialog_name: "PairwiseComparisons",
        target: SheetDialogTarget::Graph,
        open: |a, d, id| a.canvas_dialogs.open_pairwise_comparisons(d, id),
        patch: Some(|a, d, f| {
            let preset = a.locale_preset.clone();
            crate::dialogs_canvas::dialog_patch::patch_pairwise(&mut a.canvas_dialogs.pairwise, f, Some(d), &preset)
        }),
    },
    PatchedSheetDialogSpec {
        command_id: "ui.layout.composer.open",
        dialog_name: "PageLayoutComposer",
        target: SheetDialogTarget::Layout,
        open: |a, d, id| {
            let _ = a.canvas_dialogs.open_page_layout_composer(d, id);
            true
        },
        patch: Some(|a, d, f| crate::dialogs_canvas::dialog_patch::patch_page_layout(&mut a.canvas_dialogs.page_layout, f, Some(d))),
    },
    PatchedSheetDialogSpec {
        command_id: "ui.layout.equalizeScaling.open",
        dialog_name: "EqualizeScaling",
        target: SheetDialogTarget::Layout,
        open: |a, d, id| a.canvas_dialogs.open_equalize_scaling(d, id),
        patch: Some(|a, _, f| {
            let preset = a.locale_preset.clone();
            crate::dialogs_canvas::dialog_patch::patch_equalize_scaling(&mut a.canvas_dialogs.equalize_scaling, f, &preset)
        }),
    },
];

fn lookup_patched_sheet_dialog(command_id: &str) -> Option<&'static PatchedSheetDialogSpec> {
    PATCHED_SHEET_DIALOGS.iter().find(|spec| spec.command_id == command_id)
}

fn open_sheet_dialog_patched(
    app: &mut UiApp,
    params: &Value,
    target: SheetDialogTarget,
    dialog_name: &str,
    open: impl FnOnce(&mut UiApp, &ProjectDocument, &SheetId) -> bool,
    patch: Option<impl FnOnce(&mut UiApp, &ProjectDocument, &Map<String, Value>) -> Result<(), String>>,
) -> Result<Value, String> {
    let doc = app.document().clone();
    let sheet_id = target.resolve(&doc, params)?;
    if !open(app, &doc, &sheet_id) {
        return Err(format!("{} `{}` not found", target.kind_label(), sheet_id.as_str()));
    }
    if let Some(patch_fn) = patch {
        apply_open_patch(params, SHEET_ID_KEYS, |fields| patch_fn(app, &doc, fields))?;
    }
    Ok(json!({ "dialog": dialog_name, target.response_id_key(): sheet_id.as_str() }))
}

fn open_patched_sheet_dialog_spec(app: &mut UiApp, params: &Value, spec: &PatchedSheetDialogSpec) -> Result<Value, String> {
    open_sheet_dialog_patched(app, params, spec.target, spec.dialog_name, spec.open, spec.patch)
}

fn dispatch_ui_analyze_open(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let doc = app.session().document().clone();
    let target_id = resolve_target_sheet_matching(
        &doc,
        params,
        &[],
        "data or analyzable sheet",
        false,
        |d, id| {
            d.data_sheet(id).is_some()
                || d.result_sheet(id)
                    .is_some_and(|rs| rs.analyzable_data.is_some() || rs.tabs.iter().any(|t| !t.column_headers.is_empty() && !t.rows.is_empty()))
        },
        |d| d.data_sheets.first().map(|ds| ds.id.clone()),
    )?;
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
    let preset = app.locale_preset.clone();
    apply_open_patch(params, SHEET_ID_KEYS, |f| {
        crate::dialogs_canvas::dialog_patch::patch_power_wizard(&mut app.canvas_dialogs.power_wizard, f, &preset)
    })?;
    Ok(json!({
        "dialog": "PowerAndSampleSize",
        "stage": app.canvas_dialogs.power_wizard.stage.label(),
        "stage_index": app.canvas_dialogs.power_wizard.stage.index(),
    }))
}

fn dispatch_ui_format_points_open(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let doc = app.document().clone();
    let graph_id = resolve_target_graph_id(&doc, params)?;
    let graph = doc.graph_sheet(&graph_id).ok_or_else(|| format!("graph sheet `{}` not found", graph_id.as_str()))?;
    let mut points: Vec<(usize, usize, Option<usize>)> = match params.get("points") {
        Some(v) if !v.is_null() => parse_selected_points_with_subcols(v, "points")?,
        _ if !app.brushed_points.is_empty() => app
            .brushed_points
            .iter()
            .map(|pt| {
                let subcols = doc.data_sheet(&pt.sheet_id).map_or(1, |ds| crate::grid_view::data_set_subcolumn_count(&ds.archetype, pt.col));
                (pt.col, pt.row, (subcols > 1).then_some(pt.subcol))
            })
            .collect(),
        _ => doc
            .data_sheet(&graph.spec.primary_source_sheet_id)
            .map_or_else(Vec::new, |ds| app.grid.selected_graph_point_targets(ds, graph.spec.series.len().max(1))),
    };
    points.sort_unstable();
    points.dedup();
    app.canvas_dialogs.open_format_selected_points_with_subcols(&doc, &graph_id, points);
    let preset = app.locale_preset.clone();
    apply_open_patch(params, &["sheet_id", "sheetId", "points"], |f| {
        crate::dialogs_canvas::dialog_patch::patch_format_selected_points(&mut app.canvas_dialogs.format_points, f, &preset)
    })?;
    Ok(json!({
        "dialog": "FormatSelectedPoints",
        "graph_sheet_id": graph_id.as_str(),
        "selected_count": app.canvas_dialogs.format_points.selected_points.len(),
    }))
}

fn dispatch_ui_clone_pipeline_open(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let doc = app.document();
    let target = if let Some(id) = resolve_sheet_id_param(params)? {
        let ds = doc.data_sheet(&id).ok_or_else(|| format!("data sheet `{}` not found for clone pipeline", id.as_str()))?;
        Some((ds.id.clone(), ds.title.clone()))
    } else {
        doc.active_sheet_id.as_ref().and_then(|id| doc.data_sheet(id)).or_else(|| doc.data_sheets.first()).map(|ds| (ds.id.clone(), ds.title.clone()))
    };
    if let Some((source_id, title)) = target {
        app.nav.welcome.trigger_clone_pipeline(source_id.clone(), title);
        Ok(json!({ "dialog": "ClonePipeline", "open": true, "source_sheet_id": source_id.as_str() }))
    } else {
        app.nav.welcome.open = true;
        app.nav.welcome.mode = WelcomeMode::ClonePipeline;
        Ok(json!({ "dialog": "ClonePipeline", "open": true, "source_sheet_id": Value::Null }))
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
    let enabled_opt = opt_bool_param(params, &["enabled"])?;
    let ratio_opt = opt_finite_f64_param(params, &["ratio"])?;
    let auto_pin_graph_opt = opt_bool_param(params, &["auto_pin_graph"])?;
    let primary_opt = opt_non_empty_str_param(params, &["primary_sheet_id"])?.map(SheetId::try_new).transpose().map_err(|e| e.to_string())?;
    let secondary_opt = opt_non_empty_str_param(params, &["secondary_sheet_id"])?.map(SheetId::try_new).transpose().map_err(|e| e.to_string())?;

    let was_enabled = app.split.enabled;
    let prev_primary = app.split.primary_sheet_id.clone();
    let prev_secondary = app.split.secondary_sheet_id.clone();
    let is_swap = was_enabled
        && enabled_opt != Some(false)
        && primary_opt.is_some()
        && secondary_opt.is_some()
        && primary_opt == prev_secondary
        && secondary_opt == prev_primary;

    app.split.enabled = enabled_opt.unwrap_or(!app.split.enabled);
    if let Some(ratio) = ratio_opt {
        app.split.set_ratio(ratio);
    }
    if let Some(p_id) = primary_opt {
        app.split.primary_sheet_id = Some(p_id);
    }
    if let Some(s_id) = secondary_opt {
        app.split.secondary_sheet_id = Some(s_id);
    }

    let doc = app.session.document().clone();
    let (primary, secondary) = if app.split.enabled {
        let (p, s) = app.split.materialize_sheets(&doc);
        let is_data_graph_pair =
            p.as_ref().is_some_and(|id| doc.data_sheet(id).is_some()) && s.as_ref().is_some_and(|id| doc.graph_sheet(id).is_some());
        if !is_swap && (auto_pin_graph_opt == Some(true) || (!was_enabled && is_data_graph_pair && auto_pin_graph_opt != Some(false))) {
            if ratio_opt.is_none() {
                app.split.set_ratio(crate::dialogs_canvas::SplitWorkspaceState::DATA_GRAPH_DEFAULT_RATIO);
            }
            if let Some(graph_id) = s.as_ref().filter(|id| doc.graph_sheet(id).is_some()) {
                app.split.inspector_target_sheet_id = Some(graph_id.clone());
                app.split.inspector_pinned = true;
            }
        } else if !is_swap && auto_pin_graph_opt == Some(false) {
            app.split.inspector_pinned = false;
            app.split.inspector_target_sheet_id = doc.active_sheet_id.clone();
        }
        if !is_swap {
            app.split.sync_pin_invariants(&doc);
        }
        (p, s)
    } else {
        app.split.sync_pin_invariants(&doc);
        app.split.resolve_sheets(&doc)
    };

    Ok(json!({
        "enabled": app.split.enabled,
        "ratio": app.split.ratio,
        "primary_sheet_id": primary.as_ref().map(SheetId::as_str),
        "secondary_sheet_id": secondary.as_ref().map(SheetId::as_str),
        "inspector_target_sheet_id": app.split.effective_inspector_target(&doc).map(SheetId::as_str),
        "inspector_pinned": app.split.inspector_pinned,
    }))
}

fn dispatch_ui_brush_point(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    if opt_bool_param(params, &["clear"])? == Some(true) {
        app.clear_brush();
        return Ok(json!({ "brushed_cell": Value::Null }));
    }
    let sheet_id = resolve_target_sheet_matching(
        app.document(),
        params,
        &[],
        "data sheet",
        false,
        |d, id| d.data_sheet(id).is_some(),
        |d| d.data_sheets.first().map(|s| s.id.clone()),
    )?;
    let row = opt_usize_param(params, &["row"])?.unwrap_or(0);
    let col = opt_usize_param(params, &["col"])?.unwrap_or(0);
    let subcol = opt_usize_param(params, &["subcol"])?.unwrap_or(0);
    app.brush_from_grid_cell(&sheet_id, GridCellCoord::new(row, col, subcol));
    Ok(json!({ "brushed_cell": app.brushed_cell }))
}

fn dispatch_ui_result_copy_methods_report(app: &mut UiApp, params: &Value) -> Result<Value, ControlCallError> {
    let rs_id = resolve_target_result_sheet_id(app.document(), params)?;
    if let Some(fmt) = opt_non_empty_str_param(params, &["format"])? {
        app.canvas_dialogs.result_view.report_format = fmt.to_owned();
    }
    let skip_os_clipboard = opt_bool_param(params, &["skip_os_clipboard"])? == Some(true);
    let intent = app.canvas_dialogs.result_view.generate_report_intent(&rs_id);
    let outcome = app.execute_command(&intent.command_id, intent.params)?;
    let report = outcome.payload.get("report").and_then(Value::as_str).unwrap_or_default().to_owned();
    app.canvas_dialogs.result_view.last_methods_report = Some(report.clone());

    let clipboard_status = if skip_os_clipboard {
        Value::Null
    } else {
        let status = app.write_clipboard_text(&report).map_err(|err| ControlCallError::from(format!("clipboard write failed: {err}")))?;
        serde_json::to_value(status).map_err(|err| ControlCallError::from(format!("failed to serialize clipboard status: {err}")))?
    };
    Ok(json!({
        "sheet_id": rs_id.as_str(),
        "format": app.canvas_dialogs.result_view.report_format,
        "report": report,
        "clipboard_status": clipboard_status,
    }))
}

fn dispatch_ui_edit_params_open(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let doc = app.document().clone();
    let has_explicit = params.get("result_sheet_id").and_then(Value::as_str).is_some() || resolve_sheet_id_param(params)?.is_some();
    let rs_id = if has_explicit {
        resolve_target_result_sheet_id(&doc, params)?
    } else if let Some(focused_or_active) = crate::interaction::resolve_result_sheet_for_shortcut(app, true) {
        focused_or_active
    } else {
        resolve_target_result_sheet_id(&doc, params)?
    };
    if !app.canvas_dialogs.open_edit_parameters(&doc, &rs_id) {
        return Err(format!("result sheet `{}` not found", rs_id.as_str()));
    }
    Ok(json!({ "dialog": "EditAnalysisParameters", "result_sheet_id": rs_id.as_str(), "opened": true }))
}

fn dispatch_ui_result_apply_remediation(app: &UiApp, params: &Value) -> Result<Value, ControlCallError> {
    let _ = opt_non_empty_str_param(params, &["result_sheet_id"])?;
    let _ = resolve_sheet_id_param(params)?;
    let rs_id = resolve_target_result_sheet_id(app.document(), params).ok();
    Err(EngineError::with_code(
        ErrorCode::Conflict,
        "automatic assumption remediation is disabled under QOL-7 diagnostics-only policy (diagnostics_only_policy); zero document mutation performed",
    )
    .with_details(json!({
        "code": "diagnostics_only_policy",
        "policy": "QOL-7",
        "result_sheet_id": rs_id.as_ref().map(SheetId::as_str),
        "mutated": false,
    }))
    .with_remediation("Use `ui.result.inspectDiagnostics` to inspect Residuals/Diagnostics tabs or press Cmd+T to change test parameters explicitly.")
    .into())
}

fn dispatch_ui_result_inspect_diagnostics(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let doc = app.document().clone();
    let rs_id = resolve_target_result_sheet_id(&doc, params)?;
    let rs = doc.result_sheet(&rs_id).ok_or_else(|| format!("result sheet `{}` not found", rs_id.as_str()))?;
    app.canvas_dialogs.result_view.sync_with_result(rs);
    let banner = AssumptionDiagnosticBanner::from_result(rs);

    let requested_tab = opt_non_empty_str_param(params, &["tab_id"])?;
    let requested_tab_index = opt_usize_param(params, &["tab_index"])?;
    let mut detail_tab_ids: Vec<String> = banner.as_ref().map(|b| b.detail_tab_ids.clone()).unwrap_or_else(|| {
        rs.tabs
            .iter()
            .filter(|t| {
                matches!(
                    t.kind,
                    curvebook_doc::ResultTabKind::Diagnostics
                        | curvebook_doc::ResultTabKind::NormalQq
                        | curvebook_doc::ResultTabKind::Residuals
                        | curvebook_doc::ResultTabKind::CovarianceMatrix
                        | curvebook_doc::ResultTabKind::LogMinusLog
                )
            })
            .map(|t| t.id.clone())
            .collect()
    });
    if detail_tab_ids.is_empty() {
        detail_tab_ids = rs.tabs.iter().map(|t| t.id.clone()).collect();
    }

    if let Some(tab_id) = requested_tab {
        if !app.canvas_dialogs.result_view.select_tab_by_id(rs, tab_id) {
            return Err(format!("result sheet `{}` has no tab `{tab_id}`", rs_id.as_str()));
        }
    } else if let Some(idx) = requested_tab_index {
        if idx >= rs.tabs.len() {
            return Err(format!("tab_index {idx} out of range (sheet has {} tabs)", rs.tabs.len()));
        }
        app.canvas_dialogs.result_view.select_tab(idx, rs.tabs.len());
    } else if let Some(first_detail) = detail_tab_ids.first() {
        let _ = app.canvas_dialogs.result_view.select_tab_by_id(rs, first_detail);
    }
    let active_tab_id = app.canvas_dialogs.result_view.active_tab(rs).map(|t| t.id.clone());

    Ok(json!({
        "result_sheet_id": rs_id.as_str(),
        "active_tab_index": app.canvas_dialogs.result_view.active_tab_index,
        "tab_id": active_tab_id,
        "active_tab_id": active_tab_id,
        "detail_tab_ids": detail_tab_ids,
        "banner": banner,
        "mutated": false,
    }))
}

fn dispatch_ui_canvas_apply_preset(app: &mut UiApp, params: &Value) -> Result<Value, ControlCallError> {
    let sheet_id = resolve_renderable_sheet_id(app.document(), params)?;
    let preset_raw = opt_non_empty_str_param(params, &["preset"])?.ok_or_else(|| String::from("missing required `preset` string parameter"))?;
    let preset = FigureCanvasPreset::from_name(preset_raw).ok_or_else(|| format!("unknown canvas preset `{preset_raw}`"))?;
    if let Some(mode_raw) = opt_non_empty_str_param(params, &["resize_mode"])? {
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
    let raw = opt_non_empty_str_param(params, &["filter"])?.ok_or_else(|| String::from("missing non-empty `filter` string"))?;
    let filter = CanvasPreviewFilter::from_name(raw).ok_or_else(|| format!("unknown preview filter `{raw}`"))?;
    app.preview_filter = filter;
    Ok(json!({ "preview_filter": app.preview_filter.as_str() }))
}

fn dispatch_ui_workspace_preset_set(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let raw = opt_non_empty_str_param(params, &["preset"])?.ok_or_else(|| String::from("missing non-empty `preset` string"))?;
    #[cfg(not(target_arch = "wasm32"))]
    {
        let preset = crate::egui_shell::WorkspacePreset::parse(raw).ok_or_else(|| format!("unknown workspace preset `{raw}`"))?;
        let cfg = preset.default_dock_config();
        app.nav.navigator_open = cfg.navigator_open;
        app.family_open = cfg.family_open;
        app.nav.inspector_open = cfg.inspector_open;
        app.egui_views.workspace_preset = preset;
        app.egui_views.leading_collapsed_rail = cfg.leading_collapsed_rail;
        app.egui_views.trailing_collapsed_rail = cfg.trailing_collapsed_rail;
        app.egui_views.leading_width_pt = Some(cfg.leading_width_pt);
        app.egui_views.trailing_width_pt = Some(cfg.trailing_width_pt);
        Ok(json!({ "workspace_preset": app.egui_views.workspace_preset.as_str() }))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = app;
        match raw.trim().to_ascii_lowercase().as_str() {
            "default" | "data_prep" | "modeling" | "publication" | "compact" => Ok(json!({ "workspace_preset": raw.trim().to_ascii_lowercase() })),
            _ => Err(format!("unknown workspace preset `{raw}`")),
        }
    }
}

fn dispatch_ui_grid_exclude_with_reason(app: &mut UiApp, params: &Value) -> Result<Value, ControlCallError> {
    let doc = app.document();
    let sheet_id = resolve_target_data_id(doc, params)?;
    let ds = doc.data_sheet(&sheet_id).ok_or_else(|| format!("data sheet `{}` not found", sheet_id.as_str()))?;
    let mut coords = app.grid.selection_coords(ds);
    if coords.is_empty()
        && let Some(active) = app.grid.tool.active
    {
        coords.push(active);
    }
    let tag_override = opt_non_empty_str_param(params, &["reason_tag"])?.map(str::to_owned);
    let note_override = opt_str_param(params, &["note"])?.map(str::to_owned);
    let should_commit = opt_bool_param(params, &["commit"])? == Some(true);
    app.canvas_dialogs.open_exclude_with_reason(sheet_id.clone(), coords.clone());
    if let Some(tag) = tag_override {
        app.canvas_dialogs.exclude_with_reason.reason_tag = tag;
    }
    if let Some(note) = note_override {
        app.canvas_dialogs.exclude_with_reason.note = note;
    }
    if should_commit {
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
    Ok(json!({ "dialog": "ExcludeWithReason", "sheet_id": sheet_id.as_str(), "coords_count": app.canvas_dialogs.exclude_with_reason.coords.len() }))
}

fn dispatch_ui_randomize_groups_open(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let doc = app.document().clone();
    app.canvas_dialogs.open_randomize_groups(&doc);
    apply_open_patch(params, SHEET_ID_KEYS, |f| {
        crate::dialogs_canvas::dialog_patch::patch_randomize_groups(&mut app.canvas_dialogs.randomize_groups, f)
    })?;
    Ok(json!({
        "dialog": "RandomizeGroups",
        "groups": app.canvas_dialogs.randomize_groups.groups,
        "subject_count": app.canvas_dialogs.randomize_groups.subject_count,
    }))
}

fn dispatch_ui_random_sample_rows_open(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let doc = app.document().clone();
    let sheet_id = resolve_target_data_id(&doc, params)?;
    if !app.canvas_dialogs.open_random_sample_rows(&doc, Some(&sheet_id)) {
        return Err(format!("data sheet `{}` not found", sheet_id.as_str()));
    }
    apply_open_patch(params, &["source_sheet_id", "sheet_id", "sheetId"], |f| {
        crate::dialogs_canvas::dialog_patch::patch_random_sample_rows(&mut app.canvas_dialogs.random_sample_rows, f)
    })?;
    Ok(json!({
        "dialog": "RandomSampleRows",
        "source_sheet_id": sheet_id.as_str(),
        "sample_size": app.canvas_dialogs.random_sample_rows.sample_size,
    }))
}

fn dispatch_ui_simulate_distributions_open(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    app.canvas_dialogs.open_simulate_distributions();
    let preset = app.locale_preset.clone();
    apply_open_patch(params, SHEET_ID_KEYS, |f| {
        crate::dialogs_canvas::dialog_patch::patch_simulate_distributions(&mut app.canvas_dialogs.simulate_distributions, f, &preset)
    })?;
    Ok(json!({
        "dialog": "SimulateDistributions",
        "n_rows": app.canvas_dialogs.simulate_distributions.n_rows,
        "column_count": app.canvas_dialogs.simulate_distributions.columns.len(),
    }))
}

fn dispatch_ui_insert_model_equation_open(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let preset = app.locale_preset.clone();
    let mut payload = open_sheet_dialog_patched(
        app,
        params,
        SheetDialogTarget::Graph,
        "InsertModelEquation",
        |a, d, id| a.canvas_dialogs.open_insert_model_equation(d, id),
        Some(|a: &mut UiApp, _: &ProjectDocument, f: &Map<String, Value>| {
            crate::dialogs_canvas::dialog_patch::patch_insert_model_equation(&mut a.canvas_dialogs.insert_model_equation, f, &preset)
        }),
    )?;
    if let Some(obj) = payload.as_object_mut() {
        obj.insert(
            String::from("result_sheet_id"),
            app.canvas_dialogs.insert_model_equation.result_sheet_id.as_ref().map_or(Value::Null, |id| Value::String(id.as_str().to_owned())),
        );
        obj.insert(String::from("mode"), json!(app.canvas_dialogs.insert_model_equation.mode));
    }
    Ok(payload)
}

fn dispatch_ui_apply_style_from_open(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let doc = app.document().clone();
    if let Some(arr) = params.get("target_sheet_ids").and_then(Value::as_array) {
        let targets: Vec<SheetId> = arr.iter().filter_map(Value::as_str).map(SheetId::new).collect();
        if !targets.is_empty() {
            if !app.canvas_dialogs.open_apply_style_from_targets(&doc, &targets) {
                return Err(String::from("none of the requested `target_sheet_ids` matched existing graph sheets"));
            }
            return Ok(json!({
                "dialog": "ApplyStyleFrom",
                "target_sheet_ids": app.canvas_dialogs.apply_style.target_graph_ids.iter().map(SheetId::as_str).collect::<Vec<_>>(),
            }));
        }
    }
    let gallery_graphs: Vec<SheetId> = app.nav.selected_gallery_ids().into_iter().filter(|id| doc.graph_sheet(id).is_some()).collect();
    if gallery_graphs.len() >= 2 && params.get("sheet_id").is_none() && params.get("sheetId").is_none() {
        let _ = app.canvas_dialogs.open_apply_style_from_targets(&doc, &gallery_graphs);
        return Ok(json!({
            "dialog": "ApplyStyleFrom",
            "target_sheet_ids": app.canvas_dialogs.apply_style.target_graph_ids.iter().map(SheetId::as_str).collect::<Vec<_>>(),
        }));
    }
    let no_patch: Option<SheetDialogPatchFn> = None;
    open_sheet_dialog_patched(
        app,
        params,
        SheetDialogTarget::Graph,
        "ApplyStyleFrom",
        |a, d, id| a.canvas_dialogs.open_apply_style_from(d, id),
        no_patch,
    )
}

fn dispatch_ui_format_axes_open(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let doc = app.document().clone();
    let graph_id = resolve_target_graph_id(&doc, params)?;
    let graph = doc.graph_sheet(&graph_id).ok_or_else(|| format!("graph sheet `{}` not found", graph_id.as_str()))?;
    let initial_target = match opt_non_empty_str_param(params, &["target_axis"])?.unwrap_or("y").to_ascii_lowercase().as_str() {
        "x" => AxisTarget::X,
        "y" => AxisTarget::Y,
        "y2" => AxisTarget::Y2,
        other => return Err(format!("unsupported target_axis `{other}`; expected x, y, or y2")),
    };
    app.canvas_dialogs.format_axes.sync_from_graph(graph, initial_target);
    app.canvas_dialogs.active_dialog = ActiveDialog::FormatAxes { sheet_id: graph_id.clone() };
    let preset = app.locale_preset.clone();
    apply_open_patch(params, SHEET_ID_KEYS, |f| {
        crate::dialogs_canvas::dialog_patch::patch_format_axes(&mut app.canvas_dialogs.format_axes, f, Some(&doc), &preset)
    })?;
    Ok(json!({ "dialog": "FormatAxes", "sheet_id": graph_id.as_str(), "target_axis": format!("{:?}", app.canvas_dialogs.format_axes.target_axis) }))
}

fn dispatch_ui_open_formula_bar(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let doc = app.document().clone();
    let ds_id = resolve_target_data_id(&doc, params)?;
    let ds = doc.data_sheet(&ds_id).ok_or_else(|| format!("data sheet `{}` not found", ds_id.as_str()))?;
    let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
        return Err(format!("formula bar requires a Multiple Variables sheet (`{}` is {:?})", ds_id.as_str(), ds.kind()));
    };

    let req_title = opt_non_empty_str_param(params, &["title"])?;
    let req_col = opt_usize_param(params, &["column", "col"])?;

    match (req_col, req_title) {
        (Some(col), maybe_title) if col < mv.variables.len() => {
            app.grid.formula_bar.open_for_column(ds, col, Some(&doc));
            if let Some(t) = maybe_title {
                app.grid.formula_bar.target_title = t.to_owned();
            }
        }
        (Some(col), Some(t)) if col == mv.variables.len() => {
            app.grid.formula_bar.open_for_new_column(ds, t, Some(&doc));
        }
        (Some(col), _) => {
            return Err(format!("variable column {col} out of range (sheet has {} variables)", mv.variables.len()));
        }
        (None, Some(t)) => {
            if let Some(col) = mv.find_variable(t) {
                app.grid.formula_bar.open_for_column(ds, col, Some(&doc));
            } else {
                app.grid.formula_bar.open_for_new_column(ds, t, Some(&doc));
            }
        }
        (None, None) => {
            let fallback_col = app.grid.tool.active.map(|c| c.col).filter(|c| *c < mv.variables.len()).unwrap_or(0);
            if fallback_col < mv.variables.len() {
                app.grid.formula_bar.open_for_column(ds, fallback_col, Some(&doc));
            } else {
                return Err(String::from("MultipleVariables sheet has no variables to target"));
            }
        }
    }

    app.focus_widget_id = Some(String::from("formula_bar.input"));

    Ok(json!({
        "sheet_id": ds_id.as_str(),
        "active": app.grid.formula_bar.active,
        "target_column": app.grid.formula_bar.target_column,
        "target_title": app.grid.formula_bar.target_title,
        "input": app.grid.formula_bar.input,
        "is_valid": app.grid.formula_bar.is_valid,
        "parse_error": app.grid.formula_bar.parse_error,
    }))
}

fn dispatch_ui_append_cluster_column(app: &mut UiApp, params: &Value) -> Result<Value, ControlCallError> {
    let doc = app.document().clone();
    let rs_id = resolve_target_result_sheet_id(&doc, params)?;
    let rs = doc.result_sheet(&rs_id).ok_or_else(|| format!("result sheet `{}` not found", rs_id.as_str()))?.clone();
    if !matches!(rs.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Clustering { .. })) {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("result sheet `{}` is not a Cluster analysis", rs_id.as_str())).into());
    }
    let source_id = rs.analysis.source_sheet_id.clone();
    let ds = doc.data_sheet(&source_id).ok_or_else(|| format!("source data sheet `{}` not found", source_id.as_str()))?;
    let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("cluster column append requires a MultipleVariables source table (`{}`)", source_id.as_str()),
        )
        .into());
    };
    let col_title = opt_non_empty_str_param(params, &["column_title", "title"])?.unwrap_or("Cluster");
    let (assigned, assigned_count) = resolve_cluster_assignment_column_labels(&rs, mv)?;
    if assigned_count == 0 {
        return Err(EngineError::with_code(ErrorCode::BadInput, "clustering `kmeans-assignments` tab has no cluster assignments").into());
    }

    if let Some(col_idx) = mv.find_variable(col_title) {
        let edits: Vec<Value> =
            assigned.iter().enumerate().map(|(row, label_opt)| json!({ "row": row, "col": col_idx, "value": label_opt.as_deref() })).collect();
        app.execute_command("table.setCells", json!({ "sheet_id": source_id.as_str(), "edits": edits }))?;
        app.execute_command("table.setVariableRole", json!({ "sheet_id": source_id.as_str(), "col": col_idx, "role": "categorical" }))?;
        return Ok(json!({
            "result_sheet_id": rs_id.as_str(),
            "source_sheet_id": source_id.as_str(),
            "column": col_idx,
            "title": col_title,
            "updated_existing": true,
            "assigned_rows": assigned_count,
        }));
    }

    let target_col = mv.variables.len();
    let rows: Vec<Vec<Value>> = assigned.iter().map(|opt| vec![opt.as_ref().map_or(Value::Null, |s| Value::String(s.clone()))]).collect();
    app.execute_command("table.paste", json!({ "sheet_id": source_id.as_str(), "mode": "append_columns", "rows": rows }))?;
    app.execute_command("table.setGroupTitles", json!({ "sheet_id": source_id.as_str(), "col": target_col, "title": col_title }))?;
    app.execute_command("table.setVariableRole", json!({ "sheet_id": source_id.as_str(), "col": target_col, "role": "categorical" }))?;

    Ok(json!({
        "result_sheet_id": rs_id.as_str(),
        "source_sheet_id": source_id.as_str(),
        "column": target_col,
        "title": col_title,
        "updated_existing": false,
        "assigned_rows": assigned_count,
    }))
}

fn resolve_cluster_assignment_column_labels(rs: &ResultSheet, mv: &MultipleVariablesTable) -> Result<(Vec<Option<String>>, usize), String> {
    let tab = rs
        .tabs
        .iter()
        .find(|t| t.id == "kmeans-assignments")
        .ok_or_else(|| String::from("Clustering result sheet has no `kmeans-assignments` tab; enable K-means (`k_means_k`) first"))?;
    let row_count = mv.row_count();
    let mut assigned = vec![None; row_count];
    let mut assigned_count = 0usize;
    for (k, row) in tab.rows.iter().enumerate() {
        let Some(cluster_label) = format_cluster_assignment_cell(row.first()) else {
            continue;
        };
        let Some(target_row) = map_cluster_row_label_to_mv_row(tab.row_labels.get(k).map(String::as_str), k, mv) else {
            continue;
        };
        if let Some(slot) = assigned.get_mut(target_row) {
            if slot.is_none() {
                assigned_count += 1;
            }
            *slot = Some(cluster_label);
        }
    }
    Ok((assigned, assigned_count))
}

fn format_cluster_assignment_cell(cell: Option<&ResultCell>) -> Option<String> {
    match cell? {
        ResultCell::Number { value, .. } if value.is_finite() => {
            let cluster_num = (*value).round() as i64;
            (cluster_num >= 1).then(|| format!("Cluster {cluster_num}"))
        }
        ResultCell::Text(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() { None } else { Some(trimmed.to_owned()) }
        }
        _ => None,
    }
}

fn map_cluster_row_label_to_mv_row(label: Option<&str>, fallback_idx: usize, mv: &MultipleVariablesTable) -> Option<usize> {
    let row_count = mv.row_count();
    if let Some(lbl) = label.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(pos) = mv.row_titles.iter().position(|t| !t.trim().is_empty() && t.trim() == lbl) {
            return Some(pos);
        }
        if let Some(obs) = lbl.strip_prefix("Obs ").and_then(|s| s.trim().parse::<usize>().ok())
            && obs >= 1
            && obs <= row_count
        {
            return Some(obs - 1);
        }
    }
    (fallback_idx < row_count).then_some(fallback_idx)
}

fn dispatch_ui_create_graph_open(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let doc = app.document().clone();
    const KEYS: &[&str] = &["data_sheet_id", "source_sheet_id", "sheet_id", "sheetId"];
    let preferred = opt_non_empty_str_param(params, KEYS)?.map(SheetId::new);
    if !app.canvas_dialogs.open_create_graph(&doc, preferred.as_ref()) {
        return Err(String::from("no source data sheet available to create a graph"));
    }
    apply_open_patch(params, KEYS, |f| crate::dialogs_canvas::dialog_patch::patch_create_graph(&mut app.canvas_dialogs.create_graph, f))?;
    Ok(json!({ "dialog": "CreateGraph", "source_sheet_id": app.canvas_dialogs.create_graph.source_sheet_id.as_ref().map(SheetId::as_str) }))
}

fn resolve_target_embedded_index(doc: &ProjectDocument, sheet_id: &SheetId, explicit_idx: Option<usize>, want_image: bool) -> Option<usize> {
    let match_ann = |a: &curvebook_doc::GraphAnnotation| {
        if want_image {
            matches!(a, curvebook_doc::GraphAnnotation::EmbeddedImage { .. })
        } else {
            matches!(a, curvebook_doc::GraphAnnotation::EmbeddedResultTable { .. })
        }
    };
    let match_item = |it: &curvebook_doc::LayoutItem| {
        if want_image {
            matches!(it, curvebook_doc::LayoutItem::EmbeddedImage { .. })
        } else {
            matches!(it, curvebook_doc::LayoutItem::LiveTable { .. })
        }
    };
    if let Some(gs) = doc.graph_sheet(sheet_id) {
        return match explicit_idx {
            Some(idx) => gs.spec.annotations.get(idx).is_some_and(match_ann).then_some(idx),
            None => gs.spec.annotations.iter().rposition(match_ann),
        };
    }
    let ls = doc.layout_sheet(sheet_id)?;
    match explicit_idx {
        Some(idx) => ls.spec.items.get(idx).is_some_and(match_item).then_some(idx),
        None => ls.spec.items.iter().rposition(match_item),
    }
}

fn dispatch_ui_grid_overlay_toggle(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    if let Some(spacing) = opt_finite_f64_param(params, &["spacing_pt"])? {
        if spacing <= 1.0 || spacing > 200.0 {
            return Err(String::from("`spacing_pt` must be in (1.0, 200.0]"));
        }
        app.canvas_dialogs.grid_overlay_spacing_pt = spacing;
    }
    app.canvas_dialogs.grid_overlay_visible = opt_bool_param(params, &["visible"])?.unwrap_or(!app.canvas_dialogs.grid_overlay_visible);
    Ok(json!({
        "grid_overlay_visible": app.canvas_dialogs.grid_overlay_visible,
        "spacing_pt": app.canvas_dialogs.grid_overlay_spacing_pt,
    }))
}

fn dispatch_ui_embedded_object_format_open(app: &mut UiApp, params: &Value, want_image: bool) -> Result<Value, String> {
    let doc = app.document().clone();
    let sheet_id = resolve_target_sheet_matching(
        &doc,
        params,
        &[],
        "graph or layout sheet",
        true,
        |d, id| d.graph_sheet(id).is_some() || d.layout_sheet(id).is_some(),
        |d| d.graph_sheets.first().map(|g| g.id.clone()).or_else(|| d.layout_sheets.first().map(|l| l.id.clone())),
    )?;
    let explicit_idx = opt_usize_param(params, &["annotation_index", "panel_index", "item_index"])?;
    let (label, dialog_name) = if want_image { ("embedded image", "FormatImage") } else { ("embedded result table", "FormatEmbeddedTable") };
    let idx = resolve_target_embedded_index(&doc, &sheet_id, explicit_idx, want_image)
        .ok_or_else(|| format!("no {label} found on `{}`", sheet_id.as_str()))?;
    let opened = if want_image {
        app.canvas_dialogs.open_format_image(&doc, &sheet_id, idx)
    } else {
        app.canvas_dialogs.open_format_embedded_table(&doc, &sheet_id, idx)
    };
    if !opened {
        return Err(format!("failed opening {dialog_name} on `{}` index {idx}", sheet_id.as_str()));
    }
    let preset = app.locale_preset.clone();
    apply_open_patch(params, EMBEDDED_OPEN_IGNORE_KEYS, |f| {
        if want_image {
            crate::dialogs_canvas::dialog_patch::patch_format_image(&mut app.canvas_dialogs.format_image, f, &preset)
        } else {
            crate::dialogs_canvas::dialog_patch::patch_format_embedded_table(&mut app.canvas_dialogs.format_embedded_table, f, &preset)
        }
    })?;
    let is_layout = if want_image { app.canvas_dialogs.format_image.is_layout } else { app.canvas_dialogs.format_embedded_table.is_layout };
    Ok(json!({ "dialog": dialog_name, "sheet_id": sheet_id.as_str(), "target_index": idx, "is_layout": is_layout }))
}

fn dispatch_ui_sheet_find(app: &mut UiApp, _params: &Value) -> Result<Value, String> {
    let doc = app.document();
    let active_sheet_id = doc
        .active_sheet_id
        .as_ref()
        .filter(|sid| doc.data_sheet(sid).is_some() || doc.result_sheet(sid).is_some())
        .or_else(|| doc.data_sheets.first().map(|s| &s.id))
        .or_else(|| doc.result_sheets.first().map(|s| &s.id))
        .map(|sid| sid.as_str().to_owned());
    let Some(sheet_id) = active_sheet_id else {
        return Err(String::from("no data or result sheet available to search"));
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Ok(sid) = SheetId::try_new(&sheet_id) {
            app.egui_views.sheet_find.open_for_sheet(Some(sid));
        }
    }
    Ok(json!({
        "sheet_find_open": true,
        "sheet_id": sheet_id,
    }))
}

fn dispatch_ui_row_titles_toggle(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let explicit_collapsed = opt_bool_param(params, &["collapsed"])?;
    #[cfg(not(target_arch = "wasm32"))]
    let fallback_collapsed = app.egui_views.column_grid.row_titles_collapsed;
    #[cfg(target_arch = "wasm32")]
    let fallback_collapsed = false;
    let next_collapsed = explicit_collapsed.unwrap_or(!app.grid.row_titles_collapsed_override.unwrap_or(fallback_collapsed));
    app.grid.row_titles_collapsed_override = Some(next_collapsed);
    #[cfg(not(target_arch = "wasm32"))]
    {
        app.egui_views.column_grid.row_titles_collapsed = next_collapsed;
    }
    Ok(json!({ "row_titles_collapsed": next_collapsed, "row_titles_expanded": !next_collapsed }))
}

fn dispatch_ui_export_figure_open(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let sheet_id = resolve_renderable_sheet_id(app.document(), params)?;
    app.canvas_dialogs.open_export_figure(&sheet_id);
    apply_open_patch(params, SHEET_ID_KEYS, |f| crate::dialogs_canvas::dialog_patch::patch_export_figure(&mut app.canvas_dialogs.export_figure, f))?;
    Ok(json!({ "dialog": "ExportFigure", "sheet_id": sheet_id.as_str(), "format": app.canvas_dialogs.export_figure.format }))
}

#[derive(Clone, Copy)]
enum CsvExportOpenMode {
    DataTable,
    ResultSheet,
    AllDataTables,
}

fn dispatch_ui_csv_export_open(app: &mut UiApp, params: &Value, mode: CsvExportOpenMode) -> Result<Value, String> {
    use crate::dialogs_canvas::CsvExportTargetKind;

    let (target_kind, target_label, sheet_id_opt, table_count_opt) = match mode {
        CsvExportOpenMode::DataTable => {
            let id = resolve_target_data_id(app.document(), params)?;
            (CsvExportTargetKind::DataTable { sheet_id: id.clone() }, "DataTable", Some(id), None)
        }
        CsvExportOpenMode::ResultSheet => {
            let id = resolve_target_result_sheet_id(app.document(), params)?;
            (CsvExportTargetKind::ResultSheet { sheet_id: id.clone() }, "ResultSheet", Some(id), None)
        }
        CsvExportOpenMode::AllDataTables => {
            if app.document().data_sheets.is_empty() {
                return Err(String::from("no data tables available to export"));
            }
            (CsvExportTargetKind::AllDataTables, "AllDataTables", None, Some(app.document().data_sheets.len()))
        }
    };
    app.canvas_dialogs.csv_export.reset_for_locale(app.locale_preset.decimal_separator);
    app.canvas_dialogs.active_dialog = ActiveDialog::CsvExportOptions(target_kind);
    let mut payload = json!({
        "dialog": "CsvExportOptions",
        "target": target_label,
        "delimiter": app.canvas_dialogs.csv_export.delimiter.to_string(),
        "decimal_separator": app.canvas_dialogs.csv_export.decimal_separator.to_string(),
    });
    if let Some(obj) = payload.as_object_mut() {
        if let Some(id) = sheet_id_opt {
            obj.insert(String::from("sheet_id"), json!(id.as_str()));
        }
        if let Some(count) = table_count_opt {
            obj.insert(String::from("table_count"), json!(count));
        }
    }
    Ok(payload)
}

fn dispatch_ui_export_all_tables_csv_fallback(app: &mut UiApp, _params: &Value) -> Result<Value, String> {
    if app.document().data_sheets.is_empty() {
        return Err(String::from("no data tables available to export"));
    }
    Err(String::from("ui.export.allTablesCsv requires the native desktop host folder picker"))
}

fn dispatch_ui_import_warnings_open(app: &mut UiApp, _params: &Value) -> Result<Value, String> {
    app.import_warnings.panel_open = true;
    app.canvas_dialogs.active_dialog = ActiveDialog::ImportWarnings;
    Ok(json!({
        "dialog": "ImportWarnings",
        "open": true,
        "count": app.import_warnings.warnings.len(),
        "command_id": app.import_warnings.command_id,
        "source_label": app.import_warnings.source_label,
    }))
}

fn dispatch_ui_copy_for_slides(app: &mut UiApp, params: &Value) -> Result<Value, ControlCallError> {
    require_params_object_or_null(params, "ui.clipboard.copyForSlides")?;
    let sheet_id = resolve_renderable_sheet_id(app.document(), params).map_err(ControlCallError::from)?;
    let dpi = opt_u16_bounded_param(params, "dpi", 72, 1200)?.unwrap_or(300);
    let outcome = crate::interaction::copy_figure_sheet_slide_bundle_dpi(app, &sheet_id, dpi).map_err(ControlCallError::from)?;
    Ok(outcome.command_result.unwrap_or_else(|| json!({ "sheet_id": sheet_id.as_str(), "dpi": dpi })))
}

fn dispatch_ui_import_reshape_open(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    if let Some(mode_raw) = opt_non_empty_str_param(params, &["mode"])? {
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
    let copies_opt = opt_usize_param(params, &["copies"])?;
    let printer_present = params.get("printer").is_some();
    let printer_opt =
        if printer_present { opt_str_param(params, &["printer"])?.map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned) } else { None };
    let scale_opt = opt_finite_f64_param(params, &["scale_percent"])?;
    let crop_marks_opt = opt_bool_param(params, &["crop_marks"])?;
    let bleed_pt_opt = opt_finite_f64_param(params, &["bleed_pt"])?;
    let spool = opt_bool_param(params, &["spool"])?.unwrap_or(false);
    let submit = opt_bool_param(params, &["submit_to_os"])?.unwrap_or(false);
    if app.is_gui_only() && submit {
        return Err(String::from("submitting print jobs directly to the OS queue is blocked under --gui-only"));
    }

    let active = app.session.document().active_sheet_id.clone();
    app.open_print_dialog_for(sheet_override.as_ref().or(active.as_ref()));
    app.canvas_dialogs.print.submit_to_os = submit;

    if let Some(copies) = copies_opt {
        let clamped = u16::try_from(copies.clamp(1, 999)).unwrap_or(999);
        app.update_print_dialog_copies(clamped);
    }
    if printer_present {
        app.select_print_dialog_printer(printer_opt);
    }
    if let Some(scale) = scale_opt {
        app.update_print_dialog_scale(scale);
    }
    if let Some(crop_marks) = crop_marks_opt {
        app.update_print_dialog_crop_marks(crop_marks, bleed_pt_opt);
    }

    if spool {
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
    match opt_bool_param(params, &["open"])? {
        None => {
            let _ = app.nav.toggle_gallery();
        }
        Some(explicit) => app.nav.gallery.open = explicit,
    }
    Ok(json!({
        "gallery_open": app.nav.gallery.open,
        "filter_folder": app.nav.gallery.filter_folder.map(sheet_folder_kind_name),
        "card_count": app.nav.gallery.cards.len(),
    }))
}

fn dispatch_ui_pane_cycle(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let reverse = opt_bool_param(params, &["reverse"])?.unwrap_or(false);
    let active = app.cycle_focus_pane(reverse);
    Ok(json!({ "active_pane": active_pane_name(active), "reverse": reverse }))
}

fn dispatch_ui_rulers_toggle(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let unit_override = match opt_non_empty_str_param(params, &["unit"])? {
        Some(raw_unit) => Some(match raw_unit.to_ascii_lowercase().as_str() {
            "mm" | "millimeters" | "millimeter" => PhysicalUnit::Millimeters,
            "in" | "inch" | "inches" => PhysicalUnit::Inches,
            "pt" | "points" | "point" => PhysicalUnit::Points,
            other => return Err(format!("unsupported ruler unit `{other}`; expected pt, in, or mm")),
        }),
        None => None,
    };
    let visible_override = opt_bool_param(params, &["visible"])?;
    if let Some(unit) = unit_override {
        app.canvas_dialogs.ruler_unit = unit;
    }
    app.canvas_dialogs.rulers_visible = visible_override.unwrap_or(!app.canvas_dialogs.rulers_visible);
    Ok(json!({ "rulers_visible": app.canvas_dialogs.rulers_visible, "unit": physical_unit_name(app.canvas_dialogs.ruler_unit) }))
}

fn dispatch_ui_shortcut_set(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let Some(command_id) = opt_non_empty_str_param(params, &["command_id", "id"])? else {
        return Err(String::from("ui.shortcut.set requires non-empty string 'command_id'"));
    };
    let Some(shortcut) = opt_str_param(params, &["shortcut", "combo"])? else {
        return Err(String::from("ui.shortcut.set requires string 'shortcut'"));
    };
    app.shortcuts.set_shortcut(command_id, shortcut)?;
    app.canvas_dialogs.command_palette.sync_shortcuts(&app.shortcuts);
    let active = app.shortcuts.active_combo(command_id).unwrap_or("");
    Ok(json!({
        "command_id": command_id,
        "shortcut": active,
    }))
}

fn dispatch_ui_shortcut_reset(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let target = opt_non_empty_str_param(params, &["command_id", "id"])?;
    if let Some(command_id) = target {
        app.shortcuts.reset_shortcut(command_id)?;
        app.canvas_dialogs.command_palette.sync_shortcuts(&app.shortcuts);
        let active = app.shortcuts.active_combo(command_id).unwrap_or("");
        Ok(json!({
            "command_id": command_id,
            "shortcut": active,
            "reset_all": false,
        }))
    } else {
        app.shortcuts.reset_all();
        app.canvas_dialogs.command_palette.sync_shortcuts(&app.shortcuts);
        Ok(json!({
            "reset_all": true,
        }))
    }
}

fn dispatch_ui_locale_set(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let Some(raw_locale) = opt_non_empty_str_param(params, &["locale"])? else {
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
    }))
}

fn dispatch_ui_theme_set(app: &mut UiApp, params: &Value) -> Result<Value, String> {
    let Some(raw_mode) = opt_non_empty_str_param(params, &["mode", "theme"])? else {
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
    if !params.is_object() {
        return Err(String::from("command descriptor must be a JSON object with 'command' or 'command_id'"));
    }
    opt_non_empty_str_param(params, &["command", "command_id"])?
        .map(ToOwned::to_owned)
        .ok_or_else(|| String::from("missing non-empty 'command' or 'command_id' field"))
}

fn extract_nested_params(params: &Value) -> Value {
    params.as_object().map_or(Value::Null, host_routing::extract_nested_params_like_control_io)
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
        ScenePrimitive::RasterImage(img) => {
            let mut next = img.clone();
            next.bounds_pt = map_rect(img.bounds_pt);
            next.crop = curvebook_doc::CropInsetsPt {
                left_pt: img.crop.left_pt * scalar,
                right_pt: img.crop.right_pt * scalar,
                top_pt: img.crop.top_pt * scalar,
                bottom_pt: img.crop.bottom_pt * scalar,
            };
            next.clip = img.clip.map(map_rect);
            ScenePrimitive::RasterImage(next)
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
        ActiveDialog::RandomizeGroups => "RandomizeGroups",
        ActiveDialog::RandomSampleRows { .. } => "RandomSampleRows",
        ActiveDialog::SimulateDistributions => "SimulateDistributions",
        ActiveDialog::InsertModelEquation { .. } => "InsertModelEquation",
        ActiveDialog::CreateGraph { .. } => "CreateGraph",
        ActiveDialog::EqualizeScaling { .. } => "EqualizeScaling",
        ActiveDialog::FormatEmbeddedTable { .. } => "FormatEmbeddedTable",
        ActiveDialog::FormatImage { .. } => "FormatImage",
        ActiveDialog::CommandPalette => "CommandPalette",
        ActiveDialog::Checkpoints => "Checkpoints",
        ActiveDialog::About => "About",
        ActiveDialog::Feedback => "Feedback",
        ActiveDialog::ImportWarnings => "ImportWarnings",
        ActiveDialog::ImportProjectMode { .. } => "ImportProjectMode",
        ActiveDialog::CsvExportOptions(_) => "CsvExportOptions",
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

    #[test]
    fn control_auth_token_generation_validation_and_constant_time_verify() -> Result<(), Box<dyn std::error::Error>> {
        use super::{CONTROL_AUTH_TOKEN_BYTES, ControlAuth};

        let a = ControlAuth::generate()?;
        let b = ControlAuth::generate()?;
        assert_eq!(a.as_str().len(), CONTROL_AUTH_TOKEN_BYTES * 2);
        assert!(a.as_str().bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')));
        assert_ne!(a.as_str(), b.as_str());
        assert!(a.verify(a.as_str()));
        assert!(!a.verify(b.as_str()));
        assert!(!a.verify(""));
        assert!(!a.verify(&format!("{}x", a.as_str())));
        assert_eq!(format!("{a:?}"), "ControlAuth([REDACTED])");

        assert!(ControlAuth::from_token("").is_err());
        assert!(ControlAuth::from_token("has space").is_err());
        assert!(ControlAuth::from_token("has\nnewline").is_err());
        assert!(ControlAuth::from_token("a".repeat(600)).is_err());
        let explicit = ControlAuth::from_token("sample-explicit-secret")?;
        assert!(explicit.verify("sample-explicit-secret"));
        assert!(!explicit.verify("sample-explicit-secre"));
        Ok(())
    }

    #[test]
    fn control_stream_rejects_http_and_unauthenticated_before_dispatch() -> Result<(), Box<dyn std::error::Error>> {
        use std::io::{BufReader, Cursor};

        use super::{CONTROL_HTTP_FORBIDDEN_RESPONSE, ControlAuth, serve_control_stream_with_quit};
        use crate::UiApp;

        let auth = ControlAuth::from_token("sample-stream-secret")?;
        let mut app = UiApp::new();

        let http_attack = concat!(
            "POST / HTTP/1.1\r\n",
            "Host: 127.0.0.1:9001\r\n",
            "Origin: https://attacker.example\r\n",
            "Content-Type: text/plain\r\n",
            "Content-Length: 74\r\n",
            "\r\n",
            "{\"id\":\"pwn\",\"method\":\"sheet.createData\",\"params\":{\"sheet_id\":\"s1\"}}\n"
        );
        let mut http_reader = BufReader::new(Cursor::new(http_attack.as_bytes()));
        let mut http_out = Vec::new();
        let quit = serve_control_stream_with_quit(&mut app, &mut http_reader, &mut http_out, &auth)?;
        assert!(!quit);
        assert_eq!(String::from_utf8(http_out)?, CONTROL_HTTP_FORBIDDEN_RESPONSE);
        assert!(app.document().data_sheets.is_empty(), "HTTP body command must never run");

        let script = concat!(
            "{\"id\":1,\"method\":\"ping\"}\n",
            "{\"id\":2,\"method\":\"sheet.createData\",\"token\":\"wrong\",\"params\":{\"sheet_id\":\"s1\"}}\n",
            "{\"id\":3,\"method\":\"app.quit\"}\n",
            "{\"id\":4,\"method\":\"ping\",\"token\":\"sample-stream-secret\"}\n",
            "{\"id\":5,\"method\":\"ping\"}\n",
            "{\"id\":6,\"method\":\"auth\",\"params\":{\"token\":\"wrong\"}}\n",
            "{\"id\":7,\"method\":\"auth\",\"params\":{\"token\":\"sample-stream-secret\"}}\n",
            "{\"id\":8,\"method\":\"ping\",\"token\":\"wrong\"}\n",
            "{\"id\":9,\"method\":\"sheet.createData\",\"params\":{\"sheet_id\":\"s-ok\",\"title\":\"OK\"}}\n",
            "{\"id\":10,\"method\":\"app.quit\"}\n"
        );
        let mut reader = BufReader::new(Cursor::new(script.as_bytes()));
        let mut out = Vec::new();
        let quit = serve_control_stream_with_quit(&mut app, &mut reader, &mut out, &auth)?;
        assert!(quit);

        let lines: Vec<serde_json::Value> = String::from_utf8(out)?.lines().map(serde_json::from_str).collect::<Result<Vec<_>, _>>()?;
        assert_eq!(lines.len(), 10);
        for unauth_idx in [0usize, 1, 2, 4, 5, 7] {
            assert_eq!(lines[unauth_idx]["ok"], serde_json::json!(false), "expected failure at idx {unauth_idx}");
            assert!(
                lines[unauth_idx]["error"].as_str().is_some_and(|e| e.contains("unauthorized")),
                "expected unauthorized error at idx {unauth_idx}: {}",
                lines[unauth_idx]
            );
            assert_eq!(lines[unauth_idx]["error_detail"]["code"], serde_json::json!("unauthorized"));
        }
        assert_eq!(lines[3]["ok"], serde_json::json!(true));
        assert_eq!(lines[6]["ok"], serde_json::json!(true));
        assert_eq!(lines[6]["result"]["authenticated"], serde_json::json!(true));
        assert_eq!(lines[8]["ok"], serde_json::json!(true));
        assert_eq!(lines[9]["result"]["quit"], serde_json::json!(true));
        assert_eq!(app.document().data_sheets.len(), 1);
        assert_eq!(app.document().data_sheets[0].id.as_str(), "s-ok");
        Ok(())
    }

    #[test]
    #[allow(clippy::panic)]
    fn control_server_panic_guard_keeps_stream_and_tcp_listener_alive() -> Result<(), Box<dyn std::error::Error>> {
        use std::io::{BufReader, BufWriter, Cursor, ErrorKind, Write};
        use std::net::{SocketAddr, TcpStream};
        use std::time::Duration;

        use super::{
            ControlAuth, ControlLoopAction, ControlRequest, ControlResponse, bind_control_tcp_listener, dispatch_request,
            serve_control_listener_with_handler, serve_control_stream_with_handler,
        };
        use crate::UiApp;

        let auth = ControlAuth::from_token("sample-panic-guard-token")?;
        let mut app = UiApp::new();
        let stream_input = concat!(
            "{\"id\":\"a\",\"method\":\"auth\",\"params\":{\"token\":\"sample-panic-guard-token\"}}\n",
            "{\"id\":\"boom-1\",\"method\":\"test.panic\"}\n",
            "{\"id\":\"ok-after\",\"method\":\"ping\"}\n",
            "{\"id\":\"q\",\"method\":\"app.quit\"}\n"
        );
        let mut reader = BufReader::new(Cursor::new(stream_input.as_bytes()));
        let mut out = Vec::new();
        let quit = serve_control_stream_with_handler(&mut app, &mut reader, &mut out, &auth, |ui, req: ControlRequest| {
            if req.method == "test.panic" {
                panic!("simulated handler panic for R1-S10-03");
            }
            dispatch_request(ui, req)
        })?;
        assert!(quit);

        let stream_lines: Vec<serde_json::Value> = String::from_utf8(out)?.lines().map(serde_json::from_str).collect::<Result<Vec<_>, _>>()?;
        assert_eq!(stream_lines.len(), 4);
        assert_eq!(stream_lines[0]["ok"], serde_json::json!(true));
        assert_eq!(stream_lines[1]["id"], serde_json::json!("boom-1"));
        assert_eq!(stream_lines[1]["ok"], serde_json::json!(false));
        assert_eq!(stream_lines[1]["error_detail"]["code"], serde_json::json!("internal_panic"));
        assert!(stream_lines[1]["error"].as_str().is_some_and(|e| e.contains("simulated handler panic for R1-S10-03")));
        assert_eq!(stream_lines[2]["id"], serde_json::json!("ok-after"));
        assert_eq!(stream_lines[2]["ok"], serde_json::json!(true));
        assert_eq!(stream_lines[3]["result"]["quit"], serde_json::json!(true));

        let listener = match bind_control_tcp_listener(SocketAddr::from(([127, 0, 0, 1], 0))) {
            Err(err) if err.kind() == ErrorKind::PermissionDenied => return Ok(()),
            other => other?,
        };
        let addr = listener.local_addr()?;
        match TcpStream::connect_timeout(&addr, Duration::from_millis(250)) {
            Ok(probe) => {
                drop(probe);
                let _ = listener.accept();
            }
            Err(err) if matches!(err.kind(), ErrorKind::PermissionDenied | ErrorKind::TimedOut) => return Ok(()),
            Err(err) => return Err(Box::new(err)),
        }

        let token_owned = auth.as_str().to_owned();
        let client = std::thread::spawn(move || -> Result<Vec<serde_json::Value>, String> {
            let mut results = Vec::new();
            {
                let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let reader_stream = stream.try_clone().map_err(|e| e.to_string())?;
                let mut r = BufReader::new(reader_stream);
                let mut w = BufWriter::new(stream);
                for req in [
                    format!("{{\"id\":1,\"method\":\"auth\",\"params\":{{\"token\":\"{token_owned}\"}}}}\n"),
                    String::from("{\"id\":2,\"method\":\"test.panic\"}\n"),
                    String::from("{\"id\":3,\"method\":\"ping\"}\n"),
                ] {
                    w.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
                    w.flush().map_err(|e| e.to_string())?;
                    let mut line = String::new();
                    std::io::BufRead::read_line(&mut r, &mut line).map_err(|e| e.to_string())?;
                    results.push(serde_json::from_str(&line).map_err(|e| e.to_string())?);
                }
            }
            {
                let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let reader_stream = stream.try_clone().map_err(|e| e.to_string())?;
                let mut r = BufReader::new(reader_stream);
                let mut w = BufWriter::new(stream);
                for req in [
                    format!("{{\"id\":4,\"method\":\"ping\",\"token\":\"{token_owned}\"}}\n"),
                    format!("{{\"id\":5,\"method\":\"app.quit\",\"token\":\"{token_owned}\"}}\n"),
                ] {
                    w.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
                    w.flush().map_err(|e| e.to_string())?;
                    let mut line = String::new();
                    std::io::BufRead::read_line(&mut r, &mut line).map_err(|e| e.to_string())?;
                    results.push(serde_json::from_str(&line).map_err(|e| e.to_string())?);
                }
            }
            Ok(results)
        });

        let mut tcp_app = UiApp::new();
        serve_control_listener_with_handler(&mut tcp_app, &listener, &auth, |ui, req| -> (ControlResponse, ControlLoopAction) {
            if req.method == "test.panic" {
                panic!("tcp handler boom");
            }
            dispatch_request(ui, req)
        })?;
        let responses = client.join().map_err(|_| "client thread panicked")??;
        assert_eq!(responses.len(), 5);
        assert_eq!(responses[0]["ok"], serde_json::json!(true));
        assert_eq!(responses[1]["ok"], serde_json::json!(false));
        assert_eq!(responses[1]["error_detail"]["code"], serde_json::json!("internal_panic"));
        assert_eq!(responses[2]["ok"], serde_json::json!(true));
        assert_eq!(responses[3]["ok"], serde_json::json!(true));
        assert_eq!(responses[4]["result"]["quit"], serde_json::json!(true));
        Ok(())
    }

    #[test]
    fn r1_s10_04_idle_client_timeout_drops_only_stalled_connection() -> Result<(), Box<dyn std::error::Error>> {
        use std::io::{self, BufRead, Read};

        use super::{ControlAuth, dispatch_request, serve_control_stream_with_handler};
        use crate::UiApp;

        struct PartialThenTimeoutReader {
            step: usize,
        }

        impl Read for PartialThenTimeoutReader {
            fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::TimedOut, "simulated idle read timeout"))
            }
        }

        impl BufRead for PartialThenTimeoutReader {
            fn fill_buf(&mut self) -> io::Result<&[u8]> {
                match self.step {
                    0 => {
                        self.step = 1;
                        Ok(b"{\"id\":1")
                    }
                    _ => Err(io::Error::new(io::ErrorKind::TimedOut, "simulated idle read timeout")),
                }
            }

            fn consume(&mut self, _amt: usize) {}
        }

        let auth = ControlAuth::from_token("timeout-stream-secret")?;
        let mut app = UiApp::new();
        let mut reader = PartialThenTimeoutReader { step: 0 };
        let mut out = Vec::new();
        let quit = serve_control_stream_with_handler(&mut app, &mut reader, &mut out, &auth, dispatch_request)?;
        assert!(!quit, "idle client timeout must close only the stalled stream, not quit server");
        assert!(out.is_empty(), "partial unframed JSON must not produce a response");

        #[cfg(not(target_arch = "wasm32"))]
        {
            use std::io::{BufReader, BufWriter, ErrorKind, Write};
            use std::net::{SocketAddr, TcpStream};
            use std::time::Duration;

            use super::{bind_control_tcp_listener, serve_control_listener};

            let listener = match bind_control_tcp_listener(SocketAddr::from(([127, 0, 0, 1], 0))) {
                Err(err) if err.kind() == ErrorKind::PermissionDenied => return Ok(()),
                other => other?,
            };
            let addr = listener.local_addr()?;
            match TcpStream::connect_timeout(&addr, Duration::from_millis(250)) {
                Ok(probe) => {
                    drop(probe);
                    let _ = listener.accept();
                }
                Err(err) if matches!(err.kind(), ErrorKind::PermissionDenied | ErrorKind::TimedOut) => return Ok(()),
                Err(err) => return Err(Box::new(err)),
            }

            let server_auth = auth.clone();
            let server = std::thread::spawn(move || {
                let mut srv_app = UiApp::new();
                serve_control_listener(&mut srv_app, &listener, &server_auth)
            });

            let mut stalled = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
            stalled.write_all(b"{\"id\":1")?;
            stalled.flush()?;
            std::thread::sleep(Duration::from_millis(150));

            let client_b = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
            let _ = client_b.set_read_timeout(Some(Duration::from_secs(4)));
            let mut r = BufReader::new(client_b.try_clone()?);
            let mut w = BufWriter::new(client_b);
            let tok = auth.as_str();
            w.write_all(format!("{{\"id\":2,\"method\":\"ping\",\"token\":\"{tok}\"}}\n").as_bytes())?;
            w.flush()?;
            let mut reply = String::new();
            std::io::BufRead::read_line(&mut r, &mut reply)?;
            let parsed: serde_json::Value = serde_json::from_str(&reply)?;
            assert_eq!(parsed["ok"], serde_json::json!(true), "second client should succeed once stalled client times out: {parsed}");

            w.write_all(format!("{{\"id\":3,\"method\":\"app.quit\",\"token\":\"{tok}\"}}\n").as_bytes())?;
            w.flush()?;
            drop(stalled);
            let server_res = server.join().map_err(|_| "server thread panicked")?;
            assert!(server_res.is_ok(), "server should exit cleanly after app.quit: {server_res:?}");
        }

        Ok(())
    }

    #[test]
    fn r1_s10_05_batch_step_limit_and_max_frame_size_enforced() -> Result<(), Box<dyn std::error::Error>> {
        use super::{ControlResponse, MAX_CONTROL_BATCH_STEPS, MAX_CONTROL_LINE_BYTES, handle_control_line, serialize_response};
        use crate::UiApp;

        let mut app = UiApp::new();
        let initial_rulers = app.canvas_dialogs.rulers_visible;

        // 1. Reject batches exceeding MAX_CONTROL_BATCH_STEPS immediately without executing/mutating.
        let mut too_many = vec![serde_json::json!({"command": "ui.rulers.toggle"})];
        too_many.extend((0..MAX_CONTROL_BATCH_STEPS).map(|_| serde_json::json!({"command": "ui.zoom.fitSheet"})));
        assert_eq!(too_many.len(), MAX_CONTROL_BATCH_STEPS + 1);
        let req = serde_json::json!({
            "id": "too-many",
            "method": "command.batch",
            "params": { "atomic": true, "commands": too_many }
        })
        .to_string();
        let raw_resp = handle_control_line(&mut app, &req);
        let parsed: serde_json::Value = serde_json::from_str(&raw_resp)?;
        assert_eq!(parsed["id"], serde_json::json!("too-many"));
        assert_eq!(parsed["ok"], serde_json::json!(false));
        assert!(parsed["error"].as_str().is_some_and(|e| e.contains("maximum step limit")));
        assert_eq!(app.canvas_dialogs.rulers_visible, initial_rulers);

        // 2. Reject cumulative batch payloads exceeding MAX_CONTROL_LINE_BYTES and preserve atomic rollback.
        let mut big_csv = String::from("Label\n");
        let row_text = "value-0123456789abcdef-0123456789abcdef\n";
        for _ in 0..1_200 {
            big_csv.push_str(row_text);
        }
        app.execute_command("table.importCsv", serde_json::json!({ "sheet_id": "d-big", "archetype": "column", "csv": big_csv }))?;
        let mut overflowing_cmds = vec![serde_json::json!({"command": "ui.rulers.toggle"})];
        overflowing_cmds.extend((0..500).map(|_| serde_json::json!({"command": "table.exportCsv", "params": {"sheet_id": "d-big"}})));
        let overflow_req = serde_json::json!({
            "id": "overflow-batch",
            "method": "command.batch",
            "params": { "atomic": true, "commands": overflowing_cmds }
        })
        .to_string();
        let overflow_resp = handle_control_line(&mut app, &overflow_req);
        assert!(overflow_resp.len() < MAX_CONTROL_LINE_BYTES);
        let overflow_parsed: serde_json::Value = serde_json::from_str(&overflow_resp)?;
        assert_eq!(overflow_parsed["id"], serde_json::json!("overflow-batch"));
        assert_eq!(overflow_parsed["ok"], serde_json::json!(false));
        assert!(overflow_parsed["error"].as_str().is_some_and(|e| e.contains("maximum control line size")));
        assert_eq!(app.canvas_dialogs.rulers_visible, initial_rulers, "atomic batch must roll back before returning frame overflow");

        // 3. Enforce MAX_CONTROL_LINE_BYTES directly in serialize_response, preserving bounded ID.
        let oversized = ControlResponse::ok(serde_json::json!("big-frame"), serde_json::json!({ "blob": "x".repeat(MAX_CONTROL_LINE_BYTES) }));
        let bounded = serialize_response(&oversized);
        assert!(bounded.len() < MAX_CONTROL_LINE_BYTES);
        let bounded_val: serde_json::Value = serde_json::from_str(&bounded)?;
        assert_eq!(bounded_val["id"], serde_json::json!("big-frame"));
        assert_eq!(bounded_val["ok"], serde_json::json!(false));
        assert!(bounded_val["error"].as_str().is_some_and(|e| e.contains("exceeds maximum control line size")));

        Ok(())
    }

    #[test]
    fn r1_s10_06_atomic_batch_rollback_preserves_multi_entry_undo_and_clean_state() -> Result<(), Box<dyn std::error::Error>> {
        use super::{ControlRequest, dispatch_request_with_hook, handle_control_line};
        use crate::UiApp;

        let mut app = UiApp::new();
        app.execute_command("sheet.createData", serde_json::json!({ "sheet_id": "ds-base", "title": "Base", "cols": 2, "rows": 10 }))?;
        for step in 0..12 {
            app.execute_command(
                "table.setCells",
                serde_json::json!({
                    "sheet_id": "ds-base",
                    "edits": [{ "row": 0, "col": 0, "value": f64::from(step + 1) }]
                }),
            )?;
        }
        // Mark session saved/clean at undo_depth = 13 while retaining full prior undo stack.
        app.session_mut().set_file_path(Some(std::path::PathBuf::from("project.curvebook")), true);
        let baseline_undo_depth = app.session().undo_depth();
        assert!(baseline_undo_depth >= 13);
        assert!(!app.session().is_dirty());
        let baseline_doc = app.document().clone();
        let baseline_locale = app.locale.clone();
        let baseline_rulers = app.canvas_dialogs.rulers_visible;

        // 1. Read-only atomic batch succeeds without mutating dirty/undo state.
        let ro_req = ControlRequest {
            id: serde_json::json!("ro-1"),
            method: String::from("command.batch"),
            params: serde_json::json!({
                "atomic": true,
                "commands": [
                    { "command": "project.inspect", "params": {} },
                    { "command": "table.exportCsv", "params": { "sheet_id": "ds-base" } }
                ]
            }),
        };
        let mut ro_rolled_back = true;
        let (ro_resp, _) = dispatch_request_with_hook(&mut app, ro_req, &mut |_app_ref, ctx| {
            ro_rolled_back = ctx.rolled_back;
            Ok(())
        });
        assert!(ro_resp.ok, "unexpected ro_resp failure: {:?}", ro_resp.error);
        assert!(!ro_rolled_back);
        assert_eq!(app.session().undo_depth(), baseline_undo_depth);
        assert!(!app.session().is_dirty());

        // 2. Mixed document + UI atomic batch rolls back via session undo without dropping prior undo history.
        let fail_req = ControlRequest {
            id: serde_json::json!("fail-1"),
            method: String::from("command.batch"),
            params: serde_json::json!({
                "atomic": true,
                "commands": [
                    { "command": "sheet.createData", "params": { "sheet_id": "temp-fail", "title": "Temp" } },
                    { "command": "table.setCells", "params": { "sheet_id": "ds-base", "edits": [{ "row": 0, "col": 0, "value": 999.0 }] } },
                    { "command": "ui.locale.set", "params": { "locale": "de" } },
                    { "command": "ui.rulers.toggle", "params": { "visible": !baseline_rulers } },
                    { "command": "nonexistent.command", "params": {} }
                ]
            }),
        };
        let mut saw_rolled_back = false;
        let mut saw_steps = usize::MAX;
        let (fail_resp, _) = dispatch_request_with_hook(&mut app, fail_req, &mut |_app_ref, ctx| {
            saw_rolled_back = ctx.rolled_back;
            saw_steps = ctx.steps.len();
            Ok(())
        });
        assert!(!fail_resp.ok);
        assert!(saw_rolled_back, "hook must observe rolled_back=true so WAL ignores undo-driven redo depth change");
        assert_eq!(saw_steps, 0);
        assert_eq!(app.document(), &baseline_doc);
        assert_eq!(app.session().undo_depth(), baseline_undo_depth);
        assert!(!app.session().is_dirty());
        assert_eq!(app.session().file_path(), Some(std::path::Path::new("project.curvebook")));
        assert_eq!(app.locale, baseline_locale);
        assert_eq!(app.canvas_dialogs.rulers_visible, baseline_rulers);

        // 3. Atomic rollback when session was already dirty + batch undoes pre-batch entry then mutates forward.
        app.execute_command("table.setCells", serde_json::json!({ "sheet_id": "ds-base", "edits": [{ "row": 1, "col": 0, "value": 42.0 }] }))?;
        assert!(app.session().is_dirty());
        let dirty_doc = app.document().clone();
        let dirty_undo = app.session().undo_depth();
        let undo_then_mutate_req = serde_json::json!({
            "id": "fail-undo-mutate",
            "method": "command.batch",
            "params": {
                "atomic": true,
                "commands": [
                    { "command": "edit.undo", "params": {} },
                    { "command": "sheet.createData", "params": { "sheet_id": "temp-2", "title": "Temp2" } },
                    { "command": "nonexistent.command", "params": {} }
                ]
            }
        })
        .to_string();
        let undo_mutate_resp: serde_json::Value = serde_json::from_str(&handle_control_line(&mut app, &undo_then_mutate_req))?;
        assert_eq!(undo_mutate_resp["ok"], serde_json::json!(false));
        assert_eq!(app.document(), &dirty_doc);
        assert_eq!(app.session().undo_depth(), dirty_undo);
        assert!(app.session().is_dirty());

        Ok(())
    }

    #[test]
    fn r1_s10_08_strict_ui_parameter_validation_rejects_wrong_types() -> Result<(), Box<dyn std::error::Error>> {
        use super::handle_control_line;
        use crate::{ActiveDialog, UiApp};

        let mut app = UiApp::new();
        app.execute_command("sheet.createData", serde_json::json!({ "sheet_id": "ds-1", "title": "Data", "cols": 2, "rows": 4 }))?;
        app.execute_command(
            "table.setCells",
            serde_json::json!({
                "sheet_id": "ds-1",
                "edits": [
                    { "row": 0, "col": 0, "value": 1.0 },
                    { "row": 1, "col": 0, "value": 2.0 },
                    { "row": 2, "col": 0, "value": 3.0 },
                    { "row": 0, "col": 1, "value": 4.0 },
                    { "row": 1, "col": 1, "value": 5.0 },
                    { "row": 2, "col": 1, "value": 7.0 }
                ]
            }),
        )?;
        let g = app.execute_command("graph.create", serde_json::json!({ "data_sheet_id": "ds-1", "kind": "column", "title": "G1" }))?;
        let graph_id = g.payload["sheet_id"].as_str().ok_or("missing graph_id")?.to_owned();
        let an = app.execute_command(
            "analysis.run",
            serde_json::json!({
                "source_sheet_id": "ds-1",
                "kind": {
                    "DescriptiveStats": {
                        "aggregation": "RowMeans",
                        "custom_upper_percentile": null,
                        "run_normality_battery": true,
                        "hypothetical_value": null
                    }
                }
            }),
        )?;
        let res_id = an.payload["sheet_id"].as_str().ok_or("missing result_id")?.to_owned();

        // Set recognizable shortcut + ruler + split state that wrong-typed params must NOT clobber.
        app.shortcuts.set_shortcut("ui.gallery.toggle", "Alt+Shift+G")?;
        let custom_shortcut = app.shortcuts.active_combo("ui.gallery.toggle").ok_or("missing shortcut")?.to_owned();
        let initial_rulers = app.canvas_dialogs.rulers_visible;
        let initial_split = app.split.enabled;

        let bad_cases = [
            serde_json::json!({"id": 1, "method": "ui.rulers.toggle", "params": {"visible": "false"}}),
            serde_json::json!({"id": 2, "method": "ui.rulers.toggle", "params": {"unit": 123}}),
            serde_json::json!({"id": 3, "method": "ui.shortcut.reset", "params": {"command_id": 123}}),
            serde_json::json!({"id": 4, "method": "ui.shortcut.reset", "params": {"command_id": ""}}),
            serde_json::json!({"id": 5, "method": "ui.shortcut.set", "params": {"command_id": "ui.gallery.toggle", "shortcut": 99}}),
            serde_json::json!({"id": 6, "method": "ui.gridOverlay.toggle", "params": {"visible": "no"}}),
            serde_json::json!({"id": 7, "method": "ui.split.toggle", "params": {"enabled": "no"}}),
            serde_json::json!({"id": 8, "method": "ui.split.toggle", "params": {"ratio": "half"}}),
            serde_json::json!({"id": 9, "method": "ui.result.inspectDiagnostics", "params": {"result_sheet_id": res_id, "tab_index": "foo"}}),
            serde_json::json!({"id": 10, "method": "ui.graph.axes.open", "params": {"sheet_id": graph_id, "target_axis": 123}}),
            serde_json::json!({"id": 11, "method": "ui.graph.axes.open", "params": {"sheet_id": graph_id, "target_axis": "z"}}),
            serde_json::json!({"id": 12, "method": "ui.print.open", "params": {"printer": 123}}),
            serde_json::json!({"id": 13, "method": "ui.print.open", "params": {"copies": "two"}}),
            serde_json::json!({"id": 14, "method": "ui.formatPoints.open", "params": {"sheet_id": graph_id, "points": [{"series_index": "foo", "point_index": 0}]}}),
            serde_json::json!({"id": 15, "method": "ui.formatPoints.open", "params": {"sheet_id": graph_id, "points": "bad"}}),
            serde_json::json!({"id": 16, "method": "ui.navigator.toggle", "params": "wrong-type"}),
            serde_json::json!({"id": 17, "method": "ui.invoke_menu", "params": {"command_id": 123, "path": "File"}}),
            serde_json::json!({"id": 18, "method": "ui.invoke_menu", "params": {"command_id": "ui.gallery.toggle", "params": "not-an-object"}}),
            serde_json::json!({"id": 19, "method": "ui.inspect", "params": {"include_bounds": "yes"}}),
            serde_json::json!({"id": 20, "method": "command.batch", "params": {"atomic": "yes", "commands": []}}),
        ];

        for req in bad_cases {
            let reply_str = handle_control_line(&mut app, &req.to_string());
            let parsed: serde_json::Value = serde_json::from_str(&reply_str)?;
            assert_eq!(parsed["ok"], serde_json::json!(false), "expected validation failure for request {req}: {reply_str}");
        }

        // Verify none of the rejected requests silently triggered defaults or reset_all().
        assert_eq!(app.shortcuts.active_combo("ui.gallery.toggle"), Some(custom_shortcut.as_str()));
        assert_eq!(app.canvas_dialogs.rulers_visible, initial_rulers);
        assert_eq!(app.split.enabled, initial_split);

        // Open FormatSelectedPoints dialog and reject invalid ui.answer_dialog action/fields types.
        let open_reply: serde_json::Value = serde_json::from_str(&handle_control_line(
            &mut app,
            &serde_json::json!({
                "id": "open-fp",
                "method": "ui.formatPoints.open",
                "params": { "sheet_id": graph_id, "points": [{ "series_index": 0, "point_index": 0 }] }
            })
            .to_string(),
        ))?;
        assert_eq!(open_reply["ok"], serde_json::json!(true));

        let bad_answer_cases = [
            serde_json::json!({"id": "ans1", "method": "ui.answer_dialog", "params": {"action": 123}}),
            serde_json::json!({"id": "ans2", "method": "ui.answer_dialog", "params": {"fields": "bad", "action": "cancel"}}),
            serde_json::json!({"id": "ans3", "method": "ui.answer_dialog", "params": {"fields": {"size_pt": "huge"}}}),
            serde_json::json!({"id": "ans4", "method": "ui.answer_dialog", "params": {"fields": {"points": [{"series_index": "bad", "point_index": 1}]}}}),
        ];
        for req in bad_answer_cases {
            let parsed: serde_json::Value = serde_json::from_str(&handle_control_line(&mut app, &req.to_string()))?;
            assert_eq!(parsed["ok"], serde_json::json!(false), "expected ui.answer_dialog rejection for {req}: {parsed}");
        }
        assert_ne!(app.canvas_dialogs.active_dialog, ActiveDialog::None, "invalid ui.answer_dialog must not commit/close dialog");

        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn doc_export_supports_emf_tsv_pdf_version_ttf_font_direct_path_and_gui_only_rejection() -> Result<(), Box<dyn std::error::Error>> {
        use super::handle_control_line;
        use crate::UiApp;

        let mut app = UiApp::new();
        app.execute_command("sheet.createData", serde_json::json!({ "sheet_id": "ds-exp", "title": "ExportData", "cols": 2, "rows": 3 }))?;
        app.execute_command(
            "table.setCells",
            serde_json::json!({
                "sheet_id": "ds-exp",
                "edits": [
                    { "row": 0, "col": 0, "value": 1.0 },
                    { "row": 1, "col": 0, "value": 2.0 },
                    { "row": 0, "col": 1, "value": 10.0 },
                    { "row": 1, "col": 1, "value": 20.0 }
                ]
            }),
        )?;
        let g = app.execute_command("graph.create", serde_json::json!({ "data_sheet_id": "ds-exp", "kind": "xy", "title": "ExportGraph" }))?;
        let graph_id = g.payload["sheet_id"].as_str().ok_or("missing graph_id")?.to_owned();

        let dir = std::env::temp_dir().join(format!("cb_control_doc_export_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;

        // 1. Inline TSV uses tab delimiter; csv_delimiter overrides it.
        let tsv_reply: serde_json::Value = serde_json::from_str(&handle_control_line(
            &mut app,
            &serde_json::json!({ "id": "tsv-1", "method": "doc.export", "params": { "format": "tsv", "sheet_id": "ds-exp" } }).to_string(),
        ))?;
        assert_eq!(tsv_reply["ok"], serde_json::json!(true), "tsv export failed: {tsv_reply}");
        let tsv_b64 = tsv_reply["result"]["data_base64"].as_str().ok_or("missing tsv base64")?;
        let tsv_text = String::from_utf8(curvebook_platform::base64::decode(tsv_b64)?)?;
        assert!(tsv_text.contains('\t'), "expected tab delimiter in TSV export: {tsv_text:?}");

        let semi_reply: serde_json::Value = serde_json::from_str(&handle_control_line(
            &mut app,
            &serde_json::json!({ "id": "tsv-2", "method": "doc.export", "params": { "format": "tsv", "sheet_id": "ds-exp", "csv_delimiter": ";" } })
                .to_string(),
        ))?;
        assert_eq!(semi_reply["ok"], serde_json::json!(true));
        let semi_text = String::from_utf8(curvebook_platform::base64::decode(semi_reply["result"]["data_base64"].as_str().ok_or("missing b64")?)?)?;
        assert!(semi_text.contains(';'), "expected explicit delimiter override: {semi_text:?}");

        // 2. Direct-to-disk EMF export creates parent dir and omits data_base64.
        let emf_path = dir.join("nested").join("figure.emf");
        let emf_reply: serde_json::Value = serde_json::from_str(&handle_control_line(
            &mut app,
            &serde_json::json!({
                "id": "emf-1",
                "method": "doc.export",
                "params": { "format": "emf", "sheet_id": graph_id, "path": emf_path.display().to_string() }
            })
            .to_string(),
        ))?;
        assert_eq!(emf_reply["ok"], serde_json::json!(true), "emf export failed: {emf_reply}");
        assert_eq!(emf_reply["result"]["exported"], serde_json::json!(true));
        assert!(emf_reply["result"].get("data_base64").is_none(), "direct path export must omit data_base64: {emf_reply}");
        let emf_bytes = std::fs::read(&emf_path)?;
        assert_eq!(emf_reply["result"]["bytes_written"], serde_json::json!(emf_bytes.len()));
        assert!(emf_bytes.len() > 44 && emf_bytes.get(40..44) == Some(b" EMF"), "expected valid EMF signature");

        // 3. PDF 1.4 with embedded TrueType font via ttf_font writes directly to disk.
        let ttf_path = dir.join("minimal.ttf");
        std::fs::write(&ttf_path, curvebook_text::build_minimal_truetype_font())?;
        let pdf_path = dir.join("figure_v14.pdf");
        let pdf_reply: serde_json::Value = serde_json::from_str(&handle_control_line(
            &mut app,
            &serde_json::json!({
                "id": "pdf-1",
                "method": "doc.export",
                "params": {
                    "format": "pdf",
                    "sheet_id": graph_id,
                    "pdf_version": "1.4",
                    "ttf_font": ttf_path.display().to_string(),
                    "path": pdf_path.display().to_string()
                }
            })
            .to_string(),
        ))?;
        assert_eq!(pdf_reply["ok"], serde_json::json!(true), "pdf export failed: {pdf_reply}");
        assert_eq!(pdf_reply["result"]["exported"], serde_json::json!(true));
        assert!(pdf_reply["result"].get("data_base64").is_none());
        let pdf_bytes = std::fs::read(&pdf_path)?;
        assert!(pdf_bytes.starts_with(b"%PDF-1.4\n"), "expected PDF 1.4 header");
        let pdf_latin1: String = pdf_bytes.iter().map(|&b| char::from(b)).collect();
        assert!(pdf_latin1.contains("/Subtype /TrueType") && pdf_latin1.contains("/FontFile2"), "expected embedded TrueType font in PDF");

        // 4. Invalid ttf_font file is rejected cleanly.
        let bad_ttf = dir.join("not_a_font.ttf");
        std::fs::write(&bad_ttf, b"not a valid sfnt binary")?;
        let bad_font_reply: serde_json::Value = serde_json::from_str(&handle_control_line(
            &mut app,
            &serde_json::json!({
                "id": "pdf-bad-font",
                "method": "doc.export",
                "params": { "format": "pdf", "sheet_id": graph_id, "ttf_font": bad_ttf.display().to_string() }
            })
            .to_string(),
        ))?;
        assert_eq!(bad_font_reply["ok"], serde_json::json!(false));

        // 5. Under --gui-only, doc.export with path is blocked with gui_only_violation and does not touch disk.
        let blocked_path = dir.join("blocked_gui_only.svg");
        app.set_gui_only(true);
        let gui_only_reply: serde_json::Value = serde_json::from_str(&handle_control_line(
            &mut app,
            &serde_json::json!({
                "id": "gui-only-export",
                "method": "doc.export",
                "params": { "format": "svg", "sheet_id": graph_id, "path": blocked_path.display().to_string() }
            })
            .to_string(),
        ))?;
        assert_eq!(gui_only_reply["ok"], serde_json::json!(false));
        assert_eq!(gui_only_reply["error_detail"]["code"], serde_json::json!("gui_only_violation"));
        assert!(!blocked_path.exists(), "gui-only blocked doc.export must not create file on disk");

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
}
