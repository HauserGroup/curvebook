//! Shared test harness primitives (`TempDirGuard`, `assert_close`, `assert_rel_close`)
//! for CurveBook integration tests.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

/// RAII temporary directory removed on drop.
///
/// Each directory name embeds the process ID and an atomic sequence counter so concurrent
/// test threads and processes never collide.
#[derive(Debug)]
pub struct TempDirGuard {
    path: PathBuf,
    #[cfg(unix)]
    restore_writable: Vec<PathBuf>,
}

impl TempDirGuard {
    /// Create a unique temporary directory under `std::env::temp_dir()` tagged with `label`.
    ///
    /// # Errors
    /// Returns an I/O error if creating the directory fails.
    pub fn new(label: &str) -> std::io::Result<Self> {
        let seq = TEMP_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let clean: String = label.chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '_' }).collect();
        let name =
            if clean.is_empty() { format!("cb-test-{}-{seq}", std::process::id()) } else { format!("cb-{clean}-{}-{seq}", std::process::id()) };
        let path = std::env::temp_dir().join(name);
        if path.exists() {
            let _ = std::fs::remove_dir_all(&path);
        }
        std::fs::create_dir_all(&path)?;
        Ok(Self {
            path,
            #[cfg(unix)]
            restore_writable: Vec::new(),
        })
    }

    /// Return the path to the managed temporary directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return `self.path().join(name)`.
    #[must_use]
    pub fn child(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }

    /// Record a subdirectory whose permissions should be restored to `0o755` before cleanup on drop.
    #[cfg(unix)]
    pub fn track_chmod_restore(&mut self, dir: PathBuf) {
        self.restore_writable.push(dir);
    }
}

impl AsRef<Path> for TempDirGuard {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            for dir in &self.restore_writable {
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
            }
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Assert that finite `actual` and `expected` agree within absolute tolerance `tol`.
#[track_caller]
#[allow(clippy::panic)]
pub fn assert_close(actual: f64, expected: f64, tol: f64, label: &str) {
    let diff = (actual - expected).abs();
    if !actual.is_finite() || !expected.is_finite() || diff > tol {
        panic!("{label}: expected {expected:.15e} +/- {tol:.2e}, got {actual:.15e} (diff={diff:.3e})");
    }
}

/// Assert that finite `actual` and `expected` agree within relative tolerance `rel_tol * |expected|`.
#[track_caller]
#[allow(clippy::panic)]
pub fn assert_rel_close(actual: f64, expected: f64, rel_tol: f64, label: &str) {
    let diff = (actual - expected).abs();
    let limit = rel_tol * expected.abs();
    if !actual.is_finite() || !expected.is_finite() || diff > limit {
        panic!("{label}: expected {expected:.15e} (rel_tol={rel_tol:.2e}), got {actual:.15e} (diff={diff:.3e})");
    }
}

#[cfg(test)]
mod tests {
    use super::{TempDirGuard, assert_close, assert_rel_close};

    #[test]
    fn temp_dir_guard_creates_unique_dirs_and_cleans_up_on_drop() {
        let g1 = TempDirGuard::new("unit/alpha").expect("create g1");
        let g2 = TempDirGuard::new("unit/alpha").expect("create g2");
        assert_ne!(g1.path(), g2.path());
        assert!(g1.path().is_dir());
        let child = g1.child("sub.txt");
        std::fs::write(&child, b"ok").expect("write child");
        assert!(child.is_file());

        let p1 = g1.path().to_path_buf();
        drop(g1);
        assert!(!p1.exists(), "TempDirGuard must remove directory on drop");
    }

    #[test]
    fn assert_close_and_assert_rel_close_accept_valid_inputs() {
        assert_close(1.000_000_1, 1.0, 1e-6, "abs ok");
        assert_close(0.0, -0.0, 0.0, "zero ok");
        assert_rel_close(100.005, 100.0, 1e-4, "rel ok");
        assert_rel_close(0.0, 0.0, 1e-12, "rel zero ok");
    }

    #[test]
    #[should_panic(expected = "abs fail")]
    fn assert_close_rejects_exceeding_tol() {
        assert_close(1.01, 1.0, 1e-3, "abs fail");
    }

    #[test]
    #[should_panic(expected = "nan fail")]
    fn assert_close_rejects_nan() {
        assert_close(f64::NAN, 1.0, 1.0, "nan fail");
    }

    #[test]
    #[should_panic(expected = "rel fail")]
    fn assert_rel_close_rejects_exceeding_rel_tol() {
        assert_rel_close(101.0, 100.0, 1e-3, "rel fail");
    }
}
