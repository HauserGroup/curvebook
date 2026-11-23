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
use crate::PlatformError;

#[cfg(not(target_arch = "wasm32"))]
static TMP_FILE_SEQ: AtomicU64 = AtomicU64::new(1);

#[cfg(not(target_arch = "wasm32"))]
const MAX_TEMP_CREATE_ATTEMPTS: usize = 32;

#[cfg(not(target_arch = "wasm32"))]
const MAX_SYMLINK_HOPS: usize = 40;

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

/// Read all bytes from an existing regular file at `path`, enforcing a hard `max_bytes` cap.
///
/// Inspects file metadata before opening (rejecting directories, FIFOs, sockets, and device
/// nodes without blocking, and oversized files before allocating), re-validates the open
/// descriptor's metadata against TOCTOU swaps, and reads through [`Read::take`] capped at
/// `max_bytes + 1`.
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
/// or unbounded allocation.
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

    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if allow_missing && err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(io_err(&format!("open file `{}`", path.display()), &err)),
    };
    let fd_meta = file.metadata().map_err(|err| io_err(&format!("inspect open file `{}`", path.display()), &err))?;
    validate_regular_file_meta(path, &fd_meta, max_bytes)?;

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
        let dir = temp_dir("unix-special");
        let fifo = dir.join("input.pipe");
        let mkfifo_status = std::process::Command::new("mkfifo").arg(&fifo).status().unwrap();
        assert!(mkfifo_status.success());

        assert!(matches!(read_regular_file_bytes(&fifo, 1024), Err(PlatformError::InvalidPath { .. })));
        assert!(matches!(read_regular_file_bytes_if_exists(&fifo, 1024), Err(PlatformError::InvalidPath { .. })));

        let fifo_link = dir.join("link_to_fifo.csv");
        std::os::unix::fs::symlink(&fifo, &fifo_link).unwrap();
        assert!(matches!(read_regular_file_bytes(&fifo_link, 1024), Err(PlatformError::InvalidPath { .. })));

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
}
