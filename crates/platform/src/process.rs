//! Bounded child-process runner with concurrent stream draining, stdin pipe closing,
//! wall-clock and idle timeouts, output byte caps, and guaranteed drop-kill cleanup.

#[cfg(not(target_arch = "wasm32"))]
use std::io::{ErrorKind, Read, Write};
#[cfg(all(not(target_arch = "wasm32"), unix))]
use std::os::unix::process::CommandExt as _;
#[cfg(all(not(target_arch = "wasm32"), windows))]
use std::os::windows::process::CommandExt as _;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::process::{Child, Command, ExitStatus, Stdio};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{self, RecvTimeoutError};
#[cfg(not(target_arch = "wasm32"))]
use std::thread::JoinHandle;
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

#[cfg(not(target_arch = "wasm32"))]
use crate::{PlatformError, TimeoutKind};

/// Default total wall-clock timeout for [`ProcessCommand`].
pub const DEFAULT_WALL_TIMEOUT: Duration = Duration::from_secs(30);

/// Default idle timeout (time since last byte on `stdout`/`stderr`) for [`ProcessCommand`].
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(10);

/// Default per-stream output cap (16 MiB) for [`ProcessCommand`].
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

#[cfg(not(target_arch = "wasm32"))]
const READER_CHUNK_BYTES: usize = 8192;

#[cfg(not(target_arch = "wasm32"))]
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(5);

#[cfg(not(target_arch = "wasm32"))]
const ERROR_DRAIN_TIMEOUT: Duration = Duration::from_millis(250);

#[cfg(not(target_arch = "wasm32"))]
const FALLBACK_MAX_DEADLINE: Duration = Duration::from_secs(365 * 24 * 3600);

/// Buffered result returned by [`ProcessCommand::run`].
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    /// Numeric exit code if the process exited normally, or `None` if terminated by signal.
    pub status_code: Option<i32>,
    /// Whether the child exited with a zero / successful status.
    pub success: bool,
    /// Captured `stdout` bytes (bounded by `max_output_bytes`).
    pub stdout: Vec<u8>,
    /// Captured `stderr` bytes (bounded by `max_output_bytes`).
    pub stderr: Vec<u8>,
}

/// Builder for running an external OS command with bounded timeouts and output limits.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone)]
pub struct ProcessCommand {
    program: String,
    args: Vec<String>,
    env_vars: Vec<(String, String)>,
    env_remove: Vec<String>,
    current_dir: Option<PathBuf>,
    stdin_bytes: Option<Vec<u8>>,
    wall_timeout: Duration,
    idle_timeout: Duration,
    max_output_bytes: usize,
    capture_stdout: bool,
    capture_stderr: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl ProcessCommand {
    /// Construct a new command with `stdin` closed (`Stdio::null()`), 30s wall timeout,
    /// 10s idle timeout, and 16 MiB per-stream output cap.
    #[must_use]
    pub fn new(program: impl AsRef<str>) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            args: Vec::new(),
            env_vars: Vec::new(),
            env_remove: Vec::new(),
            current_dir: None,
            stdin_bytes: None,
            wall_timeout: DEFAULT_WALL_TIMEOUT,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            capture_stdout: true,
            capture_stderr: true,
        }
    }

    /// Append a single command-line argument.
    #[must_use]
    pub fn arg(mut self, arg: impl AsRef<str>) -> Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }

    /// Append multiple command-line arguments.
    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for arg in args {
            self.args.push(arg.as_ref().to_owned());
        }
        self
    }

    /// Set or override an environment variable for the child process.
    #[must_use]
    pub fn env(mut self, key: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        self.env_vars.push((key.as_ref().to_owned(), value.as_ref().to_owned()));
        self
    }

    /// Set or override multiple environment variables for the child process.
    #[must_use]
    pub fn envs<I, K, V>(mut self, vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        for (key, value) in vars {
            self.env_vars.push((key.as_ref().to_owned(), value.as_ref().to_owned()));
        }
        self
    }

    /// Remove an inherited environment variable from the child process.
    #[must_use]
    pub fn env_remove(mut self, key: impl AsRef<str>) -> Self {
        self.env_remove.push(key.as_ref().to_owned());
        self
    }

    /// Set the working directory for the child process.
    #[must_use]
    pub fn current_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.current_dir = Some(dir.as_ref().to_path_buf());
        self
    }

    /// Provide bytes to write to the child's `stdin` before closing the pipe.
    #[must_use]
    pub fn stdin_bytes(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.stdin_bytes = Some(bytes.into());
        self
    }

    /// Close `stdin` (`Stdio::null()`), discarding any previously staged `stdin_bytes`.
    #[must_use]
    pub fn clear_stdin(mut self) -> Self {
        self.stdin_bytes = None;
        self
    }

    /// Set the maximum total wall-clock duration allowed after spawn.
    #[must_use]
    pub fn wall_timeout(mut self, timeout: Duration) -> Self {
        self.wall_timeout = timeout;
        self
    }

    /// Set the maximum silence duration allowed without receiving bytes on `stdout` or `stderr`.
    #[must_use]
    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout;
        self
    }

    /// Set the maximum byte budget per output stream (`stdout` and `stderr` independently).
    #[must_use]
    pub fn max_output_bytes(mut self, max_bytes: usize) -> Self {
        self.max_output_bytes = max_bytes;
        self
    }

    /// Redirect child `stdout` and `stderr` to [`Stdio::null()`] instead of reading pipes.
    ///
    /// Useful for clipboard owners such as `wl-copy` or `xclip` that fork a background
    /// selection server inheriting open pipe descriptors.
    #[must_use]
    pub const fn discard_output(mut self) -> Self {
        self.capture_stdout = false;
        self.capture_stderr = false;
        self
    }

    /// Configure whether child `stdout` is captured via a pipe (`true`) or redirected to [`Stdio::null()`] (`false`).
    #[must_use]
    pub const fn capture_stdout(mut self, capture: bool) -> Self {
        self.capture_stdout = capture;
        self
    }

    /// Configure whether child `stderr` is captured via a pipe (`true`) or redirected to [`Stdio::null()`] (`false`).
    #[must_use]
    pub const fn capture_stderr(mut self, capture: bool) -> Self {
        self.capture_stderr = capture;
        self
    }

    /// Spawn the child process, drain streams concurrently, enforce deadlines/caps, and wait for exit.
    ///
    /// # Errors
    /// Returns [`PlatformError::ProgramNotFound`] when the binary cannot be found,
    /// [`PlatformError::TimedOut`] on wall/idle expiration, [`PlatformError::OutputLimitExceeded`]
    /// if a stream exceeds `max_output_bytes`, or [`PlatformError::Io`] on OS spawn/pipe failure.
    pub fn run(&self) -> Result<ProcessOutput, PlatformError> {
        let mut child = self.spawn_child()?;
        let stdin_pipe = child.stdin.take();
        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();

        let mut guard = ChildGuard::new(child);
        let cancelled = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel::<StreamEvent>();

        let stdin_handle = spawn_stdin_writer(stdin_pipe, self.stdin_bytes.clone(), tx.clone());
        let stdout_handle = stdout_pipe.map(|pipe| spawn_reader(pipe, StreamKind::Stdout, self.max_output_bytes, Arc::clone(&cancelled), tx.clone()));
        let stderr_handle = stderr_pipe.map(|pipe| spawn_reader(pipe, StreamKind::Stderr, self.max_output_bytes, Arc::clone(&cancelled), tx.clone()));
        drop(tx);

        let mut state = MonitorState::new(self, stdout_handle.is_none(), stderr_handle.is_none(), stdin_handle.is_none());
        let status = match state.wait_for_completion(&mut guard, &rx, &cancelled) {
            Ok(status) => {
                guard.disarm();
                status
            }
            Err(err) => {
                cancelled.store(true, Ordering::Release);
                guard.kill_group_and_wait();
                drain_workers_on_error(&rx, [stdin_handle, stdout_handle, stderr_handle]);
                return Err(err);
            }
        };

        join_best_effort(stdin_handle);
        join_best_effort(stdout_handle);
        join_best_effort(stderr_handle);

        Ok(ProcessOutput { status_code: status.code(), success: status.success(), stdout: state.stdout, stderr: state.stderr })
    }

    fn spawn_child(&self) -> Result<Child, PlatformError> {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args).stdout(if self.capture_stdout { Stdio::piped() } else { Stdio::null() }).stderr(if self.capture_stderr {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        if self.stdin_bytes.is_some() {
            cmd.stdin(Stdio::piped());
        } else {
            cmd.stdin(Stdio::null());
        }
        #[cfg(unix)]
        cmd.process_group(0);
        #[cfg(windows)]
        {
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
        }
        for key in &self.env_remove {
            cmd.env_remove(key);
        }
        for (key, value) in &self.env_vars {
            cmd.env(key, value);
        }
        if let Some(dir) = &self.current_dir {
            cmd.current_dir(dir);
        }

        match cmd.spawn() {
            Ok(child) => Ok(child),
            Err(err) if err.kind() == ErrorKind::NotFound => Err(PlatformError::ProgramNotFound { program: self.program.clone() }),
            Err(err) => Err(PlatformError::Io { context: format!("spawn `{}`", self.program), message: err.to_string() }),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct ChildGuard {
    child: Option<Child>,
    pgid: Option<u32>,
    disarmed: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl ChildGuard {
    fn new(child: Child) -> Self {
        let pid = child.id();
        Self { child: Some(child), pgid: (pid > 1).then_some(pid), disarmed: false }
    }

    fn disarm(&mut self) {
        self.disarmed = true;
    }

    fn try_wait(&mut self) -> Result<Option<ExitStatus>, std::io::Error> {
        let Some(child) = self.child.as_mut() else {
            return Ok(None);
        };
        match child.try_wait()? {
            Some(status) => {
                self.child = None;
                Ok(Some(status))
            }
            None => Ok(None),
        }
    }

    fn kill_group_and_wait(&mut self) {
        if self.disarmed {
            return;
        }
        if let Some(pgid) = self.pgid.take() {
            kill_process_group(pgid);
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.kill_group_and_wait();
    }
}

#[cfg(all(not(target_arch = "wasm32"), unix))]
fn kill_process_group(pgid: u32) {
    if pgid <= 1 || pgid == std::process::id() {
        return;
    }
    let target = format!("-{pgid}");
    for bin in ["/bin/kill", "/usr/bin/kill", "kill"] {
        if let Ok(mut killer) =
            Command::new(bin).args(["-s", "KILL", "--", &target]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
        {
            let _ = killer.wait();
            return;
        }
    }
}

#[cfg(all(not(target_arch = "wasm32"), windows))]
fn kill_process_group(pgid: u32) {
    if pgid <= 1 || pgid == std::process::id() {
        return;
    }
    let pid_str = pgid.to_string();
    if let Ok(mut killer) =
        Command::new("taskkill").args(["/PID", &pid_str, "/T", "/F"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
    {
        let _ = killer.wait();
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(any(unix, windows))))]
fn kill_process_group(_pgid: u32) {}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy)]
enum StreamKind {
    Stdout,
    Stderr,
}

#[cfg(not(target_arch = "wasm32"))]
impl StreamKind {
    const fn name(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
enum StreamEvent {
    Chunk { stream: StreamKind, bytes: Vec<u8> },
    Eof(StreamKind),
    StdinDone,
    LimitExceeded(StreamKind),
    IoErr { stream: StreamKind, message: String },
}

#[cfg(not(target_arch = "wasm32"))]
struct MonitorState {
    program: String,
    wall_timeout: Duration,
    idle_timeout: Duration,
    max_output_bytes: usize,
    wall_deadline: Instant,
    idle_deadline: Instant,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_eof: bool,
    stderr_eof: bool,
    stdin_done: bool,
    exit_status: Option<ExitStatus>,
}

#[cfg(not(target_arch = "wasm32"))]
impl MonitorState {
    fn new(cmd: &ProcessCommand, stdout_eof: bool, stderr_eof: bool, stdin_done: bool) -> Self {
        let now = Instant::now();
        Self {
            program: cmd.program.clone(),
            wall_timeout: cmd.wall_timeout,
            idle_timeout: cmd.idle_timeout,
            max_output_bytes: cmd.max_output_bytes,
            wall_deadline: add_deadline(now, cmd.wall_timeout),
            idle_deadline: add_deadline(now, cmd.idle_timeout),
            stdout: Vec::new(),
            stderr: Vec::new(),
            stdout_eof,
            stderr_eof,
            stdin_done,
            exit_status: None,
        }
    }

    fn wait_for_completion(
        &mut self,
        guard: &mut ChildGuard,
        rx: &mpsc::Receiver<StreamEvent>,
        cancelled: &AtomicBool,
    ) -> Result<ExitStatus, PlatformError> {
        loop {
            if self.exit_status.is_none()
                && let Some(status) =
                    guard.try_wait().map_err(|err| PlatformError::Io { context: format!("wait `{}`", self.program), message: err.to_string() })?
            {
                self.exit_status = Some(status);
            }

            if self.stdout_eof && self.stderr_eof && self.stdin_done {
                if let Some(status) = self.exit_status {
                    return Ok(status);
                }
                let remaining = self.remaining_until_deadline()?;
                std::thread::sleep(remaining.min(EXIT_POLL_INTERVAL));
                continue;
            }

            let remaining = self.remaining_until_deadline()?;
            match rx.recv_timeout(remaining) {
                Ok(event) => self.handle_event(event)?,
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(err) = self.check_timeout(Instant::now()) {
                        cancelled.store(true, Ordering::Release);
                        return Err(err);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.stdout_eof = true;
                    self.stderr_eof = true;
                    self.stdin_done = true;
                }
            }
        }
    }

    fn remaining_until_deadline(&self) -> Result<Duration, PlatformError> {
        let now = Instant::now();
        if let Some(err) = self.check_timeout(now) {
            return Err(err);
        }
        let next = self.wall_deadline.min(self.idle_deadline);
        Ok(next.saturating_duration_since(now))
    }

    fn check_timeout(&self, now: Instant) -> Option<PlatformError> {
        if now >= self.wall_deadline {
            return Some(PlatformError::TimedOut {
                program: self.program.clone(),
                kind: TimeoutKind::Wall,
                limit_ms: duration_to_ms(self.wall_timeout),
            });
        }
        if now >= self.idle_deadline {
            return Some(PlatformError::TimedOut {
                program: self.program.clone(),
                kind: TimeoutKind::Idle,
                limit_ms: duration_to_ms(self.idle_timeout),
            });
        }
        None
    }

    fn handle_event(&mut self, event: StreamEvent) -> Result<(), PlatformError> {
        match event {
            StreamEvent::Chunk { stream, bytes } => {
                self.idle_deadline = add_deadline(Instant::now(), self.idle_timeout);
                let target = match stream {
                    StreamKind::Stdout => &mut self.stdout,
                    StreamKind::Stderr => &mut self.stderr,
                };
                if target.len().saturating_add(bytes.len()) > self.max_output_bytes {
                    return Err(PlatformError::OutputLimitExceeded {
                        program: self.program.clone(),
                        stream: stream.name(),
                        limit_bytes: self.max_output_bytes,
                    });
                }
                target.extend_from_slice(&bytes);
                Ok(())
            }
            StreamEvent::Eof(StreamKind::Stdout) => {
                self.stdout_eof = true;
                Ok(())
            }
            StreamEvent::Eof(StreamKind::Stderr) => {
                self.stderr_eof = true;
                Ok(())
            }
            StreamEvent::StdinDone => {
                self.stdin_done = true;
                Ok(())
            }
            StreamEvent::LimitExceeded(stream) => {
                Err(PlatformError::OutputLimitExceeded { program: self.program.clone(), stream: stream.name(), limit_bytes: self.max_output_bytes })
            }
            StreamEvent::IoErr { stream, message } => {
                Err(PlatformError::Io { context: format!("read {} from `{}`", stream.name(), self.program), message })
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct WorkerHandle {
    join: JoinHandle<()>,
    done: Arc<AtomicBool>,
}

#[cfg(not(target_arch = "wasm32"))]
struct DoneFlagGuard(Arc<AtomicBool>);

#[cfg(not(target_arch = "wasm32"))]
impl Drop for DoneFlagGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl WorkerHandle {
    fn spawn<F>(f: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        let done = Arc::new(AtomicBool::new(false));
        let done_clone = Arc::clone(&done);
        let join = std::thread::spawn(move || {
            let _guard = DoneFlagGuard(done_clone);
            f();
        });
        Self { join, done }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_stdin_writer(
    stdin_pipe: Option<std::process::ChildStdin>,
    stdin_bytes: Option<Vec<u8>>,
    tx: mpsc::Sender<StreamEvent>,
) -> Option<WorkerHandle> {
    let (Some(mut stdin), Some(bytes)) = (stdin_pipe, stdin_bytes) else {
        return None;
    };
    Some(WorkerHandle::spawn(move || {
        let _ = stdin.write_all(&bytes);
        drop(stdin);
        let _ = tx.send(StreamEvent::StdinDone);
        drop(tx);
    }))
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_reader<R: Read + Send + 'static>(
    mut reader: R,
    stream: StreamKind,
    max_bytes: usize,
    cancelled: Arc<AtomicBool>,
    tx: mpsc::Sender<StreamEvent>,
) -> WorkerHandle {
    WorkerHandle::spawn(move || {
        let mut buf = [0u8; READER_CHUNK_BYTES];
        let mut total = 0usize;
        loop {
            if cancelled.load(Ordering::Acquire) {
                break;
            }
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = tx.send(StreamEvent::Eof(stream));
                    break;
                }
                Ok(n) => {
                    total = total.saturating_add(n);
                    if total > max_bytes {
                        let _ = tx.send(StreamEvent::LimitExceeded(stream));
                        break;
                    }
                    if tx.send(StreamEvent::Chunk { stream, bytes: buf[..n].to_vec() }).is_err() {
                        break;
                    }
                }
                Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(err) => {
                    let _ = tx.send(StreamEvent::IoErr { stream, message: err.to_string() });
                    break;
                }
            }
        }
        drop(reader);
        drop(tx);
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn join_best_effort(handle: Option<WorkerHandle>) {
    if let Some(h) = handle {
        let _ = h.join.join();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn drain_workers_on_error(rx: &mpsc::Receiver<StreamEvent>, handles: [Option<WorkerHandle>; 3]) {
    let deadline = add_deadline(Instant::now(), ERROR_DRAIN_TIMEOUT);
    let mut disconnected = false;
    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        match rx.recv_timeout(deadline.saturating_duration_since(now)) {
            Ok(_) => {}
            Err(RecvTimeoutError::Disconnected) => {
                disconnected = true;
                break;
            }
            Err(RecvTimeoutError::Timeout) => break,
        }
    }
    for handle in handles.into_iter().flatten() {
        if disconnected || handle.done.load(Ordering::Acquire) {
            let _ = handle.join.join();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn add_deadline(start: Instant, dur: Duration) -> Instant {
    start.checked_add(dur).unwrap_or_else(|| start + FALLBACK_MAX_DEADLINE)
}

#[cfg(not(target_arch = "wasm32"))]
fn duration_to_ms(dur: Duration) -> u64 {
    u64::try_from(dur.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use std::time::{Duration, Instant};

    use super::ProcessCommand;
    use crate::{PlatformError, TimeoutKind};

    #[test]
    fn missing_binary_returns_program_not_found() {
        let err = ProcessCommand::new("curvebook-definitely-missing-binary-xyz-123").run().unwrap_err();
        assert_eq!(err, PlatformError::ProgramNotFound { program: String::from("curvebook-definitely-missing-binary-xyz-123") });
    }

    #[cfg(unix)]
    #[test]
    fn stdin_stdout_stderr_and_exit_code_work_cleanly() {
        let out = ProcessCommand::new("sh").args(["-c", "cat; echo warn >&2"]).stdin_bytes("hello curvebook").env("CB_TEST_VAR", "ok").run().unwrap();
        assert!(out.success);
        assert_eq!(out.status_code, Some(0));
        assert_eq!(out.stdout, b"hello curvebook");
        assert_eq!(out.stderr, b"warn\n");

        let nonzero = ProcessCommand::new("sh").args(["-c", "exit 7"]).run().unwrap();
        assert!(!nonzero.success);
        assert_eq!(nonzero.status_code, Some(7));
    }

    #[cfg(unix)]
    #[test]
    fn kills_sleeping_process_promptly_on_wall_and_idle_timeout() {
        let wall_start = Instant::now();
        let wall_err = ProcessCommand::new("sh")
            .args(["-c", "sleep 5"])
            .wall_timeout(Duration::from_millis(100))
            .idle_timeout(Duration::from_secs(5))
            .run()
            .unwrap_err();
        let wall_elapsed = wall_start.elapsed();
        assert!(matches!(wall_err, PlatformError::TimedOut { kind: TimeoutKind::Wall, limit_ms: 100, .. }));
        assert!(wall_elapsed < Duration::from_secs(2), "wall timeout took too long: {wall_elapsed:?}");

        let idle_start = Instant::now();
        let idle_err = ProcessCommand::new("sh")
            .args(["-c", "sleep 5"])
            .wall_timeout(Duration::from_secs(5))
            .idle_timeout(Duration::from_millis(100))
            .run()
            .unwrap_err();
        let idle_elapsed = idle_start.elapsed();
        assert!(matches!(idle_err, PlatformError::TimedOut { kind: TimeoutKind::Idle, limit_ms: 100, .. }));
        assert!(idle_elapsed < Duration::from_secs(2), "idle timeout took too long: {idle_elapsed:?}");
    }

    #[cfg(unix)]
    #[test]
    fn capped_stdout_terminates_without_deadlock() {
        let start = Instant::now();
        let err = ProcessCommand::new("sh")
            .args(["-c", "yes curvebook_output_stream"])
            .wall_timeout(Duration::from_secs(5))
            .idle_timeout(Duration::from_secs(5))
            .max_output_bytes(1024)
            .run()
            .unwrap_err();
        assert!(matches!(err, PlatformError::OutputLimitExceeded { stream: "stdout", limit_bytes: 1024, .. }));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn discard_output_returns_immediately_when_backgrounded_child_stays_alive() {
        let start = Instant::now();
        let output = ProcessCommand::new("sh")
            .args(["-c", "(sleep 5) &"])
            .wall_timeout(Duration::from_secs(4))
            .idle_timeout(Duration::from_millis(200))
            .discard_output()
            .run()
            .expect("discard_output should not block on backgrounded grandchild");
        assert!(output.success);
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
        assert!(start.elapsed() < Duration::from_secs(2), "elapsed too long: {:?}", start.elapsed());
    }

    #[cfg(unix)]
    #[test]
    fn timeout_enforces_deadline_even_when_stdin_pipe_stalls_after_stdio_eof() {
        let huge_stdin = vec![b'x'; 2 * 1024 * 1024];
        let start = Instant::now();
        let err = ProcessCommand::new("sh")
            .args(["-c", "(sleep 10) & exec 1>&- 2>&-"])
            .stdin_bytes(huge_stdin)
            .wall_timeout(Duration::from_millis(120))
            .idle_timeout(Duration::from_millis(120))
            .run()
            .unwrap_err();
        let elapsed = start.elapsed();
        assert!(matches!(err, PlatformError::TimedOut { .. }), "unexpected error: {err:?}");
        assert!(elapsed < Duration::from_secs(2), "stalled stdin bypassed timeout: {elapsed:?}");
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_process_group_descendant_holding_pipes_after_child_exits() {
        let dir = std::env::temp_dir().join(format!("cb-proc-pgid-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let pid_file = dir.join("descendant.pid");
        let _ = std::fs::remove_file(&pid_file);

        let start = Instant::now();
        let err = ProcessCommand::new("sh")
            .args(["-c", "sleep 10 & printf '%s' \"$!\" > \"$CB_PID_FILE\"; exit 0"])
            .env("CB_PID_FILE", pid_file.to_string_lossy().as_ref())
            .wall_timeout(Duration::from_millis(500))
            .idle_timeout(Duration::from_secs(5))
            .run()
            .unwrap_err();
        let elapsed = start.elapsed();
        assert!(matches!(err, PlatformError::TimedOut { kind: TimeoutKind::Wall, .. }), "unexpected error: {err:?}");
        assert!(elapsed < Duration::from_secs(2), "timeout blocked on descendant pipe: {elapsed:?}");

        let pid_str = std::fs::read_to_string(&pid_file).expect("grandchild pid should be recorded").trim().to_string();
        assert!(!pid_str.is_empty(), "empty descendant pid");
        let reap_deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let alive = std::process::Command::new("kill")
                .args(["-0", &pid_str])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !alive {
                break;
            }
            assert!(Instant::now() < reap_deadline, "descendant pid {pid_str} survived process-group kill");
            std::thread::sleep(Duration::from_millis(10));
        }

        let _ = std::fs::remove_file(&pid_file);
        let _ = std::fs::remove_dir(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn output_limit_terminates_cleanly_even_with_large_stdin() {
        let huge_stdin = vec![b'y'; 2 * 1024 * 1024];
        let start = Instant::now();
        let err = ProcessCommand::new("sh")
            .args(["-c", "yes curvebook_output_stream"])
            .stdin_bytes(huge_stdin)
            .wall_timeout(Duration::from_secs(5))
            .idle_timeout(Duration::from_secs(5))
            .max_output_bytes(512)
            .run()
            .unwrap_err();
        assert!(matches!(err, PlatformError::OutputLimitExceeded { stream: "stdout", limit_bytes: 512, .. }));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
