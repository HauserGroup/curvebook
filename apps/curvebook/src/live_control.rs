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

/// Hard bound on concurrent live-window control client connections (`RV-10`).
pub const MAX_CONCURRENT_CONTROL_CLIENTS: usize = 32;

/// Maximum bytes buffered for a single control line before the connection completes an `"auth"`
/// handshake (`RV-12`, 64 KiB; caps 32 unauthenticated connections to <= 2 MiB total).
pub const MAX_PRE_AUTH_CONTROL_LINE_BYTES: usize = 64 * 1024;

/// Maximum wall-clock time permitted to finish reading a started newline-delimited control frame
/// regardless of 1-byte slowloris trickle progress (`RV-12`).
pub const CONTROL_LINE_READ_DEADLINE: Duration = CONTROL_CLIENT_IDLE_TIMEOUT;

const TRANSIENT_ACCEPT_BACKOFF: Duration = Duration::from_millis(25);
const ACCEPT_ERROR_LOG_INTERVAL: Duration = Duration::from_secs(1);
const OVERSIZED_READER_DRAIN_WINDOW: Duration = Duration::from_millis(50);

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
/// Connections beyond [`MAX_CONCURRENT_CONTROL_CLIENTS`] are dropped immediately without blocking
/// the accept thread.
///
/// # Errors
/// Returns an error if spawning the accept thread fails.
pub fn spawn_live_control_accept_loop<S, A, F>(bridge: LiveControlBridge, accept_next: A, serve_client: F) -> Result<LiveControlServerGuard, String>
where
    S: Send + 'static,
    A: FnMut() -> io::Result<S> + Send + 'static,
    F: Fn(S, &LiveControlBridge) -> io::Result<()> + Clone + Send + Sync + 'static,
{
    spawn_live_control_accept_loop_with_reject(bridge, accept_next, serve_client, |_stream| {})
}

/// Spawn the background non-blocking accept loop with an explicit rejection hook invoked on the
/// accept thread whenever [`MAX_CONCURRENT_CONTROL_CLIENTS`] slots are already occupied.
///
/// `on_rejected` must be bounded and non-blocking (never serving requests inline).
///
/// # Errors
/// Returns an error if spawning the accept thread fails.
pub fn spawn_live_control_accept_loop_with_reject<S, A, F, R>(
    bridge: LiveControlBridge,
    mut accept_next: A,
    serve_client: F,
    mut on_rejected: R,
) -> Result<LiveControlServerGuard, String>
where
    S: Send + 'static,
    A: FnMut() -> io::Result<S> + Send + 'static,
    F: Fn(S, &LiveControlBridge) -> io::Result<()> + Clone + Send + Sync + 'static,
    R: FnMut(S) + Send + 'static,
{
    let shutdown = Arc::clone(bridge.shutdown_flag());
    let accept_thread = std::thread::Builder::new()
        .name(String::from("curvebook-control-accept"))
        .spawn(move || {
            let active_clients = Arc::new(AtomicUsize::new(0));
            let mut last_error_log: Option<Instant> = None;
            while !bridge.is_shutdown() {
                match accept_next() {
                    Ok(stream) => dispatch_accepted_stream(stream, &bridge, &active_clients, &serve_client, &mut on_rejected),
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(err)
                        if matches!(
                            err.kind(),
                            io::ErrorKind::Interrupted | io::ErrorKind::ConnectionAborted | io::ErrorKind::ConnectionReset | io::ErrorKind::TimedOut
                        ) => {}
                    Err(err) => {
                        if !bridge.is_shutdown() {
                            let should_log = last_error_log.is_none_or(|prev| prev.elapsed() >= ACCEPT_ERROR_LOG_INTERVAL);
                            if should_log {
                                eprintln!("live control listener transient accept error: {err}");
                                last_error_log = Some(Instant::now());
                            }
                            std::thread::sleep(TRANSIENT_ACCEPT_BACKOFF);
                        }
                    }
                }
            }
        })
        .map_err(|err| format!("failed to spawn live control accept thread: {err}"))?;
    Ok(LiveControlServerGuard::new(shutdown, accept_thread))
}

fn dispatch_accepted_stream<S, F, R>(stream: S, bridge: &LiveControlBridge, active_clients: &Arc<AtomicUsize>, serve_client: &F, on_rejected: &mut R)
where
    S: Send + 'static,
    F: Fn(S, &LiveControlBridge) -> io::Result<()> + Clone + Send + Sync + 'static,
    R: FnMut(S),
{
    let reserved = active_clients.try_update(Ordering::AcqRel, Ordering::Acquire, |c| (c < MAX_CONCURRENT_CONTROL_CLIENTS).then_some(c + 1)).is_ok();
    if reserved {
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
    } else {
        on_rejected(stream);
    }
}

/// Serve one bounded JSON-lines control connection over generic `reader` and `writer`.
///
/// Before an `"auth"` handshake succeeds on the connection, lines are capped at
/// [`MAX_PRE_AUTH_CONTROL_LINE_BYTES`] (64 KiB); authenticated sessions permit up to
/// [`MAX_CONTROL_LINE_BYTES`] (16 MiB).
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
        let line_cap = if conn_authenticated { MAX_CONTROL_LINE_BYTES } else { MAX_PRE_AUTH_CONTROL_LINE_BYTES };
        let raw_bytes = match read_bounded_control_line_with_shutdown(reader, line_cap, bridge.shutdown_flag())? {
            LiveReadStatus::Line(bytes) => bytes,
            LiveReadStatus::Eof | LiveReadStatus::IdleTimeout | LiveReadStatus::Shutdown => return Ok(()),
            LiveReadStatus::Oversized => {
                write_oversized_control_error_with_limit(writer, line_cap);
                on_oversized(writer);
                drain_oversized_bufread_briefly(reader);
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

/// Write a structured oversized-line JSON error response onto `writer` citing [`MAX_CONTROL_LINE_BYTES`].
pub fn write_oversized_control_error<W: Write>(writer: &mut W) {
    write_oversized_control_error_with_limit(writer, MAX_CONTROL_LINE_BYTES);
}

/// Write a structured oversized-line JSON error response onto `writer` citing `max_bytes`.
pub fn write_oversized_control_error_with_limit<W: Write>(writer: &mut W, max_bytes: usize) {
    let resp = ControlResponse {
        id: Value::String(String::new()),
        ok: false,
        result: Value::Null,
        error: Some(format!("control line exceeds {max_bytes} bytes")),
        error_detail: None,
    };
    if let Ok(encoded) = serde_json::to_string(&resp) {
        let _ = writer.write_all(encoded.as_bytes());
        let _ = writer.write_all(b"\n");
        let _ = writer.flush();
    }
}

/// Write a bounded server-busy JSON error response onto `writer` when all client slots are full.
pub fn write_busy_control_error<W: Write>(writer: &mut W) {
    let resp = ControlResponse {
        id: Value::String(String::new()),
        ok: false,
        result: Value::Null,
        error: Some(format!("control server busy: maximum concurrent clients ({MAX_CONCURRENT_CONTROL_CLIENTS}) reached")),
        error_detail: None,
    };
    if let Ok(encoded) = serde_json::to_string(&resp) {
        let _ = writer.write_all(encoded.as_bytes());
        let _ = writer.write_all(b"\n");
        let _ = writer.flush();
    }
}

fn drain_oversized_bufread_briefly<R: BufRead>(reader: &mut R) {
    let deadline = Instant::now() + OVERSIZED_READER_DRAIN_WINDOW;
    let mut drained = 0usize;
    while drained <= MAX_CONTROL_LINE_BYTES && Instant::now() < deadline {
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

/// Read one bounded newline-terminated frame while observing `shutdown`, idle timeout, and
/// a hard per-line wall-clock deadline (`CONTROL_LINE_READ_DEADLINE`).
///
/// # Errors
/// Propagates underlying non-timeout `BufRead` errors.
pub fn read_bounded_control_line_with_shutdown<R: BufRead>(reader: &mut R, max_bytes: usize, shutdown: &AtomicBool) -> io::Result<LiveReadStatus> {
    read_bounded_control_line_with_deadlines(reader, max_bytes, CONTROL_CLIENT_IDLE_TIMEOUT, CONTROL_LINE_READ_DEADLINE, shutdown)
}

/// Read one bounded newline-terminated frame with explicit `idle_timeout` (before the first byte)
/// and `line_deadline` (total wall-clock budget once the first byte of the line arrives).
///
/// # Errors
/// Propagates underlying non-timeout `BufRead` errors.
pub fn read_bounded_control_line_with_deadlines<R: BufRead>(
    reader: &mut R,
    max_bytes: usize,
    idle_timeout: Duration,
    line_deadline: Duration,
    shutdown: &AtomicBool,
) -> io::Result<LiveReadStatus> {
    let mut line = Vec::new();
    let idle_start = Instant::now();
    let mut line_start: Option<Instant> = None;
    loop {
        if shutdown.load(Ordering::Acquire) {
            return Ok(LiveReadStatus::Shutdown);
        }
        if has_deadline_expired(idle_start, line_start, idle_timeout, line_deadline) {
            return Ok(LiveReadStatus::IdleTimeout);
        }
        match reader.fill_buf() {
            Ok([]) => {
                return Ok(if line.is_empty() { LiveReadStatus::Eof } else { LiveReadStatus::Line(line) });
            }
            Ok(avail) => {
                let started = *line_start.get_or_insert_with(Instant::now);
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
                if started.elapsed() >= line_deadline {
                    return Ok(LiveReadStatus::IdleTimeout);
                }
            }
            Err(err) if matches!(err.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                if shutdown.load(Ordering::Acquire) {
                    return Ok(LiveReadStatus::Shutdown);
                }
                if has_deadline_expired(idle_start, line_start, idle_timeout, line_deadline) {
                    return Ok(LiveReadStatus::IdleTimeout);
                }
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
}

fn has_deadline_expired(idle_start: Instant, line_start: Option<Instant>, idle_timeout: Duration, line_deadline: Duration) -> bool {
    match line_start {
        Some(started) => started.elapsed() >= line_deadline,
        None => idle_start.elapsed() >= idle_timeout,
    }
}

fn closing_window_error_output(err: &str) -> LiveControlFrameOutput {
    let resp = ControlResponse { id: Value::String(String::new()), ok: false, result: Value::Null, error: Some(err.to_owned()), error_detail: None };
    let mut response_bytes =
        serde_json::to_vec(&resp).unwrap_or_else(|_| b"{\"id\":\"\",\"ok\":false,\"result\":null,\"error\":\"closing\"}".to_vec());
    response_bytes.push(b'\n');
    LiveControlFrameOutput { response_bytes, quit: true, close_connection: true, auth_succeeded: None }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io::{self, BufRead, BufReader, BufWriter, Cursor, ErrorKind, Read, Write};
    use std::net::{Shutdown, SocketAddr, TcpStream};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::{Duration, Instant};

    use curvebook_ui_egui::{ControlAuth, UiApp, bind_control_tcp_listener, serve_control_listener};
    use serde_json::{Value, json};

    use super::{
        LiveControlBridge, LiveReadStatus, MAX_CONCURRENT_CONTROL_CLIENTS, MAX_PRE_AUTH_CONTROL_LINE_BYTES, read_bounded_control_line_with_deadlines,
        serve_live_control_io, spawn_live_control_accept_loop_with_reject, write_busy_control_error,
    };
    use crate::desktop_host::LiveControlFrameOutput;

    #[test]
    fn rv10_32_idle_clients_plus_33rd_rejects_without_blocking_and_34th_succeeds_after_slot_release() -> Result<(), Box<dyn std::error::Error>> {
        let auth = ControlAuth::from_token("rv10-slot-secret")?;
        let (bridge, channel) = LiveControlBridge::new(auth.clone());

        // Channel-driven synthetic streams prove exact 32-slot concurrency without requiring OS sockets.
        let (accept_tx, accept_rx) = mpsc::channel::<mpsc::Receiver<u8>>();
        let rejected_count = Arc::new(AtomicUsize::new(0));
        let rejected_bg = Arc::clone(&rejected_count);
        let served_count = Arc::new(AtomicUsize::new(0));
        let served_bg = Arc::clone(&served_count);

        let mut guard = spawn_live_control_accept_loop_with_reject(
            bridge.clone(),
            move || match accept_rx.recv_timeout(Duration::from_millis(10)) {
                Ok(client_rx) => Ok(client_rx),
                Err(mpsc::RecvTimeoutError::Timeout) => Err(io::Error::new(ErrorKind::WouldBlock, "idle")),
                Err(mpsc::RecvTimeoutError::Disconnected) => Err(io::Error::new(ErrorKind::WouldBlock, "closed")),
            },
            move |client_rx: mpsc::Receiver<u8>, client_bridge: &LiveControlBridge| {
                served_bg.fetch_add(1, Ordering::AcqRel);
                while !client_bridge.is_shutdown() {
                    match client_rx.recv_timeout(Duration::from_millis(20)) {
                        Ok(_) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                Ok(())
            },
            move |_excess_rx| {
                rejected_bg.fetch_add(1, Ordering::AcqRel);
            },
        )?;

        let mut idle_senders = Vec::with_capacity(MAX_CONCURRENT_CONTROL_CLIENTS);
        for _ in 0..MAX_CONCURRENT_CONTROL_CLIENTS {
            let (tx, rx) = mpsc::channel::<u8>();
            accept_tx.send(rx)?;
            idle_senders.push(tx);
        }
        let wait_deadline = Instant::now() + Duration::from_secs(2);
        while served_count.load(Ordering::Acquire) < MAX_CONCURRENT_CONTROL_CLIENTS && Instant::now() < wait_deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(served_count.load(Ordering::Acquire), MAX_CONCURRENT_CONTROL_CLIENTS);

        // 33rd connection arrives while all 32 slots are idle: must be rejected immediately (never served inline!).
        let (_tx33, rx33) = mpsc::channel::<u8>();
        accept_tx.send(rx33)?;
        let rej_deadline = Instant::now() + Duration::from_millis(500);
        while rejected_count.load(Ordering::Acquire) == 0 && Instant::now() < rej_deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(rejected_count.load(Ordering::Acquire), 1, "33rd client must be rejected immediately without blocking accept loop");
        assert_eq!(served_count.load(Ordering::Acquire), MAX_CONCURRENT_CONTROL_CLIENTS);

        // Drop 1 idle slot -> 34th connection must immediately acquire the freed slot and be served.
        drop(idle_senders.pop());
        std::thread::sleep(Duration::from_millis(40));
        let (tx34, rx34) = mpsc::channel::<u8>();
        accept_tx.send(rx34)?;
        let slot_deadline = Instant::now() + Duration::from_secs(1);
        while served_count.load(Ordering::Acquire) < MAX_CONCURRENT_CONTROL_CLIENTS + 1 && Instant::now() < slot_deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(served_count.load(Ordering::Acquire), MAX_CONCURRENT_CONTROL_CLIENTS + 1, "closing 1 slot must let next client succeed");

        let _ = tx34.send(1);
        drop(idle_senders);
        drop(channel);
        guard.stop();

        // Also run over real loopback TCP sockets whenever permitted by the execution environment.
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
        listener.set_nonblocking(true)?;
        let (tcp_bridge, tcp_channel) = LiveControlBridge::new(auth.clone());
        let responder = std::thread::spawn(move || {
            while !tcp_channel.shutdown.load(Ordering::Acquire) {
                if let Ok(req) = tcp_channel.request_rx.recv_timeout(Duration::from_millis(20)) {
                    let body = b"{\"id\":\"ok\",\"ok\":true,\"result\":{\"pong\":true},\"error\":null}\n".to_vec();
                    let _ = req.reply_tx.send(Ok(LiveControlFrameOutput {
                        response_bytes: body,
                        quit: false,
                        close_connection: false,
                        auth_succeeded: Some(true),
                    }));
                }
            }
        });
        let mut tcp_guard = spawn_live_control_accept_loop_with_reject(
            tcp_bridge.clone(),
            move || listener.accept().map(|(s, _)| s),
            |mut s: TcpStream, b: &LiveControlBridge| {
                s.set_nonblocking(false)?;
                s.set_read_timeout(Some(Duration::from_millis(100)))?;
                s.set_write_timeout(Some(Duration::from_secs(2)))?;
                let mut reader = BufReader::new(s.try_clone()?);
                serve_live_control_io(&mut reader, &mut s, b, |_| {})
            },
            |mut busy: TcpStream| {
                let _ = busy.set_nonblocking(false);
                let _ = busy.set_write_timeout(Some(Duration::from_millis(50)));
                write_busy_control_error(&mut busy);
                let _ = busy.shutdown(Shutdown::Both);
            },
        )?;

        let mut tcp_idles = Vec::with_capacity(MAX_CONCURRENT_CONTROL_CLIENTS);
        for _ in 0..MAX_CONCURRENT_CONTROL_CLIENTS {
            tcp_idles.push(TcpStream::connect_timeout(&addr, Duration::from_secs(1))?);
        }
        std::thread::sleep(Duration::from_millis(80));

        // 33rd TCP connection receives immediate busy error without blocking accept loop.
        let busy_conn = TcpStream::connect_timeout(&addr, Duration::from_secs(1))?;
        busy_conn.set_read_timeout(Some(Duration::from_millis(500)))?;
        let mut busy_line = String::new();
        let _ = BufReader::new(busy_conn).read_line(&mut busy_line)?;
        if !busy_line.is_empty() {
            let busy_json: Value = serde_json::from_str(&busy_line)?;
            assert_eq!(busy_json["ok"], json!(false));
            assert!(busy_json["error"].as_str().is_some_and(|m| m.contains("busy")));
        }

        // Closing 1 idle TCP slot immediately lets a legitimate client succeed.
        drop(tcp_idles.pop());
        std::thread::sleep(Duration::from_millis(80));
        let legit = TcpStream::connect_timeout(&addr, Duration::from_secs(1))?;
        legit.set_read_timeout(Some(Duration::from_secs(2)))?;
        let mut legit_r = BufReader::new(legit.try_clone()?);
        let mut legit_w = BufWriter::new(legit);
        legit_w.write_all(format!("{{\"id\":\"ok\",\"method\":\"ping\",\"token\":\"{}\"}}\n", auth.as_str()).as_bytes())?;
        legit_w.flush()?;
        let mut legit_reply = String::new();
        legit_r.read_line(&mut legit_reply)?;
        let legit_json: Value = serde_json::from_str(&legit_reply)?;
        assert_eq!(legit_json["ok"], json!(true));

        drop(tcp_idles);
        tcp_bridge.signal_shutdown();
        tcp_guard.stop();
        let _ = responder.join();
        Ok(())
    }

    #[test]
    fn rv11_accept_loop_continues_past_transient_and_fd_exhaustion_errors() -> Result<(), Box<dyn std::error::Error>> {
        let auth = ControlAuth::from_token("rv11-transient-secret")?;
        let (bridge, _channel) = LiveControlBridge::new(auth);

        let mut events: VecDeque<io::Result<usize>> = VecDeque::from(vec![
            Err(io::Error::new(ErrorKind::WouldBlock, "would block")),
            Err(io::Error::new(ErrorKind::Interrupted, "EINTR")),
            Err(io::Error::new(ErrorKind::ConnectionAborted, "ECONNABORTED")),
            Err(io::Error::new(ErrorKind::ConnectionReset, "ECONNRESET")),
            Err(io::Error::new(ErrorKind::TimedOut, "ETIMEDOUT")),
            Err(io::Error::from_raw_os_error(24)), // EMFILE on Unix
            Err(io::Error::other("simulated ENFILE fd exhaustion")),
            Ok(99usize),
        ]);

        let accepted_stream_id = Arc::new(AtomicUsize::new(0));
        let accepted_bg = Arc::clone(&accepted_stream_id);
        let mut guard = super::spawn_live_control_accept_loop(
            bridge.clone(),
            move || events.pop_front().unwrap_or_else(|| Err(io::Error::new(ErrorKind::WouldBlock, "done"))),
            move |stream_id: usize, b: &LiveControlBridge| {
                accepted_bg.store(stream_id, Ordering::Release);
                b.signal_shutdown();
                Ok(())
            },
        )?;

        let deadline = Instant::now() + Duration::from_secs(2);
        while accepted_stream_id.load(Ordering::Acquire) != 99 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(accepted_stream_id.load(Ordering::Acquire), 99, "accept loop must survive all transient errors and serve subsequent stream");
        guard.stop();
        Ok(())
    }

    struct TrickleReader {
        emitted: usize,
        delay: Duration,
        byte: [u8; 1],
    }

    impl Read for TrickleReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            std::thread::sleep(self.delay);
            if let Some(slot) = buf.first_mut() {
                *slot = b'a';
                self.emitted += 1;
                Ok(1)
            } else {
                Ok(0)
            }
        }
    }

    impl BufRead for TrickleReader {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            std::thread::sleep(self.delay);
            self.byte = *b"a";
            self.emitted += 1;
            Ok(&self.byte)
        }

        fn consume(&mut self, _amt: usize) {}
    }

    #[test]
    fn rv12_slowloris_trickle_hits_wall_clock_deadline_and_pre_auth_cap_is_enforced() -> Result<(), Box<dyn std::error::Error>> {
        // 1. Trickling 1 byte every 15ms (faster than idle timeout) must still hit the per-line wall-clock deadline.
        let shutdown = AtomicBool::new(false);
        let mut trickle = TrickleReader { emitted: 0, delay: Duration::from_millis(15), byte: [0] };
        let start = Instant::now();
        let outcome = read_bounded_control_line_with_deadlines(
            &mut trickle,
            MAX_PRE_AUTH_CONTROL_LINE_BYTES,
            Duration::from_millis(200),
            Duration::from_millis(65),
            &shutdown,
        )?;
        assert_eq!(outcome, LiveReadStatus::IdleTimeout, "slowloris trickle must time out at per-line deadline");
        assert!(start.elapsed() < Duration::from_secs(1), "deadline should trip promptly");

        // 2. Unauthenticated client sending > MAX_PRE_AUTH_CONTROL_LINE_BYTES (64 KiB) is rejected at 64 KiB,
        //    whereas an authenticated connection succeeds with > 64 KiB frames.
        let auth = ControlAuth::from_token("rv12-preauth-secret")?;
        let (bridge, channel) = LiveControlBridge::new(auth.clone());
        let worker = std::thread::spawn(move || {
            while let Ok(req) = channel.request_rx.recv() {
                let text = String::from_utf8_lossy(&req.raw_bytes);
                let is_auth = text.contains("\"method\":\"auth\"");
                let out = LiveControlFrameOutput {
                    response_bytes: b"{\"id\":1,\"ok\":true,\"result\":{\"ok\":true},\"error\":null}\n".to_vec(),
                    quit: text.contains("\"app.quit\""),
                    close_connection: false,
                    auth_succeeded: is_auth.then_some(true),
                };
                let _ = req.reply_tx.send(Ok(out));
            }
        });

        // Unauthenticated 70 KiB line -> rejected with 65536-byte error.
        let unauth_big = format!("{{\"id\":1,\"method\":\"ping\",\"pad\":\"{}\"}}\n", "x".repeat(MAX_PRE_AUTH_CONTROL_LINE_BYTES + 1024));
        let mut unauth_reader = Cursor::new(unauth_big.into_bytes());
        let mut unauth_out = Vec::new();
        let mut oversized_called = false;
        serve_live_control_io(&mut unauth_reader, &mut unauth_out, &bridge, |_| {
            oversized_called = true;
        })?;
        assert!(oversized_called, "pre-auth >64 KiB line must trigger oversized rejection");
        let unauth_resp: Value = serde_json::from_slice(&unauth_out)?;
        assert_eq!(unauth_resp["ok"], json!(false));
        assert!(unauth_resp["error"].as_str().is_some_and(|e| e.contains(&MAX_PRE_AUTH_CONTROL_LINE_BYTES.to_string())));

        // Authenticated connection -> 90 KiB (> 64 KiB) command line succeeds!
        let mut authed_wire = format!("{{\"id\":1,\"method\":\"auth\",\"params\":{{\"token\":\"{}\"}}}}\n", auth.as_str());
        authed_wire.push_str(&format!("{{\"id\":2,\"method\":\"ping\",\"pad\":\"{}\"}}\n", "y".repeat(MAX_PRE_AUTH_CONTROL_LINE_BYTES + 24 * 1024)));
        let mut authed_reader = Cursor::new(authed_wire.into_bytes());
        let mut authed_out = Vec::new();
        serve_live_control_io(&mut authed_reader, &mut authed_out, &bridge, |_| {})?;
        let lines: Vec<Value> = String::from_utf8(authed_out)?.lines().map(serde_json::from_str).collect::<Result<_, _>>()?;
        assert_eq!(lines.len(), 2, "both auth and >64 KiB post-auth command must succeed");
        assert_eq!(lines[0]["ok"], json!(true));
        assert_eq!(lines[1]["ok"], json!(true));

        drop(bridge);
        let _ = worker.join();
        Ok(())
    }

    #[test]
    fn rv54_headless_control_listener_serves_concurrent_clients_while_stalled_clients_hold_sockets() -> Result<(), Box<dyn std::error::Error>> {
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

        let auth = ControlAuth::generate()?;
        let server_auth = auth.clone();
        let server = std::thread::spawn(move || -> io::Result<usize> {
            let mut app = UiApp::new();
            serve_control_listener(&mut app, &listener, &server_auth)?;
            Ok(app.document().data_sheets.len())
        });

        // Open 4 stalled connections writing partial JSON without a newline.
        let mut stalled_conns = Vec::new();
        for _ in 0..4 {
            let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
            s.write_all(b"{\"id\":\"stalled\"")?;
            s.flush()?;
            stalled_conns.push(s);
        }

        // Legitimate client must succeed IMMEDIATELY (< 500ms, well below the 2s idle timeout!).
        let fast_start = Instant::now();
        let tok = auth.as_str();
        {
            let client = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
            client.set_read_timeout(Some(Duration::from_secs(2)))?;
            let mut r = BufReader::new(client.try_clone()?);
            let mut w = BufWriter::new(client);
            w.write_all(
                format!("{{\"id\":1,\"method\":\"sheet.createData\",\"token\":\"{tok}\",\"params\":{{\"sheet_id\":\"rv54-1\",\"title\":\"Concurrent\"}}}}\n")
                    .as_bytes(),
            )?;
            w.flush()?;
            let mut reply = String::new();
            r.read_line(&mut reply)?;
            let parsed: Value = serde_json::from_str(&reply)?;
            assert_eq!(parsed["ok"], json!(true));
            assert!(fast_start.elapsed() < Duration::from_millis(1200), "stalled clients must not block concurrent headless control clients");

            w.write_all(format!("{{\"id\":2,\"method\":\"app.quit\",\"token\":\"{tok}\"}}\n").as_bytes())?;
            w.flush()?;
            reply.clear();
            let _ = r.read_line(&mut reply);
        }

        let sheet_count = server.join().map_err(|_| "headless server panicked")??;
        assert_eq!(sheet_count, 1);
        drop(stalled_conns);
        Ok(())
    }
}
