//! `--workdir` path containment and recursive script/manifest path sanitization.
//!
//! Delegates path containment and parameter sanitization to [`curvebook_platform::path_guard`]
//! while preserving CLI-specific [`CliError`] exit-code semantics.

use std::path::{Path, PathBuf};

use curvebook_platform::PlatformError;
use curvebook_platform::path_guard::{PathResolver as PlatformPathResolver, WorkdirGuard as PlatformWorkdirGuard};
use serde_json::Value;

use crate::error::CliError;

/// Canonicalized `--workdir` guard rejecting parent traversal (`..`), outside paths, and symlink escapes.
#[derive(Debug, Clone)]
pub struct WorkdirGuard(PlatformWorkdirGuard);

impl WorkdirGuard {
    /// Validate and canonicalize `--workdir`.
    pub fn new(workdir: &Path) -> Result<Self, CliError> {
        PlatformWorkdirGuard::new(workdir).map(Self).map_err(|err| match err {
            PlatformError::InvalidPath { reason } => CliError::Usage(format!("--{reason}")),
            PlatformError::Io { context, message } => CliError::Usage(format!("{context}: {message}")),
            other => CliError::from(other),
        })
    }

    /// Resolve and verify that `raw_path` stays strictly inside `--workdir`.
    pub fn resolve(&self, raw_path: &str) -> Result<PathBuf, CliError> {
        self.0.resolve(raw_path).map_err(|err| map_guard_error(err, true))
    }
}

/// Strategy for resolving file paths in `run`, `batch`, and direct command invocations.
#[derive(Debug, Clone)]
pub enum PathResolver {
    /// Strict containment inside `--workdir` (rejects escapes with exit code `2`).
    Sandboxed(WorkdirGuard),
    /// Unsandboxed resolution relative to `base_dir` (when `--workdir` is omitted).
    Unconstrained { base_dir: PathBuf },
}

impl PathResolver {
    fn as_platform(&self) -> PlatformPathResolver {
        match self {
            Self::Sandboxed(guard) => PlatformPathResolver::Sandboxed(guard.0.clone()),
            Self::Unconstrained { base_dir } => PlatformPathResolver::Unconstrained { base_dir: base_dir.clone() },
        }
    }

    /// Resolve a filesystem path string according to the active containment policy.
    pub fn resolve(&self, raw_path: &str) -> Result<PathBuf, CliError> {
        match self {
            Self::Sandboxed(guard) => guard.resolve(raw_path),
            Self::Unconstrained { .. } => self.as_platform().resolve(raw_path).map_err(|err| map_guard_error(err, false)),
        }
    }

    /// Recursively inspect and rewrite any filesystem-path fields in a JSON command parameter tree.
    pub fn sanitize_params(&self, params: &mut Value) -> Result<(), CliError> {
        let sandboxed = matches!(self, Self::Sandboxed(_));
        self.as_platform().sanitize_params(params).map_err(|err| match err {
            PlatformError::InvalidPath { reason } if reason.starts_with("parameter `") => CliError::Schema(reason),
            other => map_guard_error(other, sandboxed),
        })
    }
}

fn map_guard_error(err: PlatformError, sandboxed: bool) -> CliError {
    match err {
        PlatformError::PathEscape { .. } => CliError::PathEscape(err.to_string()),
        PlatformError::InvalidPath { reason } => {
            if sandboxed {
                CliError::PathEscape(reason)
            } else {
                CliError::Schema(reason)
            }
        }
        other => CliError::from(other),
    }
}

#[cfg(test)]
mod tests {
    use curvebook_platform::path_guard::PATH_PARAM_KEYS;
    use serde_json::json;

    use super::{PathResolver, WorkdirGuard};
    use crate::error::CliError;

    #[test]
    fn sandbox_rewrites_path_keys_but_keeps_inline_csv_text() {
        let dir = std::env::temp_dir().join(format!("curvebook_sandbox_csv_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let guard = WorkdirGuard::new(&dir).unwrap();
        let resolver = PathResolver::Sandboxed(guard);

        let inline = "x,y\n1,2\n../3,4\n";
        let mut params = json!({ "csv": inline, "path": "out.csv" });
        resolver.sanitize_params(&mut params).unwrap();

        assert_eq!(params["csv"], json!(inline), "inline CSV text must not be rewritten as a path");
        let canonical = std::fs::canonicalize(&dir).unwrap();
        assert_eq!(params["path"], json!(canonical.join("out.csv").to_string_lossy()));
        assert!(!PATH_PARAM_KEYS.contains(&"csv"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sandbox_rejects_empty_and_non_string_paths_consistently() {
        let dir = std::env::temp_dir().join(format!("curvebook_sandbox_err_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let guard = WorkdirGuard::new(&dir).unwrap();
        assert!(matches!(guard.resolve(""), Err(CliError::PathEscape(_))));

        let resolver = PathResolver::Sandboxed(guard);
        let mut bad_param = json!({ "path": 42 });
        assert!(matches!(resolver.sanitize_params(&mut bad_param), Err(CliError::Schema(_))));

        let unconstrained = PathResolver::Unconstrained { base_dir: dir.clone() };
        assert!(matches!(unconstrained.resolve("  "), Err(CliError::Schema(_))));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
