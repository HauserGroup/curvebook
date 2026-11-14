//! Crash-safe atomic file writes via unique sibling temp file, `sync_all`, and `rename`.

#[cfg(not(target_arch = "wasm32"))]
use std::fs::{File, OpenOptions};
#[cfg(not(target_arch = "wasm32"))]
use std::io::{ErrorKind, Write};
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
    let (parent, file_stem) = validate_target_path(path)?;
    let (mut file, tmp_path) = create_unique_sibling_temp(parent, &file_stem)?;
    let mut guard = TempFileGuard::new(tmp_path.clone());

    file.write_all(bytes).map_err(|err| io_err(&format!("write temp file `{}`", tmp_path.display()), &err))?;
    file.sync_all().map_err(|err| io_err(&format!("sync temp file `{}`", tmp_path.display()), &err))?;
    drop(file);

    before_rename(&tmp_path).map_err(|err| io_err(&format!("pre-rename hook for `{}`", path.display()), &err))?;
    std::fs::rename(&tmp_path, path).map_err(|err| io_err(&format!("rename `{}` to `{}`", tmp_path.display(), path.display()), &err))?;
    guard.disarm();

    sync_parent_dir_best_effort(parent);
    Ok(())
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
fn create_unique_sibling_temp(parent: &Path, stem: &str) -> Result<(File, PathBuf), PlatformError> {
    let pid = std::process::id();
    for _ in 0..MAX_TEMP_CREATE_ATTEMPTS {
        let seq = TMP_FILE_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp_path = parent.join(format!(".{stem}.tmp.{pid}.{seq}"));
        match OpenOptions::new().write(true).create_new(true).open(&tmp_path) {
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

    use super::{resolve_parent_dir, write_atomic, write_atomic_with_hook};
    use crate::PlatformError;

    static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

    fn temp_dir(label: &str) -> PathBuf {
        let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("lumencraft-platform-fs-{label}-{}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn list_dir_entries(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn write_atomic_happy_path_creates_and_replaces_without_leftover_temps() {
        let dir = temp_dir("happy");
        let target = dir.join("project.lumencraft");

        write_atomic(&target, b"first payload").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"first payload");
        assert_eq!(list_dir_entries(&dir), vec![String::from("project.lumencraft")]);

        write_atomic(&target, b"second longer payload").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"second longer payload");
        assert_eq!(list_dir_entries(&dir), vec![String::from("project.lumencraft")]);

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
}
