//! Host platform safety primitives for LumenCraft: atomic filesystem writes, bounded
//! subprocess execution, and canonicalized `--workdir` path containment.
//!
//! Layer L0 crate: depends only on `std`, `serde`, and `serde_json`. Compiles cleanly on
//! `wasm32` targets with host-only OS operations gated behind `#[cfg(not(target_arch = "wasm32"))]`.
#![forbid(unsafe_code)]

pub mod fs;
pub mod path_guard;
pub mod process;

use std::fmt;

#[cfg(not(target_arch = "wasm32"))]
pub use fs::{read_regular_file_bytes, read_regular_file_bytes_if_exists, read_regular_file_utf8, write_atomic, write_atomic_with_hook};
pub use path_guard::PATH_PARAM_KEYS;
#[cfg(not(target_arch = "wasm32"))]
pub use path_guard::{PathResolver, WorkdirGuard, ensure_regular_file};
#[cfg(not(target_arch = "wasm32"))]
pub use process::{DEFAULT_IDLE_TIMEOUT, DEFAULT_MAX_OUTPUT_BYTES, DEFAULT_WALL_TIMEOUT, ProcessCommand, ProcessOutput};

/// Kind of timeout deadline exceeded while executing a child process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutKind {
    /// Total elapsed wall-clock limit since process spawn was exceeded.
    Wall,
    /// No bytes were received from `stdout` or `stderr` within the idle window.
    Idle,
}

impl fmt::Display for TimeoutKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wall => f.write_str("wall"),
            Self::Idle => f.write_str("idle"),
        }
    }
}

/// Structured error returned by `lumencraft-platform` filesystem, subprocess, and path-guard helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformError {
    /// Target executable could not be found on `PATH` or at the specified location.
    ProgramNotFound { program: String },
    /// Child process exceeded its wall-clock or idle timeout and was terminated.
    TimedOut { program: String, kind: TimeoutKind, limit_ms: u64 },
    /// Child process wrote more than `limit_bytes` to `stdout` or `stderr` and was terminated.
    OutputLimitExceeded { program: String, stream: &'static str, limit_bytes: usize },
    /// Target regular file exceeded `limit_bytes` before or during bounded reading.
    FileTooLarge { path: String, actual_bytes: Option<u64>, limit_bytes: usize },
    /// Input path or workdir configuration was empty, non-directory, or syntactically invalid.
    InvalidPath { reason: String },
    /// Path escaped or attempted to traverse outside the confined `workdir`.
    PathEscape { path: String, workdir: String },
    /// Underlying operating-system I/O failure with operation context.
    Io { context: String, message: String },
}

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProgramNotFound { program } => write!(f, "program `{program}` was not found"),
            Self::TimedOut { program, kind, limit_ms } => {
                write!(f, "program `{program}` exceeded {kind} timeout of {limit_ms}ms")
            }
            Self::OutputLimitExceeded { program, stream, limit_bytes } => {
                write!(f, "program `{program}` exceeded {stream} output cap of {limit_bytes} bytes")
            }
            Self::FileTooLarge { path, actual_bytes, limit_bytes } => match actual_bytes {
                Some(actual) => write!(f, "file `{path}` size {actual} bytes exceeds limit of {limit_bytes} bytes"),
                None => write!(f, "file `{path}` exceeds limit of {limit_bytes} bytes"),
            },
            Self::InvalidPath { reason } => write!(f, "invalid path: {reason}"),
            Self::PathEscape { path, workdir } => {
                write!(f, "path `{path}` escapes workdir `{workdir}`")
            }
            Self::Io { context, message } => write!(f, "{context}: {message}"),
        }
    }
}

impl std::error::Error for PlatformError {}

#[cfg(test)]
mod tests {
    use super::{PlatformError, TimeoutKind};

    #[test]
    fn error_variants_format_cleanly() {
        let missing = PlatformError::ProgramNotFound { program: String::from("missing-bin") };
        assert!(missing.to_string().contains("missing-bin"));

        let timeout = PlatformError::TimedOut { program: String::from("sleep"), kind: TimeoutKind::Wall, limit_ms: 100 };
        assert!(timeout.to_string().contains("wall timeout of 100ms"));

        let idle = PlatformError::TimedOut { program: String::from("sleep"), kind: TimeoutKind::Idle, limit_ms: 50 };
        assert!(idle.to_string().contains("idle timeout of 50ms"));

        let cap = PlatformError::OutputLimitExceeded { program: String::from("yes"), stream: "stdout", limit_bytes: 1024 };
        assert!(cap.to_string().contains("stdout output cap of 1024 bytes"));

        let large_known = PlatformError::FileTooLarge { path: String::from("huge.csv"), actual_bytes: Some(2048), limit_bytes: 1024 };
        assert!(large_known.to_string().contains("size 2048 bytes exceeds limit of 1024 bytes"));

        let large_stream = PlatformError::FileTooLarge { path: String::from("growing.csv"), actual_bytes: None, limit_bytes: 1024 };
        assert!(large_stream.to_string().contains("exceeds limit of 1024 bytes"));

        let invalid = PlatformError::InvalidPath { reason: String::from("empty") };
        assert!(invalid.to_string().contains("empty"));

        let escape = PlatformError::PathEscape { path: String::from("../x"), workdir: String::from("/wd") };
        assert!(escape.to_string().contains("escapes workdir"));

        let io = PlatformError::Io { context: String::from("write"), message: String::from("disk full") };
        assert!(io.to_string().contains("disk full"));
    }
}
