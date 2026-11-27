//! Shared integration test helpers for `curvebook` desktop suites.
#![forbid(unsafe_code)]
#![allow(dead_code)]

use std::collections::VecDeque;
use std::fs;
use std::io::{self, BufRead, BufReader, BufWriter, Cursor, ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use curvebook::desktop_host::{DesktopHostConfig, DesktopHostRuntime, DesktopWindowId, DesktopWindowSession};
use curvebook::gui_window::{CurveBookNativeApp, LiveControlChannel, install_desktop_font_definitions};
use curvebook::live_control::{LiveControlBridge, LiveControlServerGuard, serve_live_control_io, spawn_live_control_accept_loop};
use curvebook_doc::{Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, SubcolumnMode, TableArchetype, XyTable};
use curvebook_engine::Session;
use curvebook_format::zip::ZipWriter;
use curvebook_ui_egui::control_io::CONTROL_CLIENT_IDLE_TIMEOUT;
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::{ControlAuth, bind_control_tcp_listener};
use eframe::egui;
use serde_json::Value;

static TEMP_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

/// RAII temporary directory removed automatically on drop.
#[derive(Debug)]
pub struct TempDirGuard {
    pub path: PathBuf,
}

impl TempDirGuard {
    /// Create a fresh uniquely-named directory under `std::env::temp_dir()`.
    pub fn new(prefix: &str) -> io::Result<Self> {
        let seq = TEMP_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!("cb-{prefix}-{}-{seq}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    /// Borrow the directory path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Build a child path inside the temporary directory.
    #[must_use]
    pub fn child(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Generate a unique path under `std::env::temp_dir()`.
#[must_use]
pub fn unique_temp_path(prefix: &str, name: &str) -> PathBuf {
    let seq = TEMP_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    std::env::temp_dir().join(format!("cb-{prefix}-{}-{seq}-{nanos}-{name}", std::process::id()))
}

/// Encode bytes as standard Base64 via `curvebook_platform::base64`.
#[must_use]
pub fn encode_base64(input: &[u8]) -> String {
    curvebook_platform::base64::encode(input)
}

/// Decode standard Base64 via `curvebook_platform::base64`.
pub fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    curvebook_platform::base64::decode(input).map_err(|err| err.to_string())
}

/// Parse `(width, height)` from a PNG IHDR header.
#[must_use]
pub fn png_dimensions(png: &[u8]) -> Option<(u32, u32)> {
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    let chunk = png.get(12..24)?;
    let (ihdr, dims) = chunk.split_at_checked(4)?;
    if ihdr != b"IHDR" {
        return None;
    }
    let (w_bytes, h_bytes) = dims.split_at_checked(4)?;
    let w = u32::from_be_bytes(w_bytes.try_into().ok()?);
    let h = u32::from_be_bytes(h_bytes.try_into().ok()?);
    Some((w, h))
}

// ---------------------------------------------------------------------------
// Headless `egui` + `CurveBookNativeApp` harness
// ---------------------------------------------------------------------------

/// Build an `egui::Context` configured with CurveBook's bundled font definitions.
#[must_use]
pub fn new_egui_context() -> egui::Context {
    let ctx = egui::Context::default();
    install_desktop_font_definitions(&ctx, None);
    ctx
}

/// Headless driver for [`CurveBookNativeApp`] frames and synthetic input events.
pub struct NativeHarness {
    pub ctx: egui::Context,
    pub app: CurveBookNativeApp,
    pub screen: egui::Vec2,
    pub scale: Option<f32>,
    pub time_s: f64,
}

impl NativeHarness {
    /// Wrap an existing [`CurveBookNativeApp`] with default `1280×800` viewport.
    #[must_use]
    pub fn new(app: CurveBookNativeApp) -> Self {
        Self::with_screen(app, egui::vec2(1280.0, 800.0))
    }

    /// Wrap an existing [`CurveBookNativeApp`] with an explicit logical screen size.
    #[must_use]
    pub fn with_screen(app: CurveBookNativeApp, screen: egui::Vec2) -> Self {
        Self { ctx: new_egui_context(), app, screen, scale: None, time_s: 0.0 }
    }

    /// Pin an explicit `native_pixels_per_point` scale factor on the harness context and inputs.
    #[must_use]
    pub fn with_scale(mut self, scale: f32) -> Self {
        self.scale = Some(scale);
        self.ctx.set_pixels_per_point(scale);
        self
    }

    /// Build a headless harness backed by a fresh [`DesktopHostRuntime`] and optional [`Session`].
    pub fn from_config(cfg: DesktopHostConfig, session: Option<Session>) -> Result<(Self, DesktopWindowId), String> {
        let screen = egui::vec2(cfg.initial_viewport.logical_width as f32, cfg.initial_viewport.logical_height as f32);
        let mut runtime = DesktopHostRuntime::new(cfg.clone());
        let window_id = match session {
            Some(s) => runtime.open_window_with_session(s, cfg.initial_viewport, false)?,
            None => runtime.open_window(None, cfg.initial_viewport)?,
        };
        let app = CurveBookNativeApp::new(runtime, window_id, None);
        Ok((Self::with_screen(app, screen), window_id))
    }

    /// Step one frame with an explicit [`egui::RawInput`].
    pub fn step_raw(&mut self, mut raw: egui::RawInput) -> egui::FullOutput {
        if raw.screen_rect.is_none() {
            raw.screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.screen));
        }
        if let Some(scale) = self.scale {
            raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(scale);
        }
        if raw.time.is_none() {
            let ctx_time = self.ctx.input(|i| i.time);
            self.time_s = self.time_s.max(ctx_time) + 0.016;
            raw.time = Some(self.time_s);
        }
        self.app.drain_synthetic_events_into(&mut raw);
        let mut out = self.ctx.run_ui(raw, |ui| self.app.step_ui(ui));
        out.textures_delta.clear();
        out
    }

    /// Step one frame with the given `egui::Event` list.
    pub fn step_events(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.step_raw(egui::RawInput { events, ..Default::default() })
    }

    /// Step one frame and return emitted root [`egui::ViewportCommand`]s.
    pub fn step_cmds(&mut self, events: Vec<egui::Event>) -> Vec<egui::ViewportCommand> {
        let out = self.step_events(events);
        out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default()
    }

    /// Step `count` idle frames to settle layout/repaint state, returning the last frame's commands.
    pub fn settle(&mut self, count: usize) -> Vec<egui::ViewportCommand> {
        let mut last = Vec::new();
        for _ in 0..count {
            last = self.step_cmds(Vec::new());
        }
        last
    }

    /// Click a logical position (move + primary press/release, then settle) and return viewport commands.
    pub fn click_pos(&mut self, pos: egui::Pos2) -> Vec<egui::ViewportCommand> {
        let mut cmds = self.step_cmds(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ]);
        cmds.extend(self.settle(3));
        cmds
    }

    /// Double-click a logical position (move + two press/release cycles, then 1 settle frame).
    pub fn double_click_pos(&mut self, pos: egui::Pos2) {
        let _ = self.step_events(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ]);
        let _ = self.step_events(Vec::new());
    }

    /// Click the center of a rectangle.
    pub fn click_rect(&mut self, rect: egui::Rect) -> Vec<egui::ViewportCommand> {
        self.click_pos(rect.center())
    }

    /// Look up `widget_id` in `inspect_meta` and click its center.
    pub fn click_widget(&mut self, widget_id: &str) -> Result<Vec<egui::ViewportCommand>, String> {
        let rect =
            self.app.inspect_meta().find_by_widget_id(widget_id).map(|n| n.clipped_rect).ok_or_else(|| format!("missing widget `{widget_id}`"))?;
        Ok(self.click_rect(rect))
    }

    /// Open an in-window top menu, optionally hover a submenu, and click an item.
    pub fn open_menu_item(&mut self, top_menu_id: &str, submenu_id: Option<&str>, item_id: &str) -> Result<(), String> {
        let _ = self.click_widget(top_menu_id)?;
        if let Some(sub) = submenu_id {
            let center = self
                .app
                .inspect_meta()
                .find_by_widget_id(sub)
                .map(|n| n.clipped_rect.center())
                .ok_or_else(|| format!("missing submenu `{sub}` under `{top_menu_id}`"))?;
            for _ in 0..3 {
                let _ = self.step_events(vec![egui::Event::PointerMoved(center)]);
            }
        }
        let _ = self.click_widget(item_id)?;
        Ok(())
    }

    /// Move pointer to `pos`, dispatch a `MouseWheel` event, and settle one frame.
    pub fn send_wheel(&mut self, pos: egui::Pos2, delta: egui::Vec2, modifiers: egui::Modifiers) {
        let _ = self.step_events(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta,
                modifiers,
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
            },
        ]);
        let _ = self.step_events(Vec::new());
    }

    /// Drag primary pointer from `from` through midpoint to `to` and release.
    pub fn drag_between(&mut self, from: egui::Pos2, to: egui::Pos2) {
        let mid = from.lerp(to, 0.5);
        let _ = self.step_events(vec![
            egui::Event::PointerMoved(from),
            egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        ]);
        let _ = self.step_events(vec![egui::Event::PointerMoved(mid)]);
        let _ = self.step_events(vec![egui::Event::PointerMoved(to)]);
        let _ = self.step_events(vec![egui::Event::PointerButton {
            pos: to,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        let _ = self.step_events(Vec::new());
    }

    /// Send a single key-down event + settle and return collected viewport commands.
    pub fn key_down(&mut self, key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::ViewportCommand> {
        let mut cmds = self.step_cmds(vec![egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }]);
        cmds.extend(self.settle(2));
        cmds
    }

    /// Send a single key press + release.
    pub fn press_key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        let _ = self.step_events(vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }]);
        let _ = self.step_events(vec![egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers }]);
    }
}

/// Open a single window on a new [`DesktopHostRuntime`] with standard viewport sizing.
pub fn open_host_window(
    in_window_menus: bool,
    new_instance: bool,
    session: Option<Session>,
    width: f64,
    height: f64,
) -> Result<(DesktopHostRuntime, DesktopWindowId), String> {
    let vp = ViewportMetrics::new(width, height, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus,
        new_instance,
        recovery_root: None,
        initial_viewport: vp,
        ..DesktopHostConfig::default()
    });
    let wid = match session {
        Some(s) => rt.open_window_with_session(s, vp, false)?,
        None => rt.open_window(None, vp)?,
    };
    Ok((rt, wid))
}

/// Current wall-clock timestamp in milliseconds since Unix epoch.
#[must_use]
pub fn now_epoch_ms() -> u64 {
    u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis())).unwrap_or(u64::MAX)
}

/// Parse all non-empty JSON lines from a byte slice.
pub fn read_jsonl_values(bytes: &[u8]) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let mut out = Vec::new();
    for line in BufReader::new(Cursor::new(bytes)).lines() {
        let line = line?;
        if !line.trim().is_empty() {
            out.push(serde_json::from_str(&line)?);
        }
    }
    Ok(out)
}

/// Feed a slice of JSON-line requests through `DesktopWindowSession::serve_control_stream_with_quit`.
pub fn send_control_lines(
    win: &mut DesktopWindowSession,
    auth: &ControlAuth,
    reqs: &[Value],
) -> Result<(Vec<Value>, bool), Box<dyn std::error::Error>> {
    let mut input = String::new();
    for req in reqs {
        input.push_str(&serde_json::to_string(req)?);
        input.push('\n');
    }
    let mut reader = BufReader::new(Cursor::new(input.into_bytes()));
    let mut out = Vec::new();
    let quit = win.serve_control_stream_with_quit(&mut reader, &mut out, auth)?;
    Ok((read_jsonl_values(&out)?, quit))
}

// ---------------------------------------------------------------------------
// In-memory pipe & interactive `LiveControlBridge` client
// ---------------------------------------------------------------------------

/// `io::Read` adapter backed by an `mpsc::Receiver<Vec<u8>>`.
pub struct ChannelPipeReader {
    rx: mpsc::Receiver<Vec<u8>>,
    buf: VecDeque<u8>,
    poll_timeout: Option<Duration>,
}

impl ChannelPipeReader {
    /// Create a blocking reader (returns EOF on disconnect).
    #[must_use]
    pub const fn blocking(rx: mpsc::Receiver<Vec<u8>>) -> Self {
        Self { rx, buf: VecDeque::new(), poll_timeout: None }
    }

    /// Create a polling reader returning `WouldBlock` on `poll_timeout`.
    #[must_use]
    pub const fn with_timeout(rx: mpsc::Receiver<Vec<u8>>, poll_timeout: Duration) -> Self {
        Self { rx, buf: VecDeque::new(), poll_timeout: Some(poll_timeout) }
    }
}

impl Read for ChannelPipeReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        while self.buf.is_empty() {
            if let Some(timeout) = self.poll_timeout {
                match self.rx.recv_timeout(timeout) {
                    Ok(chunk) if chunk.is_empty() => continue,
                    Ok(chunk) => self.buf.extend(chunk),
                    Err(mpsc::RecvTimeoutError::Timeout) => return Err(io::Error::from(ErrorKind::WouldBlock)),
                    Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(0),
                }
            } else {
                match self.rx.recv() {
                    Ok(chunk) if chunk.is_empty() => continue,
                    Ok(chunk) => self.buf.extend(chunk),
                    Err(_) => return Ok(0),
                }
            }
        }
        let mut n = 0usize;
        for dst in out.iter_mut() {
            let Some(b) = self.buf.pop_front() else {
                break;
            };
            *dst = b;
            n += 1;
        }
        Ok(n)
    }
}

/// `io::Write` adapter backed by an `mpsc::Sender<Vec<u8>>`.
pub struct ChannelPipeWriter {
    tx: mpsc::Sender<Vec<u8>>,
}

impl ChannelPipeWriter {
    #[must_use]
    pub const fn new(tx: mpsc::Sender<Vec<u8>>) -> Self {
        Self { tx }
    }
}

impl Write for ChannelPipeWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.tx.send(data.to_vec()).map_err(|_| io::Error::new(ErrorKind::BrokenPipe, "channel closed"))?;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Synchronous JSON-line client for an in-memory `serve_live_control_io` thread.
pub struct InteractiveControlClient {
    reader: BufReader<ChannelPipeReader>,
    writer: ChannelPipeWriter,
}

impl InteractiveControlClient {
    /// Construct a paired `(client, server_reader, server_writer)` using in-memory channels.
    #[must_use]
    pub fn pair(poll_timeout: Option<Duration>) -> (Self, BufReader<ChannelPipeReader>, ChannelPipeWriter) {
        let (c2s_tx, c2s_rx) = mpsc::channel();
        let (s2c_tx, s2c_rx) = mpsc::channel();
        let client_reader = match poll_timeout {
            Some(t) => ChannelPipeReader::with_timeout(s2c_rx, t),
            None => ChannelPipeReader::blocking(s2c_rx),
        };
        let server_reader = match poll_timeout {
            Some(t) => ChannelPipeReader::with_timeout(c2s_rx, t),
            None => ChannelPipeReader::blocking(c2s_rx),
        };
        (
            Self { reader: BufReader::new(client_reader), writer: ChannelPipeWriter::new(c2s_tx) },
            BufReader::new(server_reader),
            ChannelPipeWriter::new(s2c_tx),
        )
    }

    /// Send one JSON request line and wait up to 10s for the JSON reply line.
    pub fn send(&mut self, payload: &Value) -> Result<Value, String> {
        let mut raw = serde_json::to_vec(payload).map_err(|e| e.to_string())?;
        raw.push(b'\n');
        self.writer.write_all(&raw).map_err(|e| e.to_string())?;
        self.writer.flush().map_err(|e| e.to_string())?;

        let mut reply_line = String::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match self.reader.read_line(&mut reply_line) {
                Ok(0) => return Err(String::from("EOF waiting for control response")),
                Ok(_) => break,
                Err(err) if err.kind() == ErrorKind::WouldBlock && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(err) => return Err(err.to_string()),
            }
        }
        serde_json::from_str(&reply_line).map_err(|e| format!("invalid JSON reply `{reply_line}`: {e}"))
    }
}

/// Extract `[x, y, w, h]` bounds from a `ui.inspect` node JSON value.
#[must_use]
pub fn bounds_rect(node: &Value) -> Option<[f64; 4]> {
    let arr = node.get("bounds")?.as_array()?;
    Some([arr.first()?.as_f64()?, arr.get(1)?.as_f64()?, arr.get(2)?.as_f64()?, arr.get(3)?.as_f64()?])
}

/// Search all array sections in a `ui.inspect` response for a widget's `[x, y, w, h]` bounds.
#[must_use]
pub fn find_bounds_in_snapshot(snap: &Value, target_wid: &str) -> Option<[f64; 4]> {
    let result_obj = snap.pointer("/result").and_then(Value::as_object)?;
    for value in result_obj.values() {
        if let Some(arr) = value.as_array() {
            for item in arr {
                if item.get("widget_id").and_then(Value::as_str) == Some(target_wid) {
                    return bounds_rect(item);
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Subprocess & loopback TCP control helpers
// ---------------------------------------------------------------------------

/// RAII child process guard that terminates and reaps the child on drop.
pub struct ChildGuard(pub Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[must_use]
pub fn env_truthy(var: &str) -> bool {
    std::env::var(var).is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

#[must_use]
pub fn loopback_required() -> bool {
    env_truthy("CI") || env_truthy("CURVEBOOK_REQUIRE_LOOPBACK")
}

/// Bind an ephemeral `127.0.0.1:0` control listener and verify loopback connectability.
/// Returns `Ok(None)` when sandboxed loopback sockets are denied outside CI.
pub fn bind_loopback_probe_listener() -> Result<Option<(TcpListener, SocketAddr)>, Box<dyn std::error::Error>> {
    let listener = match bind_control_tcp_listener(SocketAddr::from(([127, 0, 0, 1], 0))) {
        Ok(l) => l,
        Err(err) if err.kind() == ErrorKind::PermissionDenied && !loopback_required() => return Ok(None),
        Err(err) => return Err(Box::new(err)),
    };
    let addr = listener.local_addr()?;
    match TcpStream::connect_timeout(&addr, Duration::from_millis(250)) {
        Ok(probe) => {
            drop(probe);
            let _ = listener.accept();
            Ok(Some((listener, addr)))
        }
        Err(err) if err.kind() == ErrorKind::PermissionDenied && !loopback_required() => Ok(None),
        Err(err) => Err(Box::new(err)),
    }
}

/// Probe and reserve an ephemeral loopback port, or `Ok(None)` when loopback is unavailable.
pub fn reserve_ephemeral_port() -> Result<Option<u16>, Box<dyn std::error::Error>> {
    let Some((listener, addr)) = bind_loopback_probe_listener()? else {
        return Ok(None);
    };
    let port = addr.port();
    drop(listener);
    Ok(Some(port))
}

/// Poll until `token_path` exists and return its contents, failing fast if `child` exits early.
pub fn wait_for_token_file(child: &mut Child, token_path: &Path, timeout: Duration) -> Result<String, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + timeout;
    while !token_path.exists() {
        if let Some(status) = child.try_wait()? {
            return Err(format!("curvebook --control exited early with {status}").into());
        }
        if Instant::now() >= deadline {
            return Err(format!("timed out waiting for control token file `{}`", token_path.display()).into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(fs::read_to_string(token_path)?)
}

pub type LoopbackIoPair = (BufReader<TcpStream>, BufWriter<TcpStream>);

/// Connect to a loopback TCP control socket with standard read/write timeouts.
/// Returns `Ok(None)` when sandboxed connect is denied outside CI.
pub fn tcp_connect_loopback(addr: SocketAddr, io_timeout: Duration) -> Result<Option<LoopbackIoPair>, Box<dyn std::error::Error>> {
    let stream = match TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
        Ok(s) => s,
        Err(err) if err.kind() == ErrorKind::PermissionDenied && !loopback_required() => return Ok(None),
        Err(err) => return Err(Box::new(err)),
    };
    stream.set_read_timeout(Some(io_timeout))?;
    stream.set_write_timeout(Some(io_timeout))?;
    let reader = BufReader::new(stream.try_clone()?);
    Ok(Some((reader, BufWriter::new(stream))))
}

/// Send one JSON line over a connected TCP writer/reader pair and parse the JSON reply line.
pub fn send_json_line(
    writer: &mut BufWriter<TcpStream>,
    reader: &mut BufReader<TcpStream>,
    payload: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    writer.write_all(serde_json::to_string(payload)?.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(serde_json::from_str(&line)?)
}

/// Start a [`LiveControlServerGuard`] + [`LiveControlChannel`] bound to an existing listener.
pub fn start_live_control_server_on_listener(
    listener: TcpListener,
    auth: ControlAuth,
) -> Result<(LiveControlServerGuard, LiveControlChannel), Box<dyn std::error::Error>> {
    listener.set_nonblocking(true)?;
    let (bridge, channel) = LiveControlBridge::new(auth);
    let guard = spawn_live_control_accept_loop(
        bridge,
        move || listener.accept().map(|(stream, _)| stream),
        |mut stream: TcpStream, bridge: &LiveControlBridge| {
            stream.set_nonblocking(false)?;
            stream.set_read_timeout(Some(Duration::from_millis(100)))?;
            stream.set_write_timeout(Some(CONTROL_CLIENT_IDLE_TIMEOUT))?;
            let mut reader = BufReader::new(stream.try_clone()?);
            serve_live_control_io(&mut reader, &mut stream, bridge, |tcp| {
                let _ = tcp.shutdown(Shutdown::Write);
                let _ = tcp.set_read_timeout(Some(Duration::from_millis(20)));
                let mut discard = [0u8; 8192];
                let mut drained = 0usize;
                let deadline = Instant::now() + Duration::from_millis(50);
                while drained < 2 * 1024 * 1024 && Instant::now() < deadline {
                    match tcp.read(&mut discard) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => drained += n,
                    }
                }
            })
        },
    )?;
    Ok((guard, channel))
}

/// Drive a [`CurveBookNativeApp`] headless until `close_confirmed()` or `timeout` elapses.
/// Returns `true` if the viewport emitted `ViewportCommand::Close`.
pub fn drive_live_window_until_quit(app: &mut CurveBookNativeApp, ctx: &egui::Context, timeout: Duration) -> bool {
    let mut saw_close_cmd = false;
    let deadline = Instant::now() + timeout;
    while !app.close_confirmed() && Instant::now() < deadline {
        let raw_input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        let mut output = ctx.run_ui(raw_input, |ui| app.step_ui(ui));
        output.textures_delta.clear();
        if output.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.commands.contains(&egui::ViewportCommand::Close)) {
            saw_close_cmd = true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    saw_close_cmd
}

// ---------------------------------------------------------------------------
// Document & interchange fixture builders
// ---------------------------------------------------------------------------

/// Build a ZIP archive in memory from `(entry_name, utf8_body)` pairs (used for `.xlsx` / `.prism`).
pub fn build_zip_entries(entries: &[(&str, &str)]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut zw = ZipWriter::new();
    for (name, body) in entries {
        zw.add_file(name, body.as_bytes().to_vec())?;
    }
    Ok(zw.finish()?)
}

/// Build a minimal single-column XY project document.
pub fn build_xy_doc(sheet_id: &str, title: &str, pairs: &[(f64, f64)]) -> Result<ProjectDocument, Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::default();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, pairs.len().max(1))?;
    if let Some(col) = xy.data_sets.first_mut() {
        col.title = String::from("Signal");
    }
    let mut ds = DataSheet::new(sheet_id, title, TableArchetype::Xy(xy))?;
    for (r, &(x, y)) in pairs.iter().enumerate() {
        ds.set_x_or_time_cell(r, 0, Cell::number(x))?;
        ds.set_cell(CellCoord::new(r, 0, 0), Cell::number(y))?;
    }
    doc.add_data_sheet(ds)?;
    Ok(doc)
}

/// Build a simple Column project document from `(col_title, values)` pairs.
pub fn build_col_doc(sheet_id: &str, title: &str, cols: &[(&str, &[f64])]) -> Result<ProjectDocument, Box<dyn std::error::Error>> {
    let rows = cols.iter().map(|(_, v)| v.len()).max().unwrap_or(1).max(1);
    let mut tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols.len().max(1), rows)?;
    for (c, (name, vals)) in cols.iter().enumerate() {
        if let Some(ds_col) = tbl.data_sets.get_mut(c) {
            ds_col.title = (*name).to_owned();
            for (r, &v) in vals.iter().enumerate() {
                if let Some(cell) = ds_col.cells.get_mut(r).and_then(|row| row.first_mut()) {
                    *cell = Cell::number(v);
                }
            }
        }
    }
    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(DataSheet::new(sheet_id, title, TableArchetype::Column(tbl))?)?;
    Ok(doc)
}
