//! Filesystem safety primitives: crash-safe atomic writes and bounded regular-file reads.

#[cfg(not(target_arch = "wasm32"))]
use std::fs::{File, Metadata, OpenOptions, Permissions};
#[cfg(not(target_arch = "wasm32"))]
use std::io::{ErrorKind, Read, Write};
#[cfg(all(not(target_arch = "wasm32"), unix))]
use std::os::unix::fs::OpenOptionsExt as _;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, SystemTime};

#[cfg(not(target_arch = "wasm32"))]
use crate::PlatformError;

#[cfg(not(target_arch = "wasm32"))]
static TMP_FILE_SEQ: AtomicU64 = AtomicU64::new(1);

#[cfg(not(target_arch = "wasm32"))]
const MAX_TEMP_CREATE_ATTEMPTS: usize = 32;

#[cfg(not(target_arch = "wasm32"))]
const MAX_SYMLINK_HOPS: usize = 40;

/// Default minimum age (5 minutes) before an orphaned `.{stem}.tmp.{pid}.{seq}` sibling file
/// from a dead/other process is removed by automatic atomic-save cleanup.
#[cfg(not(target_arch = "wasm32"))]
pub const DEFAULT_STALE_TEMP_MAX_AGE: Duration = Duration::from_secs(300);

#[cfg(not(target_arch = "wasm32"))]
const MAX_AUTO_SWEEP_DIR_ENTRIES: usize = 1024;

/// Guard owning a freshly written, `sync_all`-ed sibling staging file.
///
/// Dropping a [`StagedSibling`] removes the staged file unless the caller either commits it via
/// [`Self::persist_to`] or explicitly takes ownership of its path via [`Self::into_path`].
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct StagedSibling {
    path: PathBuf,
    armed: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl StagedSibling {
    /// Borrow the filesystem path of the staged sibling file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Disarm automatic cleanup and hand off ownership of the staged file path to the caller.
    ///
    /// Multi-file transactional writers (such as unpacked project saves) use this handoff after
    /// recording the staged path in their own commit/recovery manifest.
    #[must_use]
    pub fn into_path(mut self) -> PathBuf {
        self.armed = false;
        std::mem::take(&mut self.path)
    }

    /// Atomically rename the staged sibling onto `target`, disarming cleanup on success.
    ///
    /// # Errors
    /// Returns [`PlatformError::Io`] if renaming fails (leaving the staged sibling armed for
    /// cleanup on drop).
    pub fn persist_to(mut self, target: &Path) -> Result<(), PlatformError> {
        std::fs::rename(&self.path, target).map_err(|err| io_err(&format!("rename `{}` to `{}`", self.path.display(), target.display()), &err))?;
        self.armed = false;
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for StagedSibling {
    fn drop(&mut self) {
        if self.armed && !self.path.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Create, write, permission-restore, and `sync_all` an exclusive sibling staging file in `parent`.
///
/// The staged file is named `.{sanitized_stem}.{tag}.{pid}.{seq}`, where `sanitized_stem` keeps
/// up to 48 ASCII alphanumeric / `.` / `-` / `_` characters from `target`'s file name (defaulting
/// to `"file"`). When `existing_perms` is `Some`, Unix creation starts with mode `0o600` before
/// restoring `existing_perms` so replacement writes never expose restricted files under a
/// permissive umask.
///
/// If writing, restoring permissions, or `sync_all` fails, the staged file is removed before
/// returning. On success, the returned [`StagedSibling`] remains armed for cleanup until committed
/// with [`StagedSibling::persist_to`] or explicitly handed off with [`StagedSibling::into_path`].
///
/// # Errors
/// Returns [`PlatformError::Io`] if unique sibling creation is exhausted or if writing, restoring
/// permissions, or syncing fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_staged_sibling(
    parent: &Path,
    target: &Path,
    tag: &str,
    bytes: &[u8],
    existing_perms: Option<Permissions>,
) -> Result<StagedSibling, PlatformError> {
    let stem = sanitize_sibling_stem(target);
    let safe_tag: String = tag.chars().filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_').take(16).collect();
    let effective_tag = if safe_tag.is_empty() { "tmp" } else { safe_tag.as_str() };
    let (mut file, staged_path) = create_unique_sibling_temp(parent, &stem, effective_tag, existing_perms.is_some())?;
    let staged = StagedSibling { path: staged_path, armed: true };

    file.write_all(bytes).map_err(|err| io_err(&format!("write temp file `{}`", staged.path.display()), &err))?;
    if let Some(perms) = existing_perms {
        file.set_permissions(perms).map_err(|err| io_err(&format!("set permissions on temp file `{}`", staged.path.display()), &err))?;
    }
    file.sync_all().map_err(|err| io_err(&format!("sync temp file `{}`", staged.path.display()), &err))?;
    drop(file);
    Ok(staged)
}

/// Atomically write `bytes` to `path` using a unique sibling temp file, `sync_all`, and `rename`.
///
/// # Errors
/// Returns [`PlatformError::InvalidPath`] if `path` has no target file name, or
/// [`PlatformError::Io`] if creating, writing, syncing, or renaming the temp file fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), PlatformError> {
    write_atomic_with_hook(path, bytes, |_| Ok(()))
}

/// Atomically write `bytes` to `path`, invoking `before_rename(&temp_path)` after `sync_all()`
/// and before `rename`.
///
/// If any step (including `before_rename`) returns an error, the sibling temp file is removed so
/// zero partial or temporary files remain and any existing file at `path` is left untouched.
///
/// # Errors
/// Returns [`PlatformError::InvalidPath`] if `path` has no target file name, or
/// [`PlatformError::Io`] if any I/O operation or `before_rename` fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_atomic_with_hook<F>(path: &Path, bytes: &[u8], before_rename: F) -> Result<(), PlatformError>
where
    F: FnOnce(&Path) -> Result<(), std::io::Error>,
{
    let _ = validate_target_path(path)?;
    let (resolved_target, existing_perms) = resolve_atomic_target(path)?;
    let parent = validate_target_path(&resolved_target)?;
    let staged = write_staged_sibling(parent, &resolved_target, "tmp", bytes, existing_perms)?;

    before_rename(staged.path()).map_err(|err| io_err(&format!("pre-rename hook for `{}`", resolved_target.display()), &err))?;
    staged.persist_to(&resolved_target)?;

    sync_dir(parent)?;
    // Best-effort cleanup of stale orphaned `.{stem}.tmp.{pid}.{seq}` siblings left behind by
    // prior hard crashes (SIGKILL/power loss) for the same target stem.
    let stem = sanitize_sibling_stem(&resolved_target);
    let _ = sweep_stale_temp_files_inner(parent, Some(&stem), DEFAULT_STALE_TEMP_MAX_AGE, MAX_AUTO_SWEEP_DIR_ENTRIES);
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn resolve_atomic_target(path: &Path) -> Result<(PathBuf, Option<Permissions>), PlatformError> {
    let lmeta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok((path.to_path_buf(), None)),
        Err(err) => return Err(io_err(&format!("inspect target `{}`", path.display()), &err)),
    };
    if !lmeta.file_type().is_symlink() {
        if !lmeta.file_type().is_file() {
            return Err(PlatformError::InvalidPath { reason: format!("target path `{}` exists and is not a regular file", path.display()) });
        }
        return Ok((path.to_path_buf(), Some(lmeta.permissions())));
    }
    resolve_symlink_target(path)
}

#[cfg(not(target_arch = "wasm32"))]
fn resolve_symlink_target(path: &Path) -> Result<(PathBuf, Option<Permissions>), PlatformError> {
    match std::fs::metadata(path) {
        Ok(meta) => {
            if !meta.file_type().is_file() {
                return Err(PlatformError::InvalidPath { reason: format!("target path `{}` resolves to a non-regular file", path.display()) });
            }
            let canonical = std::fs::canonicalize(path).map_err(|err| io_err(&format!("canonicalize symlink `{}`", path.display()), &err))?;
            Ok((canonical, Some(meta.permissions())))
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {
            let mut current = path.to_path_buf();
            for _ in 0..MAX_SYMLINK_HOPS {
                match std::fs::symlink_metadata(&current) {
                    Ok(meta) if meta.file_type().is_symlink() => {
                        let target =
                            std::fs::read_link(&current).map_err(|read_err| io_err(&format!("read symlink `{}`", current.display()), &read_err))?;
                        current = if target.is_relative() { resolve_parent_dir(&current).join(target) } else { target };
                    }
                    Ok(meta) if !meta.file_type().is_file() => {
                        return Err(PlatformError::InvalidPath {
                            reason: format!("target path `{}` resolves to a non-regular file", current.display()),
                        });
                    }
                    Ok(meta) => return Ok((current, Some(meta.permissions()))),
                    Err(not_found) if not_found.kind() == ErrorKind::NotFound => return Ok((current, None)),
                    Err(other) => return Err(io_err(&format!("inspect symlink target `{}`", current.display()), &other)),
                }
            }
            Err(PlatformError::Io {
                context: format!("resolve symlink `{}`", path.display()),
                message: String::from("too many levels of symbolic links"),
            })
        }
        Err(err) => Err(io_err(&format!("inspect symlink target `{}`", path.display()), &err)),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn validate_target_path(path: &Path) -> Result<&Path, PlatformError> {
    if path.as_os_str().is_empty() {
        return Err(PlatformError::InvalidPath { reason: String::from("target path must not be empty") });
    }
    if path.file_name().is_none() {
        return Err(PlatformError::InvalidPath { reason: format!("target path `{}` has no file name", path.display()) });
    }
    Ok(resolve_parent_dir(path))
}

#[cfg(not(target_arch = "wasm32"))]
fn sanitize_sibling_stem(target: &Path) -> String {
    let sanitized: String = target
        .file_name()
        .map(|name| {
            name.to_string_lossy().chars().filter(|ch| ch.is_ascii_alphanumeric() || *ch == '.' || *ch == '-' || *ch == '_').take(48).collect()
        })
        .unwrap_or_default();
    if sanitized.is_empty() { String::from("file") } else { sanitized }
}

#[cfg(not(target_arch = "wasm32"))]
fn resolve_parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn create_unique_sibling_temp(parent: &Path, stem: &str, tag: &str, restrict_mode: bool) -> Result<(File, PathBuf), PlatformError> {
    let pid = std::process::id();
    for _ in 0..MAX_TEMP_CREATE_ATTEMPTS {
        let seq = TMP_FILE_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp_path = parent.join(format!(".{stem}.{tag}.{pid}.{seq}"));
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        if restrict_mode {
            // Avoid briefly exposing restricted file contents under a permissive umask before chmod.
            opts.mode(0o600);
        }
        #[cfg(not(unix))]
        let _ = restrict_mode;
        match opts.open(&tmp_path) {
            Ok(file) => return Ok((file, tmp_path)),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(io_err(&format!("create temp file `{}`", tmp_path.display()), &err)),
        }
    }
    Err(PlatformError::Io {
        context: format!("create temp file in `{}`", parent.display()),
        message: String::from("exhausted unique sibling temp-file attempts"),
    })
}

/// Remove orphaned `.{stem}.tmp.{pid}.{seq}` staging files in `dir` that were left behind by a
/// hard crash (`SIGKILL` / power loss) between temp creation and atomic rename.
///
/// Safety guarantees against racing active writers:
/// - Only regular files (never symlinks or directories, verified via `symlink_metadata`) matching
///   the exact `.{stem}.tmp.{pid}.{seq}` naming pattern are considered.
/// - Files owned by the current process (`pid == std::process::id()`) are never removed.
/// - On Unix, any candidate whose PID is still alive (or cannot be proven absent due to `EPERM`)
///   is skipped.
/// - Every candidate must have a modification timestamp at least `max_age` in the past.
///
/// Returns the number of stale temp files removed.
///
/// # Errors
/// Returns [`PlatformError::InvalidPath`] if `dir` is empty or not a directory, or
/// [`PlatformError::Io`] if reading `dir` fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn sweep_stale_temp_files(dir: &Path, max_age: Duration) -> Result<usize, PlatformError> {
    if dir.as_os_str().is_empty() {
        return Err(PlatformError::InvalidPath { reason: String::from("directory path must not be empty") });
    }
    let meta = std::fs::metadata(dir).map_err(|err| io_err(&format!("inspect directory `{}`", dir.display()), &err))?;
    if !meta.is_dir() {
        return Err(PlatformError::InvalidPath { reason: format!("path `{}` must be a directory", dir.display()) });
    }
    sweep_stale_temp_files_inner(dir, None, max_age, usize::MAX)
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_sibling_temp_name(file_name: &str) -> Option<(&str, &str, u32, u64)> {
    let rest = file_name.strip_prefix('.')?;
    let (before_seq, seq_str) = rest.rsplit_once('.')?;
    let (before_pid, pid_str) = before_seq.rsplit_once('.')?;
    let (stem, tag) = before_pid.rsplit_once('.')?;
    if stem.is_empty() || tag.is_empty() {
        return None;
    }
    let pid: u32 = pid_str.parse().ok()?;
    let seq: u64 = seq_str.parse().ok()?;
    if pid == 0 || seq == 0 {
        return None;
    }
    Some((stem, tag, pid, seq))
}

#[cfg(all(not(target_arch = "wasm32"), unix))]
fn is_pid_alive(pid: u32) -> Option<bool> {
    if pid == std::process::id() {
        return Some(true);
    }
    if Path::new("/proc/self").is_dir() {
        return Some(Path::new(&format!("/proc/{pid}")).exists());
    }
    let pid_str = pid.to_string();
    for bin in ["/bin/kill", "/usr/bin/kill", "/run/current-system/sw/bin/kill", "/run/current-system/profile/bin/kill"] {
        let output = std::process::Command::new(bin)
            .env_clear()
            .args(["-0", "--", &pid_str])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .output();
        match output {
            Ok(out) => {
                if out.status.success() {
                    return Some(true);
                }
                let stderr = String::from_utf8_lossy(&out.stderr).to_ascii_lowercase();
                // Distinguish definite ESRCH ("no such process") from EPERM (another user's live PID).
                if stderr.contains("no such process") {
                    return Some(false);
                }
                return Some(true);
            }
            Err(err) if err.kind() == ErrorKind::NotFound => continue,
            Err(_) => return None,
        }
    }
    None
}

#[cfg(all(not(target_arch = "wasm32"), not(unix)))]
fn is_pid_alive(pid: u32) -> Option<bool> {
    if pid == std::process::id() { Some(true) } else { None }
}

#[cfg(not(target_arch = "wasm32"))]
fn sweep_stale_temp_files_inner(dir: &Path, stem_filter: Option<&str>, max_age: Duration, max_entries: usize) -> Result<usize, PlatformError> {
    let entries = std::fs::read_dir(dir).map_err(|err| io_err(&format!("read directory `{}`", dir.display()), &err))?;
    let now = SystemTime::now();
    let current_pid = std::process::id();
    let mut removed = 0usize;

    for (scanned, entry_res) in entries.enumerate() {
        if scanned >= max_entries {
            break;
        }
        let Ok(entry) = entry_res else {
            continue;
        };
        let os_name = entry.file_name();
        let Some(name) = os_name.to_str() else {
            continue;
        };
        let Some((stem, tag, pid, _seq)) = parse_sibling_temp_name(name) else {
            continue;
        };
        if tag != "tmp" || pid == current_pid {
            continue;
        }
        if let Some(expected_stem) = stem_filter
            && stem != expected_stem
        {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.file_type().is_file() {
            continue;
        }
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let Ok(elapsed) = now.duration_since(modified) else {
            continue;
        };
        if elapsed < max_age {
            continue;
        }
        if is_pid_alive(pid) == Some(true) {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            removed = removed.saturating_add(1);
        }
    }
    Ok(removed)
}

/// Read all bytes from an existing regular file at `path`, enforcing a hard `max_bytes` cap.
///
/// Inspects file metadata before opening, opens with `O_NONBLOCK` on Unix so a concurrent TOCTOU
/// swap to a FIFO cannot block `open(2)`, re-validates the open descriptor's metadata (`fstat`),
/// and reads through [`Read::take`] capped at `max_bytes + 1`.
///
/// # Errors
/// Returns [`PlatformError::InvalidPath`] when `path` is empty or does not refer to a regular
/// file, [`PlatformError::FileTooLarge`] when the file size exceeds `max_bytes`, or
/// [`PlatformError::Io`] on OS inspection, open, or read failure.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_regular_file_bytes(path: &Path, max_bytes: usize) -> Result<Vec<u8>, PlatformError> {
    let Some(bytes) = read_regular_file_inner(path, max_bytes, false)? else {
        return Err(PlatformError::Io { context: format!("inspect file `{}`", path.display()), message: String::from("file not found") });
    };
    Ok(bytes)
}

/// Read all bytes from `path` if it exists, enforcing regular-file and `max_bytes` checks.
///
/// Returns `Ok(None)` when `path` does not exist. Non-regular files (directories, FIFOs,
/// character/block devices, sockets) and oversized files return explicit errors without blocking
/// or unbounded allocation. On Unix, `open(2)` uses `O_NONBLOCK` prior to `fstat` validation so
/// TOCTOU swaps to a FIFO return an error immediately instead of hanging.
///
/// # Errors
/// Returns [`PlatformError::InvalidPath`] when `path` is empty or non-regular,
/// [`PlatformError::FileTooLarge`] when size exceeds `max_bytes`, or [`PlatformError::Io`] on
/// non-`NotFound` OS failures.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_regular_file_bytes_if_exists(path: &Path, max_bytes: usize) -> Result<Option<Vec<u8>>, PlatformError> {
    read_regular_file_inner(path, max_bytes, true)
}

/// Read valid UTF-8 text from an existing regular file at `path`, bounded by `max_bytes`.
///
/// # Errors
/// Returns the same errors as [`read_regular_file_bytes`], or [`PlatformError::Io`] if the file
/// content is not valid UTF-8.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_regular_file_utf8(path: &Path, max_bytes: usize) -> Result<String, PlatformError> {
    let bytes = read_regular_file_bytes(path, max_bytes)?;
    String::from_utf8(bytes).map_err(|err| PlatformError::Io { context: format!("decode UTF-8 file `{}`", path.display()), message: err.to_string() })
}

/// Target-exact `O_NONBLOCK` flag for safe `OpenOptionsExt::custom_flags` without `unsafe` or
/// extra crate dependencies.
#[cfg(all(not(target_arch = "wasm32"), unix))]
const fn unix_o_nonblock() -> Option<i32> {
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", target_os = "dragonfly"))]
    {
        Some(0x0004)
    }
    #[cfg(all(
        any(target_os = "linux", target_os = "android"),
        any(
            target_arch = "x86",
            target_arch = "x86_64",
            target_arch = "arm",
            target_arch = "aarch64",
            target_arch = "riscv32",
            target_arch = "riscv64",
            target_arch = "powerpc",
            target_arch = "powerpc64",
            target_arch = "s390x",
            target_arch = "loongarch64"
        )
    ))]
    {
        Some(0x0800)
    }
    #[cfg(all(any(target_os = "linux", target_os = "android"), any(target_arch = "mips", target_arch = "mips64")))]
    {
        Some(0x0080)
    }
    #[cfg(all(any(target_os = "linux", target_os = "android"), any(target_arch = "sparc", target_arch = "sparc64")))]
    {
        Some(0x4000)
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly",
        all(
            any(target_os = "linux", target_os = "android"),
            any(
                target_arch = "x86",
                target_arch = "x86_64",
                target_arch = "arm",
                target_arch = "aarch64",
                target_arch = "riscv32",
                target_arch = "riscv64",
                target_arch = "powerpc",
                target_arch = "powerpc64",
                target_arch = "s390x",
                target_arch = "loongarch64",
                target_arch = "mips",
                target_arch = "mips64",
                target_arch = "sparc",
                target_arch = "sparc64"
            )
        )
    )))]
    {
        None
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn open_nonblocking_regular_file(path: &Path, max_bytes: usize, allow_missing: bool) -> Result<Option<(File, Metadata)>, PlatformError> {
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    if let Some(flag) = unix_o_nonblock() {
        opts.custom_flags(flag);
    }
    let file = match opts.open(path) {
        Ok(file) => file,
        Err(err) if allow_missing && err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(io_err(&format!("open file `{}`", path.display()), &err)),
    };
    let fd_meta = file.metadata().map_err(|err| io_err(&format!("inspect open file `{}`", path.display()), &err))?;
    validate_regular_file_meta(path, &fd_meta, max_bytes)?;
    Ok(Some((file, fd_meta)))
}

#[cfg(not(target_arch = "wasm32"))]
fn read_regular_file_inner(path: &Path, max_bytes: usize, allow_missing: bool) -> Result<Option<Vec<u8>>, PlatformError> {
    if path.as_os_str().is_empty() {
        return Err(PlatformError::InvalidPath { reason: String::from("file path must not be empty") });
    }
    let pre_meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) if allow_missing && err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(io_err(&format!("inspect file `{}`", path.display()), &err)),
    };
    validate_regular_file_meta(path, &pre_meta, max_bytes)?;

    let Some((file, fd_meta)) = open_nonblocking_regular_file(path, max_bytes, allow_missing)? else {
        return Ok(None);
    };

    let reserve = usize::try_from(fd_meta.len()).unwrap_or(0).min(max_bytes);
    let mut bytes = Vec::with_capacity(reserve);
    let read_cap = u64::try_from(max_bytes).unwrap_or(u64::MAX).saturating_add(1);
    file.take(read_cap).read_to_end(&mut bytes).map_err(|err| io_err(&format!("read file `{}`", path.display()), &err))?;
    if bytes.len() > max_bytes {
        return Err(PlatformError::FileTooLarge { path: path.display().to_string(), actual_bytes: None, limit_bytes: max_bytes });
    }
    Ok(Some(bytes))
}

#[cfg(not(target_arch = "wasm32"))]
fn validate_regular_file_meta(path: &Path, meta: &Metadata, max_bytes: usize) -> Result<(), PlatformError> {
    if !meta.file_type().is_file() {
        return Err(PlatformError::InvalidPath { reason: format!("path `{}` must be a regular file", path.display()) });
    }
    let max_u64 = u64::try_from(max_bytes).unwrap_or(u64::MAX);
    if meta.len() > max_u64 {
        return Err(PlatformError::FileTooLarge { path: path.display().to_string(), actual_bytes: Some(meta.len()), limit_bytes: max_bytes });
    }
    Ok(())
}

/// Synchronize directory metadata (`fsync`) on `dir` so preceding file renames, creations, or
/// unlinks in `dir` are durable across power loss.
///
/// Platforms or filesystems that reject opening or `fsync`-ing directories (`EINVAL`, `ENOTSUP`,
/// `EACCES`, `EISDIR`, or read-only directory descriptors returning `EBADF` /
/// `ERROR_INVALID_HANDLE`) are treated as non-fatal no-ops, whereas genuine storage failures
/// (`EIO`, `ENOSPC`, etc.) propagate as [`PlatformError::Io`].
///
/// # Errors
/// Returns [`PlatformError::Io`] if opening or syncing `dir` fails with a non-ignorable OS error.
#[cfg(not(target_arch = "wasm32"))]
pub fn sync_dir(dir: &Path) -> Result<(), PlatformError> {
    let handle = match File::open(dir) {
        Ok(file) => file,
        Err(err) if is_ignorable_dir_sync_error(&err) => return Ok(()),
        Err(err) => return Err(io_err(&format!("open directory `{}` for sync", dir.display()), &err)),
    };
    check_dir_sync_result(dir, handle.sync_all())
}

#[cfg(not(target_arch = "wasm32"))]
fn check_dir_sync_result(dir: &Path, result: std::io::Result<()>) -> Result<(), PlatformError> {
    match result {
        Ok(()) => Ok(()),
        Err(err) if is_ignorable_dir_sync_error(&err) => Ok(()),
        Err(err) => Err(io_err(&format!("sync directory `{}`", dir.display()), &err)),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn is_ignorable_dir_sync_error(err: &std::io::Error) -> bool {
    if matches!(err.kind(), ErrorKind::InvalidInput | ErrorKind::Unsupported | ErrorKind::PermissionDenied | ErrorKind::IsADirectory) {
        return true;
    }
    #[cfg(unix)]
    if err.raw_os_error() == Some(9) {
        return true;
    }
    #[cfg(windows)]
    if matches!(err.raw_os_error(), Some(1 | 5 | 6 | 50 | 87)) {
        return true;
    }
    false
}

#[cfg(not(target_arch = "wasm32"))]
fn io_err(context: &str, err: &std::io::Error) -> PlatformError {
    PlatformError::Io { context: context.to_owned(), message: err.to_string() }
}

/// Recursively remove `path` after verifying via `symlink_metadata` that it is a regular,
/// non-symlink directory.
///
/// # Errors
/// Returns [`PlatformError::InvalidPath`] when `path` is empty, a symlink, or not a directory, or
/// [`PlatformError::Io`] if metadata inspection or recursive removal fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn remove_dir_all_if_not_symlink(path: &Path) -> Result<(), PlatformError> {
    if path.as_os_str().is_empty() {
        return Err(PlatformError::InvalidPath { reason: String::from("directory path must not be empty") });
    }
    let meta = std::fs::symlink_metadata(path).map_err(|err| io_err(&format!("inspect directory `{}`", path.display()), &err))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(PlatformError::InvalidPath { reason: format!("path `{}` must be a regular non-symlink directory", path.display()) });
    }
    std::fs::remove_dir_all(path).map_err(|err| io_err(&format!("remove directory `{}`", path.display()), &err))
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use std::io::Error;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{
        check_dir_sync_result, read_regular_file_bytes, read_regular_file_bytes_if_exists, read_regular_file_utf8, resolve_parent_dir, sync_dir,
        write_atomic, write_atomic_with_hook,
    };
    use crate::PlatformError;

    static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

    fn temp_dir(label: &str) -> PathBuf {
        let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("curvebook-platform-fs-{label}-{}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn list_dir_entries(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn write_atomic_happy_path_creates_and_replaces_without_leftover_temps() {
        let dir = temp_dir("happy");
        let target = dir.join("project.curvebook");

        write_atomic(&target, b"first payload").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"first payload");
        assert_eq!(list_dir_entries(&dir), vec![String::from("project.curvebook")]);

        write_atomic(&target, b"second longer payload").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"second longer payload");
        assert_eq!(list_dir_entries(&dir), vec![String::from("project.curvebook")]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_atomic_with_hook_failure_cleans_temp_and_preserves_target() {
        let dir = temp_dir("hook-fail");
        let target = dir.join("missing.txt");

        let err = write_atomic_with_hook(&target, b"partial", |tmp| {
            assert!(tmp.exists());
            Err(Error::other("simulated crash before rename"))
        })
        .unwrap_err();
        assert!(matches!(err, PlatformError::Io { .. }));
        assert!(!target.exists());
        assert!(list_dir_entries(&dir).is_empty(), "temp files leaked: {:?}", list_dir_entries(&dir));

        write_atomic(&target, b"original intact").unwrap();
        let err2 = write_atomic_with_hook(&target, b"replacement", |_| Err(Error::other("abort"))).unwrap_err();
        assert!(matches!(err2, PlatformError::Io { .. }));
        assert_eq!(std::fs::read(&target).unwrap(), b"original intact");
        assert_eq!(list_dir_entries(&dir), vec![String::from("missing.txt")]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_parent_normalizes_to_dot_and_invalid_paths_reject() {
        assert_eq!(resolve_parent_dir(Path::new("relative.txt")), Path::new("."));
        assert!(matches!(write_atomic(Path::new(""), b"x"), Err(PlatformError::InvalidPath { .. })));
        assert!(matches!(write_atomic(Path::new("/"), b"x"), Err(PlatformError::InvalidPath { .. })));
    }

    #[test]
    fn read_regular_file_enforces_bounds_type_and_non_blocking_checks() {
        let dir = temp_dir("bounded-read");
        let file_path = dir.join("small.txt");
        std::fs::write(&file_path, b"hello").unwrap();

        assert_eq!(read_regular_file_bytes(&file_path, 5).unwrap(), b"hello");
        assert_eq!(read_regular_file_utf8(&file_path, 5).unwrap(), "hello");
        assert!(matches!(read_regular_file_bytes(&file_path, 4), Err(PlatformError::FileTooLarge { actual_bytes: Some(5), limit_bytes: 4, .. })));

        let sparse_path = dir.join("sparse.bin");
        let sparse = std::fs::File::create(&sparse_path).unwrap();
        sparse.set_len(64 * 1024 * 1024 + 1).unwrap();
        assert!(matches!(
            read_regular_file_bytes(&sparse_path, 1024),
            Err(PlatformError::FileTooLarge { actual_bytes: Some(67_108_865), limit_bytes: 1024, .. })
        ));

        let missing_path = dir.join("missing.txt");
        assert_eq!(read_regular_file_bytes_if_exists(&missing_path, 128).unwrap(), None);
        assert!(matches!(read_regular_file_bytes(&missing_path, 128), Err(PlatformError::Io { .. })));
        assert!(matches!(read_regular_file_bytes(Path::new(""), 128), Err(PlatformError::InvalidPath { .. })));
        assert!(matches!(read_regular_file_bytes(&dir, 128), Err(PlatformError::InvalidPath { .. })));

        let invalid_utf8 = dir.join("invalid.bin");
        std::fs::write(&invalid_utf8, [0xff, 0xfe, 0x00]).unwrap();
        assert!(matches!(read_regular_file_utf8(&invalid_utf8, 16), Err(PlatformError::Io { .. })));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn read_regular_file_rejects_fifo_symlink_to_fifo_and_char_device_without_blocking() {
        use std::time::{Duration, Instant};

        let dir = temp_dir("unix-special");
        let fifo = dir.join("input.pipe");
        let mkfifo_status = std::process::Command::new("mkfifo").arg(&fifo).status().unwrap();
        assert!(mkfifo_status.success());

        let start = Instant::now();
        assert!(matches!(read_regular_file_bytes(&fifo, 1024), Err(PlatformError::InvalidPath { .. })));
        assert!(matches!(read_regular_file_bytes_if_exists(&fifo, 1024), Err(PlatformError::InvalidPath { .. })));

        // GAP-04-04: directly exercise the O_NONBLOCK open + fstat path on the raw FIFO without
        // pre-open `metadata()`, proving a TOCTOU swap to a FIFO returns `InvalidPath` immediately
        // instead of hanging in blocking `open(2)`.
        assert!(super::unix_o_nonblock().is_some());
        assert!(matches!(super::open_nonblocking_regular_file(&fifo, 1024, false), Err(PlatformError::InvalidPath { .. })));
        assert!(start.elapsed() < Duration::from_secs(2), "FIFO check blocked unexpectedly: {:?}", start.elapsed());

        let fifo_link = dir.join("link_to_fifo.csv");
        std::os::unix::fs::symlink(&fifo, &fifo_link).unwrap();
        assert!(matches!(read_regular_file_bytes(&fifo_link, 1024), Err(PlatformError::InvalidPath { .. })));
        assert!(matches!(super::open_nonblocking_regular_file(&fifo_link, 1024, false), Err(PlatformError::InvalidPath { .. })));

        let dev_zero = Path::new("/dev/zero");
        if dev_zero.exists() {
            assert!(matches!(read_regular_file_bytes(dev_zero, 1024), Err(PlatformError::InvalidPath { .. })));
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_preserves_restrictive_permissions_and_follows_symlinks() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let dir = temp_dir("perms-symlink");
        let secret = dir.join("secret.curvebook");
        std::fs::write(&secret, b"v1").unwrap();
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();

        write_atomic(&secret, b"v2").unwrap();
        let mode_after = std::fs::metadata(&secret).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode_after, 0o600, "restrictive mode must be preserved across atomic overwrite");
        assert_eq!(std::fs::read(&secret).unwrap(), b"v2");

        // Update through a multi-hop symlink chain: link_outer -> link_inner -> target_dir/prefs.json
        let target_dir = dir.join("real");
        std::fs::create_dir_all(&target_dir).unwrap();
        let real_prefs = target_dir.join("prefs.json");
        std::fs::write(&real_prefs, b"{\"theme\":\"light\"}").unwrap();
        std::fs::set_permissions(&real_prefs, std::fs::Permissions::from_mode(0o640)).unwrap();

        let link_inner = dir.join("inner_prefs.json");
        symlink("real/prefs.json", &link_inner).unwrap();
        let link_outer = dir.join("outer_prefs.json");
        symlink(&link_inner, &link_outer).unwrap();

        write_atomic(&link_outer, b"{\"theme\":\"dark\"}").unwrap();
        assert!(std::fs::symlink_metadata(&link_outer).unwrap().file_type().is_symlink(), "outer symlink should remain a symlink");
        assert!(std::fs::symlink_metadata(&link_inner).unwrap().file_type().is_symlink(), "inner symlink should remain a symlink");
        assert_eq!(std::fs::read(&real_prefs).unwrap(), b"{\"theme\":\"dark\"}");
        assert_eq!(std::fs::metadata(&real_prefs).unwrap().permissions().mode() & 0o777, 0o640);
        assert!(list_dir_entries(&target_dir).iter().all(|name| !name.starts_with('.')));

        // Dangling symlink populates target without clobbering the symlink itself.
        let dangling_target = target_dir.join("future.json");
        let dangling_link = dir.join("dangling.json");
        symlink(&dangling_target, &dangling_link).unwrap();
        write_atomic(&dangling_link, b"created-via-symlink").unwrap();
        assert!(std::fs::symlink_metadata(&dangling_link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read(&dangling_target).unwrap(), b"created-via-symlink");
        assert_eq!(std::fs::read(&dangling_link).unwrap(), b"created-via-symlink");

        // Writing over an existing directory is rejected explicitly.
        assert!(matches!(write_atomic(&target_dir, b"nope"), Err(PlatformError::InvalidPath { .. })));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_dir_ignores_unsupported_dir_fsync_but_propagates_storage_failures() {
        use std::io::ErrorKind;

        let dir = temp_dir("sync-dir");
        sync_dir(&dir).expect("sync_dir on valid directory");

        for kind in [ErrorKind::InvalidInput, ErrorKind::Unsupported, ErrorKind::PermissionDenied, ErrorKind::IsADirectory] {
            check_dir_sync_result(&dir, Err(Error::new(kind, "dir sync rejected by filesystem"))).expect("non-fatal dir sync rejection");
        }
        #[cfg(unix)]
        check_dir_sync_result(&dir, Err(Error::from_raw_os_error(9))).expect("EBADF on read-only dir fd is ignored");

        let storage_full = check_dir_sync_result(&dir, Err(Error::new(ErrorKind::StorageFull, "no space left"))).unwrap_err();
        assert!(matches!(storage_full, PlatformError::Io { .. }));

        #[cfg(unix)]
        {
            let eio = check_dir_sync_result(&dir, Err(Error::from_raw_os_error(5))).unwrap_err();
            assert!(matches!(eio, PlatformError::Io { .. }));
        }

        let missing = sync_dir(&dir.join("does-not-exist")).unwrap_err();
        assert!(matches!(missing, PlatformError::Io { .. }));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_staged_sibling_cleans_up_on_drop_and_supports_explicit_handoff() {
        use super::write_staged_sibling;

        let dir = temp_dir("staged-sibling");
        let target = dir.join("weird name!?.json");

        // Dropping without handoff removes the staged file.
        let dropped_path = {
            let staged = write_staged_sibling(&dir, &target, "stage", b"ephemeral", None).unwrap();
            let p = staged.path().to_path_buf();
            assert!(p.is_file());
            assert_eq!(std::fs::read(&p).unwrap(), b"ephemeral");
            let fname = p.file_name().unwrap().to_string_lossy();
            assert!(fname.starts_with(".weirdname.json.stage."), "unexpected staged name: {fname}");
            p
        };
        assert!(!dropped_path.exists(), "dropped StagedSibling must remove file");

        // Explicit handoff via `into_path` preserves the file for caller-managed commit/cleanup.
        let staged = write_staged_sibling(&dir, &target, "stage", b"handed-off", None).unwrap();
        let handed_off = staged.into_path();
        assert!(handed_off.is_file());
        assert_eq!(std::fs::read(&handed_off).unwrap(), b"handed-off");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_dir_all_if_not_symlink_removes_regular_dirs_and_rejects_symlinks_and_files() {
        use super::remove_dir_all_if_not_symlink;

        let dir = temp_dir("remove-dir");
        let regular_sub = dir.join("nested/child");
        std::fs::create_dir_all(&regular_sub).unwrap();
        std::fs::write(regular_sub.join("data.txt"), b"ok").unwrap();

        let regular_file = dir.join("file.txt");
        std::fs::write(&regular_file, b"not a dir").unwrap();
        assert!(matches!(remove_dir_all_if_not_symlink(&regular_file), Err(PlatformError::InvalidPath { .. })));

        #[cfg(unix)]
        {
            let target_dir = dir.join("kept-target");
            std::fs::create_dir_all(&target_dir).unwrap();
            std::fs::write(target_dir.join("keep.txt"), b"stay").unwrap();
            let symlink_dir = dir.join("link-dir");
            std::os::unix::fs::symlink(&target_dir, &symlink_dir).unwrap();
            assert!(matches!(remove_dir_all_if_not_symlink(&symlink_dir), Err(PlatformError::InvalidPath { .. })));
            assert!(target_dir.join("keep.txt").is_file(), "symlink target must not be deleted");
        }

        remove_dir_all_if_not_symlink(&dir.join("nested")).unwrap();
        assert!(!dir.join("nested").exists());
        remove_dir_all_if_not_symlink(&dir).unwrap();
        assert!(!dir.exists());
    }

    #[test]
    fn gap_04_07_sweep_stale_temp_files_cleans_dead_orphans_without_touching_live_writers() {
        use std::time::Duration;

        use super::{parse_sibling_temp_name, sweep_stale_temp_files, sweep_stale_temp_files_inner};

        assert_eq!(parse_sibling_temp_name(".my.proj.curvebook.tmp.4242.7"), Some(("my.proj.curvebook", "tmp", 4242, 7)));
        assert_eq!(parse_sibling_temp_name(".gitignore"), None);
        assert_eq!(parse_sibling_temp_name("not_hidden.tmp.4242.7"), None);
        assert_eq!(parse_sibling_temp_name(".file.tmp.0.1"), None);

        let dir = temp_dir("stale-sweep");
        let cur_pid = std::process::id();

        // Obtain a guaranteed-dead PID by spawning and reaping a trivial child.
        #[cfg(unix)]
        let dead_pid = {
            let mut child = std::process::Command::new("sh")
                .args(["-c", "exit 0"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            let pid = child.id();
            let _ = child.wait();
            pid
        };
        #[cfg(not(unix))]
        let dead_pid = cur_pid.wrapping_add(50_000).max(2);

        let stale_orphan = dir.join(format!(".project.curvebook.tmp.{dead_pid}.1"));
        let other_stem_orphan = dir.join(format!(".other.curvebook.tmp.{dead_pid}.2"));
        let live_self = dir.join(format!(".project.curvebook.tmp.{cur_pid}.99"));
        let unrelated_hidden = dir.join(".gitignore");
        let non_tmp_tag = dir.join(format!(".project.curvebook.stage.{dead_pid}.3"));
        std::fs::write(&stale_orphan, b"orphaned").unwrap();
        std::fs::write(&other_stem_orphan, b"other-orphaned").unwrap();
        std::fs::write(&live_self, b"active-writer").unwrap();
        std::fs::write(&unrelated_hidden, b"*.tmp\n").unwrap();
        std::fs::write(&non_tmp_tag, b"staged").unwrap();

        #[cfg(unix)]
        let symlink_orphan = {
            let real_keep = dir.join("keep.txt");
            std::fs::write(&real_keep, b"do not delete").unwrap();
            let link = dir.join(format!(".project.curvebook.tmp.{dead_pid}.4"));
            std::os::unix::fs::symlink(&real_keep, &link).unwrap();
            link
        };

        // High max_age must preserve newly created files even for dead PIDs.
        assert_eq!(sweep_stale_temp_files(&dir, Duration::from_secs(3600)).unwrap(), 0);
        assert!(stale_orphan.exists());

        // Stem-filtered sweep removes only the matching stem's stale dead-PID `.tmp.` file.
        assert_eq!(sweep_stale_temp_files_inner(&dir, Some("project.curvebook"), Duration::ZERO, 1024).unwrap(), 1);
        assert!(!stale_orphan.exists(), "matching dead-PID orphan must be removed");
        assert!(other_stem_orphan.exists(), "non-matching stem must be preserved by stem-filtered sweep");
        assert!(live_self.exists(), "active current-PID temp file must never be removed");
        assert!(unrelated_hidden.exists());
        assert!(non_tmp_tag.exists());
        #[cfg(unix)]
        assert!(std::fs::symlink_metadata(&symlink_orphan).unwrap().file_type().is_symlink());

        // Full directory sweep cleans the remaining dead-PID `.tmp.` orphan and leaves everything else.
        assert_eq!(sweep_stale_temp_files(&dir, Duration::ZERO).unwrap(), 1);
        assert!(!other_stem_orphan.exists());
        assert!(live_self.exists());
        assert!(unrelated_hidden.exists());
        assert!(non_tmp_tag.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
