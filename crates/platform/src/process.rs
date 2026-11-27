//! Bounded child-process runner with concurrent stream draining, stdin pipe closing,
//! wall-clock and idle timeouts, output byte caps, and guaranteed drop-kill cleanup.

#[cfg(not(target_arch = "wasm32"))]
use std::ffi::OsString;
#[cfg(not(target_arch = "wasm32"))]
use std::io::{ErrorKind, Read, Write};
#[cfg(all(not(target_arch = "wasm32"), unix))]
use std::os::unix::fs::PermissionsExt as _;
#[cfg(all(not(target_arch = "wasm32"), unix))]
use std::os::unix::process::CommandExt as _;
#[cfg(all(not(target_arch = "wasm32"), windows))]
use std::os::windows::process::CommandExt as _;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::process::{Child, Command, ExitStatus, Stdio};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{self, RecvTimeoutError};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Arc, OnceLock};
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

/// Windows `CreateProcess` flag (`CREATE_NEW_PROCESS_GROUP`) isolating child process-group signals.
pub const WINDOWS_CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

/// Windows `CreateProcess` flag (`CREATE_NO_WINDOW`) preventing transient console windows for helper processes.
pub const WINDOWS_CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Combined Windows `creation_flags` applied when spawning child processes.
#[must_use]
pub const fn windows_child_creation_flags() -> u32 {
    WINDOWS_CREATE_NEW_PROCESS_GROUP | WINDOWS_CREATE_NO_WINDOW
}

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
///
/// Child processes inherit a sanitized environment with a pinned system `PATH` (`DEFAULT_SAFE_UNIX_PATH`
/// on Unix; system `System32` / `Windows` directories on Windows) rather than the parent process's untrusted `PATH`. Callers that deliberately need an
/// executable outside those directories may pass an explicit executable path or override `PATH`
/// via [`Self::env`].
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
    /// Useful for clipboard owners (`wl-copy`, `xclip`) and desktop launchers (`xdg-open`,
    /// `open`) that fork a background server or viewer process. Because no output streams are
    /// captured, `idle_timeout` is disabled and only `wall_timeout` applies.
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
        let sanitized_env = build_sanitized_env_from(std::env::vars_os(), &self.env_vars, &self.env_remove);
        let executable = resolve_program_path(&self.program, &sanitized_env)?;
        let mut cmd = Command::new(&executable);
        cmd.env_clear();
        for (key, value) in &sanitized_env {
            cmd.env(key, value);
        }
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
        cmd.creation_flags(windows_child_creation_flags());
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

/// Return a cached, non-secret local host/machine fingerprint (`"<os>:<arch>:<host>"`) when
/// running on a native OS, or `None` on `wasm32` / when unavailable.
#[must_use]
pub fn local_host_fingerprint() -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static CACHED_HOST: OnceLock<Option<String>> = OnceLock::new();
        CACHED_HOST.get_or_init(detect_local_host_fingerprint).clone()
    }
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn sanitize_host_token(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let clean: String = trimmed.chars().filter(|ch| !ch.is_control() && *ch != '/').take(128).collect();
    let normalized = clean.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    Some(format!("{}:{}:{normalized}", std::env::consts::OS, std::env::consts::ARCH))
}

#[cfg(all(not(target_arch = "wasm32"), unix))]
fn detect_local_host_fingerprint() -> Option<String> {
    for path in ["/etc/machine-id", "/var/lib/dbus/machine-id", "/proc/sys/kernel/hostname"] {
        if let Ok(raw) = crate::fs::read_regular_file_utf8(Path::new(path), 256)
            && let Some(fp) = sanitize_host_token(&raw)
        {
            return Some(fp);
        }
    }
    for (bin, args) in [("/bin/hostname", &[][..]), ("/usr/bin/uname", &["-n"][..])] {
        if let Ok(out) =
            ProcessCommand::new(bin).args(args).wall_timeout(Duration::from_secs(2)).idle_timeout(Duration::from_secs(2)).max_output_bytes(1024).run()
            && out.success
            && let Ok(text) = String::from_utf8(out.stdout)
            && let Some(fp) = sanitize_host_token(&text)
        {
            return Some(fp);
        }
    }
    None
}

#[cfg(all(not(target_arch = "wasm32"), windows))]
fn detect_local_host_fingerprint() -> Option<String> {
    std::env::var("COMPUTERNAME").ok().and_then(|raw| sanitize_host_token(&raw))
}

#[cfg(all(not(target_arch = "wasm32"), not(any(unix, windows))))]
fn detect_local_host_fingerprint() -> Option<String> {
    None
}

/// Pinned system-standard Unix `PATH` used for sanitized child environments.
///
/// # Security & Portability Trade-off
/// Inheriting the parent process's `PATH` allows untrusted current-directory (`.`), user-writable,
/// or temporary directories to shadow OS helpers (`sh`, `id`, `xdg-open`, `lp`, `xclip`). Pinning
/// `PATH` prevents hijacking while still supporting standard OS package layouts:
/// - Core OS directories (`/usr/bin`, `/bin`, `/usr/sbin`, `/sbin`) are searched **first** so
///   system-protected binaries always take precedence over local overrides.
/// - `/usr/local/bin` supports FreeBSD ports/packages (where desktop utilities such as `xdg-open`,
///   `wl-copy`, `xclip`, and CUPS `lp` are installed) and local admin installs; on hosts where
///   `/usr/local/bin` is locally writable (e.g. Homebrew on Intel macOS), it only resolves binaries
///   absent from `/usr/bin:/bin:/usr/sbin:/sbin`.
/// - `/run/current-system/sw/bin` (NixOS) and `/run/current-system/profile/bin` (GNU Guix) are
///   root-managed immutable system profiles where those distributions expose desktop utilities.
#[cfg(all(not(target_arch = "wasm32"), unix))]
const DEFAULT_SAFE_UNIX_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin:/usr/local/bin:/run/current-system/sw/bin:/run/current-system/profile/bin";

#[cfg(not(target_arch = "wasm32"))]
fn env_keys_match(a: &str, b: &str) -> bool {
    #[cfg(windows)]
    {
        a.eq_ignore_ascii_case(b)
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn set_env_entry(entries: &mut Vec<(String, String)>, key: String, value: String) {
    if let Some((_, existing_val)) = entries.iter_mut().find(|(k, _)| env_keys_match(k, &key)) {
        *existing_val = value;
    } else {
        entries.push((key, value));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn remove_env_entry(entries: &mut Vec<(String, String)>, key: &str) {
    entries.retain(|(k, _)| !env_keys_match(k, key));
}

#[cfg(not(target_arch = "wasm32"))]
fn is_safe_scalar_env_value(val: &str, max_len: usize) -> bool {
    !val.is_empty() && val.len() <= max_len && !val.chars().any(char::is_control)
}

#[cfg(all(not(target_arch = "wasm32"), unix))]
fn is_safe_unix_abs_path(val: &str) -> bool {
    is_safe_scalar_env_value(val, 1024) && val.starts_with('/') && !val.split('/').any(|seg| seg == "..")
}

#[cfg(all(not(target_arch = "wasm32"), unix))]
fn allowlisted_parent_env_entry(key: &str, val: &str) -> bool {
    if key.starts_with("DYLD_") || key.starts_with("LD_") {
        return false;
    }
    match key {
        "TMPDIR" | "HOME" | "XDG_RUNTIME_DIR" | "XAUTHORITY" => is_safe_unix_abs_path(val),
        "LANG" | "LC_CTYPE" | "LC_ALL" | "TZ" | "CUPS_SERVER" | "DISPLAY" | "WAYLAND_DISPLAY" | "__CF_USER_TEXT_ENCODING" => {
            is_safe_scalar_env_value(val, 256)
        }
        _ => false,
    }
}

#[cfg(all(not(target_arch = "wasm32"), windows))]
fn allowlisted_parent_env_entry(key: &str, val: &str) -> bool {
    if !is_safe_scalar_env_value(val, 512) {
        return false;
    }
    ["SystemRoot", "WINDIR", "SystemDrive", "COMSPEC", "PATHEXT", "TEMP", "TMP", "USERPROFILE", "APPDATA", "LOCALAPPDATA"]
        .iter()
        .any(|allowed| key.eq_ignore_ascii_case(allowed))
}

#[cfg(all(not(target_arch = "wasm32"), not(any(unix, windows))))]
fn allowlisted_parent_env_entry(_key: &str, _val: &str) -> bool {
    false
}

#[cfg(all(not(target_arch = "wasm32"), windows))]
fn resolve_windows_system_root_from_pairs(pairs: &[(String, String)]) -> String {
    for candidate_key in ["SystemRoot", "WINDIR"] {
        if let Some((_, val)) = pairs.iter().find(|(k, _)| k.eq_ignore_ascii_case(candidate_key))
            && is_safe_scalar_env_value(val, 260)
        {
            return val.trim_end_matches(['\\', '/']).to_owned();
        }
    }
    String::from(r"C:\Windows")
}

#[cfg(not(target_arch = "wasm32"))]
fn build_sanitized_env_from<I>(parent_env: I, explicit_vars: &[(String, String)], explicit_remove: &[String]) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    let mut parent_utf8 = Vec::new();
    for (raw_k, raw_v) in parent_env {
        if let (Ok(k), Ok(v)) = (raw_k.into_string(), raw_v.into_string())
            && !k.is_empty()
            && !k.contains('=')
            && !k.contains('\0')
            && !v.contains('\0')
        {
            parent_utf8.push((k, v));
        }
    }

    let mut out = Vec::new();
    #[cfg(unix)]
    set_env_entry(&mut out, String::from("PATH"), String::from(DEFAULT_SAFE_UNIX_PATH));
    #[cfg(windows)]
    {
        let root = resolve_windows_system_root_from_pairs(&parent_utf8);
        let safe_path = format!(r"{root}\System32;{root};{root}\System32\Wbem;{root}\System32\WindowsPowerShell\v1.0");
        set_env_entry(&mut out, String::from("PATH"), safe_path);
    }

    for (key, val) in parent_utf8 {
        if allowlisted_parent_env_entry(&key, &val) {
            set_env_entry(&mut out, key, val);
        }
    }
    for key in explicit_remove {
        remove_env_entry(&mut out, key);
    }
    for (key, val) in explicit_vars {
        if !key.is_empty() && !key.contains('=') && !key.contains('\0') && !val.contains('\0') {
            set_env_entry(&mut out, key.clone(), val.clone());
        }
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
fn has_explicit_path_separator(program: &str) -> bool {
    #[cfg(windows)]
    {
        program.contains('/') || program.contains('\\') || program.contains(':')
    }
    #[cfg(not(windows))]
    {
        program.contains('/')
    }
}

#[cfg(all(not(target_arch = "wasm32"), unix))]
fn is_executable_regular_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && (meta.permissions().mode() & 0o111) != 0)
}

#[cfg(all(not(target_arch = "wasm32"), not(unix)))]
fn is_executable_regular_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file())
}

#[cfg(not(target_arch = "wasm32"))]
fn resolve_program_path(program: &str, sanitized_env: &[(String, String)]) -> Result<PathBuf, PlatformError> {
    if program.is_empty() || program.contains('\0') {
        return Err(PlatformError::ProgramNotFound { program: program.to_owned() });
    }
    if has_explicit_path_separator(program) {
        return Ok(PathBuf::from(program));
    }
    let Some((_, path_val)) = sanitized_env.iter().find(|(k, _)| env_keys_match(k, "PATH")) else {
        return Err(PlatformError::ProgramNotFound { program: program.to_owned() });
    };
    #[cfg(windows)]
    let pathext: Vec<String> = sanitized_env
        .iter()
        .find(|(k, _)| env_keys_match(k, "PATHEXT"))
        .map(|(_, v)| v.as_str())
        .unwrap_or(".COM;.EXE;.BAT;.CMD;.PS1")
        .split(';')
        .map(str::trim)
        .filter(|ext| ext.starts_with('.'))
        .map(str::to_owned)
        .collect();

    for dir in std::env::split_paths(path_val) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let direct = dir.join(program);
        if is_executable_regular_file(&direct) {
            return Ok(direct);
        }
        #[cfg(windows)]
        if Path::new(program).extension().is_none() {
            for ext in &pathext {
                let with_ext = dir.join(format!("{program}{ext}"));
                if is_executable_regular_file(&with_ext) {
                    return Ok(with_ext);
                }
            }
        }
    }
    Err(PlatformError::ProgramNotFound { program: program.to_owned() })
}

/// Drop guard ensuring child processes and their process groups are terminated on timeout,
/// output-cap overflow, error, or panic unwind.
///
/// # PID-Reuse & Platform Notes (`#![forbid(unsafe_code)]`)
/// - **Windows:** While `child` is `Some`, its open OS process `HANDLE` pins the PID so the
///   kernel cannot reuse it while `taskkill.exe /PID <pid> /T /F` runs. As soon as [`Self::try_wait`]
///   reaps the child and drops the `HANDLE`, `pgid` is cleared to `None` so a subsequently reused
///   PID can never be signaled. Because this crate enforces `#![forbid(unsafe_code)]`, it cannot
///   invoke raw Win32 Job Object FFI (`CreateJobObjectW`/`AssignProcessToJobObject`) to track
///   orphaned grandchildren after the direct child has already exited and been reaped.
/// - **Unix:** The child leads its own process group (`process_group(0)`). While any descendant
///   remains in that process group, POSIX reserves the PGID against reuse so `kill -s KILL -- -<pgid>`
///   reaps pipe-holding descendants on timeout. Once all streams are closed after reap (or on
///   [`Self::disarm`]), `pgid` is cleared to avoid signaling a recycled PGID.
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
        self.pgid = None;
    }

    fn clear_pgid(&mut self) {
        self.pgid = None;
    }

    fn clear_pgid_after_reap(&mut self, clear_immediately: bool) {
        if clear_immediately {
            self.pgid = None;
        }
    }

    fn try_wait(&mut self) -> Result<Option<ExitStatus>, std::io::Error> {
        let Some(child) = self.child.as_mut() else {
            return Ok(None);
        };
        match child.try_wait()? {
            Some(status) => {
                self.child = None;
                // On Windows, dropping `Child` closes the process HANDLE and immediately allows
                // PID reuse; clear `pgid` so `taskkill /PID` can never target a reused PID.
                self.clear_pgid_after_reap(cfg!(windows));
                Ok(Some(status))
            }
            None => Ok(None),
        }
    }

    fn kill_group_and_wait(&mut self) {
        if self.disarmed {
            return;
        }
        // Kill the process group/tree BEFORE dropping `self.child` so the open OS handle/zombie
        // still pins the PID/PGID against reuse during termination.
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
    for bin in ["/bin/kill", "/usr/bin/kill", "/run/current-system/sw/bin/kill", "/run/current-system/profile/bin/kill"] {
        let mut cmd = Command::new(bin);
        cmd.env_clear().args(["-s", "KILL", "--", &target]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        match cmd.spawn() {
            Ok(mut killer) => {
                let _ = killer.wait();
                return;
            }
            Err(err) if err.kind() == ErrorKind::NotFound => continue,
            Err(_) => return,
        }
    }
}

#[cfg(all(not(target_arch = "wasm32"), windows))]
fn kill_process_group(pgid: u32) {
    if pgid <= 1 || pgid == std::process::id() {
        return;
    }
    let system_root = std::env::var("SystemRoot")
        .or_else(|_| std::env::var("WINDIR"))
        .ok()
        .filter(|v| is_safe_scalar_env_value(v, 260))
        .unwrap_or_else(|| String::from(r"C:\Windows"));
    let taskkill_path = PathBuf::from(system_root.trim_end_matches(['\\', '/'])).join("System32").join("taskkill.exe");
    let pid_str = pgid.to_string();
    let mut cmd = Command::new(taskkill_path);
    cmd.env_clear()
        .args(["/PID", &pid_str, "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(WINDOWS_CREATE_NO_WINDOW);
    if let Ok(mut killer) = cmd.spawn() {
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
    enforce_idle_timeout: bool,
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
            enforce_idle_timeout: cmd.capture_stdout || cmd.capture_stderr,
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

    fn poll_child_exit(&mut self, guard: &mut ChildGuard) -> Result<(), PlatformError> {
        if self.exit_status.is_none()
            && let Some(status) =
                guard.try_wait().map_err(|err| PlatformError::Io { context: format!("wait `{}`", self.program), message: err.to_string() })?
        {
            self.exit_status = Some(status);
        }
        Ok(())
    }

    fn try_finish_on_exit(&mut self, guard: &mut ChildGuard) -> Result<Option<ExitStatus>, PlatformError> {
        if self.stdout_eof
            && self.stderr_eof
            && self.stdin_done
            && let Some(status) = self.exit_status
        {
            guard.clear_pgid();
            return Ok(Some(status));
        }

        Ok(None)
    }

    fn wait_for_completion(
        &mut self,
        guard: &mut ChildGuard,
        rx: &mpsc::Receiver<StreamEvent>,
        cancelled: &AtomicBool,
    ) -> Result<ExitStatus, PlatformError> {
        loop {
            self.poll_child_exit(guard)?;
            if let Some(status) = self.try_finish_on_exit(guard)? {
                return Ok(status);
            }

            if self.stdout_eof && self.stderr_eof && self.stdin_done {
                let remaining = self.remaining_until_deadline()?;
                std::thread::sleep(remaining.min(EXIT_POLL_INTERVAL));
                continue;
            }

            let remaining = self.remaining_until_deadline()?;
            match rx.recv_timeout(remaining) {
                Ok(event) => self.handle_event(event)?,
                Err(RecvTimeoutError::Timeout) => {
                    // Re-poll child exit before declaring timeout so a process that finished right
                    // as `wait_slice` elapsed does not spuriously lose to an idle/wall deadline.
                    self.poll_child_exit(guard)?;
                    if let Some(status) = self.try_finish_on_exit(guard)? {
                        return Ok(status);
                    }
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
        let next = if self.enforce_idle_timeout { self.wall_deadline.min(self.idle_deadline) } else { self.wall_deadline };
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
        if self.enforce_idle_timeout && now >= self.idle_deadline {
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
                    if cancelled.load(Ordering::Acquire) {
                        break;
                    }
                    total = total.saturating_add(n);
                    if total > max_bytes {
                        let _ = tx.send(StreamEvent::LimitExceeded(stream));
                        break;
                    }
                    let Some(chunk) = buf.get(..n) else {
                        let _ = tx.send(StreamEvent::IoErr { stream, message: format!("reader returned {n} bytes for {}-byte buffer", buf.len()) });
                        break;
                    };
                    if tx.send(StreamEvent::Chunk { stream, bytes: chunk.to_vec() }).is_err() {
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

    #[test]
    fn windows_child_creation_flags_include_process_group_and_no_window() {
        let flags = super::windows_child_creation_flags();
        assert_eq!(super::WINDOWS_CREATE_NEW_PROCESS_GROUP, 0x0000_0200);
        assert_eq!(super::WINDOWS_CREATE_NO_WINDOW, 0x0800_0000);
        assert_ne!(flags & super::WINDOWS_CREATE_NEW_PROCESS_GROUP, 0);
        assert_ne!(flags & super::WINDOWS_CREATE_NO_WINDOW, 0);
        assert_eq!(flags, super::WINDOWS_CREATE_NEW_PROCESS_GROUP | super::WINDOWS_CREATE_NO_WINDOW);
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
            let alive = std::process::Command::new("/bin/kill")
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

    #[test]
    fn sanitized_environment_strips_secrets_loader_vars_and_untrusted_path() {
        use std::ffi::OsString;

        use super::{build_sanitized_env_from, local_host_fingerprint};

        let fake_parent = vec![
            (OsString::from("LD_PRELOAD"), OsString::from("/tmp/evil.so")),
            (OsString::from("LD_LIBRARY_PATH"), OsString::from("/tmp/evil-lib")),
            (OsString::from("DYLD_INSERT_LIBRARIES"), OsString::from("/tmp/evil.dylib")),
            (OsString::from("DYLD_LIBRARY_PATH"), OsString::from("/tmp/evil-dyld")),
            (OsString::from("DYLD_FRAMEWORK_PATH"), OsString::from("/tmp/evil-fw")),
            (OsString::from("CURVEBOOK_CONTROL_TOKEN"), OsString::from("secret-control-token")),
            (OsString::from("AWS_SECRET_ACCESS_KEY"), OsString::from("aws-secret")),
            (OsString::from("GITHUB_TOKEN"), OsString::from("dummy-git-credential-placeholder")),
            (OsString::from("ARBITRARY_USER_VAR"), OsString::from("leaked")),
            (OsString::from("PATH"), OsString::from("/tmp/attacker-bin:/usr/bin:/bin")),
            (OsString::from("LANG"), OsString::from("en_US.UTF-8")),
            (OsString::from("TMPDIR"), OsString::from("/tmp/safe-sub")),
            (OsString::from("HOME"), OsString::from("/tmp/../escape")),
        ];
        let overrides = vec![(String::from("EXPLICIT_KEY"), String::from("explicit-val"))];
        let removals = vec![String::from("TMPDIR")];
        let env = build_sanitized_env_from(fake_parent, &overrides, &removals);

        for forbidden in [
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "DYLD_INSERT_LIBRARIES",
            "DYLD_LIBRARY_PATH",
            "DYLD_FRAMEWORK_PATH",
            "CURVEBOOK_CONTROL_TOKEN",
            "AWS_SECRET_ACCESS_KEY",
            "GITHUB_TOKEN",
            "ARBITRARY_USER_VAR",
            "TMPDIR",
            "HOME",
        ] {
            assert!(!env.iter().any(|(k, _)| k == forbidden), "forbidden variable `{forbidden}` leaked into child env: {env:?}");
        }
        let path_val = env.iter().find(|(k, _)| k.eq_ignore_ascii_case("PATH")).map(|(_, v)| v.as_str()).expect("pinned PATH present");
        assert!(!path_val.contains("attacker-bin"), "untrusted parent PATH leaked: {path_val}");
        assert!(env.iter().any(|(k, v)| k == "EXPLICIT_KEY" && v == "explicit-val"));
        #[cfg(unix)]
        {
            assert_eq!(path_val, "/usr/bin:/bin:/usr/sbin:/sbin:/usr/local/bin:/run/current-system/sw/bin:/run/current-system/profile/bin");
            assert!(env.iter().any(|(k, v)| k == "LANG" && v == "en_US.UTF-8"));
        }

        let fp = local_host_fingerprint().expect("native host fingerprint");
        assert!(fp.starts_with(std::env::consts::OS));
        assert!(fp.split(':').count() >= 3);
    }

    #[cfg(unix)]
    #[test]
    fn end_to_end_child_process_receives_cleaned_env_and_safe_kill_path() {
        use std::os::unix::fs::PermissionsExt as _;

        if std::env::var_os("CB_RV06_INJECTED_PARENT_MODE").is_some() {
            assert_eq!(std::env::var("CURVEBOOK_CONTROL_TOKEN").as_deref(), Ok("top-secret-bearer-token"));
            let marker = std::env::var("CB_RV06_MARKER_FILE").expect("marker path");

            let env_out = ProcessCommand::new("sh").args(["-c", "env"]).env("CB_EXPLICIT_OK", "allowed-1").run().expect("run sh env");
            assert!(env_out.success);
            let child_env = String::from_utf8_lossy(&env_out.stdout);
            assert!(!child_env.contains("top-secret-bearer-token"), "control token leaked: {child_env}");
            assert!(!child_env.contains("CURVEBOOK_CONTROL_TOKEN="), "token key leaked: {child_env}");
            assert!(!child_env.contains("AWS_SECRET_ACCESS_KEY="), "aws secret leaked: {child_env}");
            assert!(!child_env.contains("CB_ARBITRARY_SHELL_VAR="), "arbitrary var leaked: {child_env}");
            assert!(!child_env.contains("cb-rv06-hijack"), "untrusted PATH leaked: {child_env}");
            assert!(child_env.contains("CB_EXPLICIT_OK=allowed-1"), "explicit env missing: {child_env}");

            let id_out = ProcessCommand::new("id").arg("-u").run().expect("run system id");
            assert!(id_out.success);
            assert!(String::from_utf8_lossy(&id_out.stdout).trim().parse::<u32>().is_ok(), "hijacked `id` executed instead of system `id`");

            let _ = ProcessCommand::new("sh")
                .args(["-c", "sleep 5"])
                .wall_timeout(Duration::from_millis(80))
                .idle_timeout(Duration::from_secs(2))
                .run()
                .expect_err("sleep must time out");
            assert!(!std::path::Path::new(&marker).exists(), "hostile PATH hijacked `kill` or `id`!");
            return;
        }

        let hijack_dir = std::env::temp_dir().join(format!("cb-rv06-hijack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&hijack_dir);
        std::fs::create_dir_all(&hijack_dir).expect("create hijack dir");
        let marker_file = hijack_dir.join("hijacked.txt");
        for fake_bin in ["kill", "id"] {
            let script_path = hijack_dir.join(fake_bin);
            let script = format!("#!/bin/sh\necho hijacked > '{}'\necho 999999\nexit 0\n", marker_file.display());
            std::fs::write(&script_path, script).expect("write fake script");
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).expect("chmod +x");
        }

        let orig_path = std::env::var("PATH").unwrap_or_else(|_| String::from("/usr/bin:/bin"));
        let poisoned_path = format!("{}:{orig_path}", hijack_dir.display());
        let status = std::process::Command::new(std::env::current_exe().expect("current test exe"))
            .args(["--exact", "process::tests::end_to_end_child_process_receives_cleaned_env_and_safe_kill_path", "--nocapture"])
            .env("CB_RV06_INJECTED_PARENT_MODE", "1")
            .env("CB_RV06_MARKER_FILE", &marker_file)
            .env("CURVEBOOK_CONTROL_TOKEN", "top-secret-bearer-token")
            .env("AWS_SECRET_ACCESS_KEY", "fake-aws-secret-value")
            .env("CB_ARBITRARY_SHELL_VAR", "should-not-inherit")
            .env("PATH", poisoned_path)
            .status()
            .expect("spawn re-entrant env check");
        let marker_existed = marker_file.exists();
        let _ = std::fs::remove_dir_all(&hijack_dir);
        assert!(status.success(), "re-entrant subprocess env check failed: {status}");
        assert!(!marker_existed, "hostile PATH script was invoked");
    }

    #[cfg(unix)]
    #[test]
    fn gap_04_01_discard_output_does_not_enforce_idle_timeout_and_respects_wall_timeout() {
        // With `discard_output()`, zero bytes can ever arrive on stdout/stderr, so `idle_timeout`
        // must NOT truncate `wall_timeout` while a launcher is still running.
        let ok = ProcessCommand::new("sh")
            .args(["-c", "sleep 0.25"])
            .wall_timeout(Duration::from_secs(3))
            .idle_timeout(Duration::from_millis(60))
            .discard_output()
            .run()
            .expect("discard_output must not fire idle timeout before wall timeout");
        assert!(ok.success);

        // `wall_timeout` still terminates a hung `discard_output()` child promptly.
        let wall_start = Instant::now();
        let err = ProcessCommand::new("sh")
            .args(["-c", "sleep 5"])
            .wall_timeout(Duration::from_millis(120))
            .idle_timeout(Duration::from_millis(40))
            .discard_output()
            .run()
            .unwrap_err();
        assert!(matches!(err, PlatformError::TimedOut { kind: TimeoutKind::Wall, limit_ms: 120, .. }), "unexpected err: {err:?}");
        assert!(wall_start.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn gap_04_02_child_guard_clears_pgid_on_disarm_and_windows_reap_policy() {
        use super::ChildGuard;

        let child = std::process::Command::new("sh")
            .args(["-c", "exit 0"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn short-lived child");
        let mut guard = ChildGuard::new(child);
        assert!(guard.pgid.is_some());
        // Simulate Windows post-reap guard policy: clearing pgid prevents signaling a reused PID.
        guard.clear_pgid_after_reap(true);
        assert!(guard.pgid.is_none());
        let deadline = Instant::now() + Duration::from_secs(2);
        while guard.try_wait().expect("try_wait").is_none() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        guard.disarm();
        assert!(guard.pgid.is_none());
    }
}
