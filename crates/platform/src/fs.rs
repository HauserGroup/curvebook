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

#[cfg(not(target_arch = "wasm32"))]
struct TempFileGuard {
    path: PathBuf,
    armed: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl TempFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
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
    let (parent, file_stem) = validate_target_path(&resolved_target)?;
    let (mut file, tmp_path) = create_unique_sibling_temp(parent, &file_stem, existing_perms.is_some())?;
    let mut guard = TempFileGuard::new(tmp_path.clone());

    file.write_all(bytes).map_err(|err| io_err(&format!("write temp file `{}`", tmp_path.display()), &err))?;
    if let Some(perms) = existing_perms {
        file.set_permissions(perms).map_err(|err| io_err(&format!("set permissions on temp file `{}`", tmp_path.display()), &err))?;
    }
    file.sync_all().map_err(|err| io_err(&format!("sync temp file `{}`", tmp_path.display()), &err))?;
    drop(file);

    before_rename(&tmp_path).map_err(|err| io_err(&format!("pre-rename hook for `{}`", resolved_target.display()), &err))?;
    std::fs::rename(&tmp_path, &resolved_target)
        .map_err(|err| io_err(&format!("rename `{}` to `{}`", tmp_path.display(), resolved_target.display()), &err))?;
    guard.disarm();

    sync_parent_dir_best_effort(parent);
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
        if lmeta.is_dir() {
            return Err(PlatformError::InvalidPath { reason: format!("target path `{}` is a directory", path.display()) });
        }
        return Ok((path.to_path_buf(), Some(lmeta.permissions())));
    }
    resolve_symlink_target(path)
}

#[cfg(not(target_arch = "wasm32"))]
fn resolve_symlink_target(path: &Path) -> Result<(PathBuf, Option<Permissions>), PlatformError> {
    match std::fs::metadata(path) {
        Ok(meta) => {
            if meta.is_dir() {
                return Err(PlatformError::InvalidPath { reason: format!("target path `{}` resolves to a directory", path.display()) });
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
                    Ok(meta) if meta.is_dir() => {
                        return Err(PlatformError::InvalidPath { reason: format!("target path `{}` resolves to a directory", current.display()) });
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
fn validate_target_path(path: &Path) -> Result<(&Path, String), PlatformError> {
    if path.as_os_str().is_empty() {
        return Err(PlatformError::InvalidPath { reason: String::from("target path must not be empty") });
    }
    let Some(file_name) = path.file_name() else {
        return Err(PlatformError::InvalidPath { reason: format!("target path `{}` has no file name", path.display()) });
    };
    let parent = resolve_parent_dir(path);
    let sanitized: String =
        file_name.to_string_lossy().chars().filter(|ch| ch.is_ascii_alphanumeric() || *ch == '.' || *ch == '-' || *ch == '_').take(48).collect();
    let stem = if sanitized.is_empty() { String::from("file") } else { sanitized };
    Ok((parent, stem))
}

#[cfg(not(target_arch = "wasm32"))]
fn resolve_parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn create_unique_sibling_temp(parent: &Path, stem: &str, restrict_mode: bool) -> Result<(File, PathBuf), PlatformError> {
    let pid = std::process::id();
    for _ in 0..MAX_TEMP_CREATE_ATTEMPTS {
        let seq = TMP_FILE_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp_path = parent.join(format!(".{stem}.tmp.{pid}.{seq}"));
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

#[cfg(not(target_arch = "wasm32"))]
fn sync_parent_dir_best_effort(parent: &Path) {
    if let Ok(dir) = File::open(parent) {
        let _ = dir.sync_all();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn io_err(context: &str, err: &std::io::Error) -> PlatformError {
    PlatformError::Io { context: context.to_owned(), message: err.to_string() }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use std::io::Error;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{
        read_regular_file_bytes, read_regular_file_bytes_if_exists, read_regular_file_utf8, resolve_parent_dir, write_atomic, write_atomic_with_hook,
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
}
