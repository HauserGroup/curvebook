#![forbid(unsafe_code)]

//! Non-blocking stream control bridge for live `eframe`/`egui` desktop windows (`R7.3`).

use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::{Duration, Instant};

use curvebook_ui_egui::control_io::CONTROL_CLIENT_IDLE_TIMEOUT;
use curvebook_ui_egui::{ControlAuth, ControlResponse, MAX_CONTROL_LINE_BYTES};
use serde_json::Value;

use crate::desktop_host::{ControlFrameKind, LiveControlFrameOutput, classify_control_frame};
use crate::gui_window::{LiveControlChannel, LiveControlRequest};

const MAX_CONCURRENT_CONTROL_CLIENTS: usize = 32;

/// RAII guard that stops and joins the live-window control accept thread on drop.
pub struct LiveControlServerGuard {
    shutdown: Arc<AtomicBool>,
    accept_thread: Option<std::thread::JoinHandle<()>>,
}

impl LiveControlServerGuard {
    /// Wrap an already-spawned accept thread handle and its shared shutdown flag.
    #[must_use]
    pub const fn new(shutdown: Arc<AtomicBool>, accept_thread: std::thread::JoinHandle<()>) -> Self {
        Self { shutdown, accept_thread: Some(accept_thread) }
    }

    /// Signal shutdown and join the accept thread immediately.
    pub fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
    }

    /// Shared shutdown signal observed by the accept loop, client threads, and live window.
    #[must_use]
    pub fn shutdown_flag(&self) -> &Arc<AtomicBool> {
        &self.shutdown
    }
}

impl Drop for LiveControlServerGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Cloneable thread-safe bridge forwarding control frames into a live [`LiveControlChannel`].
#[derive(Clone)]
pub struct LiveControlBridge {
    auth: ControlAuth,
    request_tx: mpsc::Sender<LiveControlRequest>,
    egui_ctx_cell: Arc<OnceLock<egui::Context>>,
    shutdown: Arc<AtomicBool>,
}

impl LiveControlBridge {
    /// Construct a paired [`LiveControlBridge`] and [`LiveControlChannel`].
    #[must_use]
    pub fn new(auth: ControlAuth) -> (Self, LiveControlChannel) {
        let (request_tx, request_rx) = mpsc::channel::<LiveControlRequest>();
        let egui_ctx_cell = Arc::new(OnceLock::new());
        let shutdown = Arc::new(AtomicBool::new(false));
        let bridge = Self { auth, request_tx, egui_ctx_cell: Arc::clone(&egui_ctx_cell), shutdown: Arc::clone(&shutdown) };
        let channel = LiveControlChannel { request_rx, egui_ctx_cell, shutdown, post_hook: None };
        (bridge, channel)
    }

    /// Shared shutdown signal observed by the accept loop, client threads, and live window.
    #[must_use]
    pub fn shutdown_flag(&self) -> &Arc<AtomicBool> {
        &self.shutdown
    }

    /// Whether shutdown has already been requested.
    #[must_use]
    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }

    /// Request shutdown across the accept loop and active client connections.
    pub fn signal_shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }

    /// Queue one bounded raw frame onto the live UI window thread and await its response.
    #[must_use]
    pub fn dispatch_frame(&self, raw_bytes: Vec<u8>, conn_authenticated: bool) -> Option<LiveControlFrameOutput> {
        let (reply_tx, reply_rx) = mpsc::channel::<Result<LiveControlFrameOutput, String>>();
        let request = LiveControlRequest { raw_bytes, auth: self.auth.clone(), conn_authenticated, reply_tx };
        if self.request_tx.send(request).is_err() {
            return None;
        }
        if let Some(egui_ctx) = self.egui_ctx_cell.get() {
            egui_ctx.request_repaint();
        }
        match reply_rx.recv() {
            Ok(Ok(out)) => Some(out),
            Ok(Err(err)) => Some(closing_window_error_output(&err)),
            Err(_) => None,
        }
    }
}

struct ActiveClientGuard(Arc<AtomicUsize>);

impl Drop for ActiveClientGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Outcome of reading one bounded newline-delimited control frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveReadStatus {
    Line(Vec<u8>),
    Eof,
    Oversized,
    IdleTimeout,
    Shutdown,
}

/// Spawn the background non-blocking accept loop dispatching accepted streams via `serve_client`.
///
/// # Errors
/// Returns an error if spawning the accept thread fails.
pub fn spawn_live_control_accept_loop<S, A, F>(
    bridge: LiveControlBridge,
    mut accept_next: A,
    serve_client: F,
) -> Result<LiveControlServerGuard, String>
where
    S: Send + 'static,
    A: FnMut() -> io::Result<S> + Send + 'static,
    F: Fn(S, &LiveControlBridge) -> io::Result<()> + Clone + Send + Sync + 'static,
{
    let shutdown = Arc::clone(bridge.shutdown_flag());
    let accept_thread = std::thread::Builder::new()
        .name(String::from("curvebook-control-accept"))
        .spawn(move || {
            let active_clients = Arc::new(AtomicUsize::new(0));
            while !bridge.is_shutdown() {
                match accept_next() {
                    Ok(stream) => dispatch_accepted_stream(stream, &bridge, &active_clients, &serve_client),
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(err) => {
                        if !bridge.is_shutdown() {
                            eprintln!("live control listener accept error: {err}");
                        }
                        break;
                    }
                }
            }
        })
        .map_err(|err| format!("failed to spawn live control accept thread: {err}"))?;
    Ok(LiveControlServerGuard::new(shutdown, accept_thread))
}

fn dispatch_accepted_stream<S, F>(stream: S, bridge: &LiveControlBridge, active_clients: &Arc<AtomicUsize>, serve_client: &F)
where
    S: Send + 'static,
    F: Fn(S, &LiveControlBridge) -> io::Result<()> + Clone + Send + Sync + 'static,
{
    if active_clients.load(Ordering::Acquire) < MAX_CONCURRENT_CONTROL_CLIENTS {
        active_clients.fetch_add(1, Ordering::AcqRel);
        let guard = ActiveClientGuard(Arc::clone(active_clients));
        let client_bridge = bridge.clone();
        let client_serve = serve_client.clone();
        let spawned = std::thread::Builder::new().name(String::from("curvebook-control-client")).spawn(move || {
            let _guard = guard;
            if let Err(err) = client_serve(stream, &client_bridge) {
                eprintln!("live control connection closed after error: {err}");
            }
        });
        if spawned.is_err() {
            eprintln!("live control client thread spawn failed; rejecting client");
        }
    } else if let Err(err) = serve_client(stream, bridge) {
        eprintln!("live control connection closed after error: {err}");
    }
}

/// Serve one bounded JSON-lines control connection over generic `reader` and `writer`.
///
/// # Errors
/// Propagates underlying stream read/write failures.
pub fn serve_live_control_io<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    bridge: &LiveControlBridge,
    on_oversized: impl FnOnce(&mut W),
) -> io::Result<()> {
    let mut conn_authenticated = false;
    loop {
        let raw_bytes = match read_bounded_control_line_with_shutdown(reader, MAX_CONTROL_LINE_BYTES, bridge.shutdown_flag())? {
            LiveReadStatus::Line(bytes) => bytes,
            LiveReadStatus::Eof | LiveReadStatus::IdleTimeout | LiveReadStatus::Shutdown => return Ok(()),
            LiveReadStatus::Oversized => {
                write_oversized_control_error(writer);
                on_oversized(writer);
                return Ok(());
            }
        };
        if classify_control_frame(&raw_bytes) == ControlFrameKind::Empty {
            continue;
        }
        let Some(out) = bridge.dispatch_frame(raw_bytes, conn_authenticated) else {
            return Ok(());
        };
        if let Some(auth_ok) = out.auth_succeeded {
            conn_authenticated = auth_ok;
        }
        writer.write_all(&out.response_bytes)?;
        writer.flush()?;
        if out.quit {
            bridge.signal_shutdown();
        }
        if out.close_connection {
            return Ok(());
        }
    }
}

/// Write a structured oversized-line JSON error response onto `writer`.
pub fn write_oversized_control_error<W: Write>(writer: &mut W) {
    let resp = ControlResponse {
        id: Value::String(String::new()),
        ok: false,
        result: Value::Null,
        error: Some(format!("control line exceeds {MAX_CONTROL_LINE_BYTES} bytes")),
        error_detail: None,
    };
    if let Ok(encoded) = serde_json::to_string(&resp) {
        let _ = writer.write_all(encoded.as_bytes());
        let _ = writer.write_all(b"\n");
        let _ = writer.flush();
    }
}

/// Read one bounded newline-terminated frame while observing `shutdown` and idle timeout.
///
/// # Errors
/// Propagates underlying non-timeout `BufRead` errors.
pub fn read_bounded_control_line_with_shutdown<R: BufRead>(reader: &mut R, max_bytes: usize, shutdown: &AtomicBool) -> io::Result<LiveReadStatus> {
    let mut line = Vec::new();
    let mut last_activity = Instant::now();
    loop {
        if shutdown.load(Ordering::Acquire) {
            return Ok(LiveReadStatus::Shutdown);
        }
        match reader.fill_buf() {
            Ok([]) => {
                return Ok(if line.is_empty() { LiveReadStatus::Eof } else { LiveReadStatus::Line(line) });
            }
            Ok(avail) => {
                last_activity = Instant::now();
                if let Some(newline_pos) = avail.iter().position(|&b| b == b'\n') {
                    let take = newline_pos + 1;
                    if line.len().saturating_add(take) > max_bytes {
                        return Ok(LiveReadStatus::Oversized);
                    }
                    line.extend_from_slice(&avail[..take]);
                    reader.consume(take);
                    return Ok(LiveReadStatus::Line(line));
                }
                if line.len().saturating_add(avail.len()) > max_bytes {
                    return Ok(LiveReadStatus::Oversized);
                }
                line.extend_from_slice(avail);
                let consumed = avail.len();
                reader.consume(consumed);
            }
            Err(err) if matches!(err.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                if shutdown.load(Ordering::Acquire) {
                    return Ok(LiveReadStatus::Shutdown);
                }
                if last_activity.elapsed() >= CONTROL_CLIENT_IDLE_TIMEOUT {
                    return Ok(LiveReadStatus::IdleTimeout);
                }
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
}

fn closing_window_error_output(err: &str) -> LiveControlFrameOutput {
    let resp = ControlResponse { id: Value::String(String::new()), ok: false, result: Value::Null, error: Some(err.to_owned()), error_detail: None };
    let mut response_bytes =
        serde_json::to_vec(&resp).unwrap_or_else(|_| b"{\"id\":\"\",\"ok\":false,\"result\":null,\"error\":\"closing\"}".to_vec());
    response_bytes.push(b'\n');
    LiveControlFrameOutput { response_bytes, quit: true, close_connection: true, auth_succeeded: None }
}
