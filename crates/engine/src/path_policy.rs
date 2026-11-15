//! Shared engine-level external filesystem path policy and safe file I/O helpers.
//!
//! Enforces `--workdir` containment, rejects symlinks/traversal outside allowed roots, refuses
//! non-regular files (directories, FIFOs, sockets, device nodes) via non-blocking metadata checks
//! before reading, and guards stored `ExternalDelimitedFile` links during recalculation.

#[cfg(not(target_arch = "wasm32"))]
use std::fs;
use std::path::{Path, PathBuf};

use lumencraft_doc::{ErrorCode, LinkSource};
#[cfg(not(target_arch = "wasm32"))]
use lumencraft_platform::{PlatformError, WorkdirGuard, ensure_regular_file};
use serde_json::{Value, json};

use crate::EngineError;

const ENGINE_PATH_PARAM_KEYS: &[&str] =
    &["path", "file_path", "filePath", "source_path", "sourcePath", "template_path", "templatePath", "ttf_font", "ttfFont", "font_path", "fontPath"];

/// Policy governing external filesystem reads, writes, and stored `ExternalDelimitedFile` links
/// on an engine [`crate::Session`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ExternalPathPolicy {
    /// No directory confinement; non-regular file checks still protect reads.
    #[default]
    Unconstrained,
    /// Confine all filesystem reads, writes, and stored external links strictly inside `WorkdirGuard`.
    #[cfg(not(target_arch = "wasm32"))]
    Workdir(WorkdirGuard),
    /// Reject every external filesystem access (reads, writes, and stored external link refreshes).
    RejectExternal,
}

impl ExternalPathPolicy {
    /// Policy imposing no directory confinement.
    #[must_use]
    pub const fn unconstrained() -> Self {
        Self::Unconstrained
    }

    /// Policy denying all external filesystem reads, writes, and external link refreshes.
    #[must_use]
    pub const fn reject_external() -> Self {
        Self::RejectExternal
    }

    /// Construct a policy confining all engine filesystem operations inside `workdir`.
    ///
    /// # Errors
    /// Returns [`EngineError`] with [`ErrorCode::BadInput`] if `workdir` is not an accessible
    /// directory, or [`ErrorCode::IoError`] if canonicalization fails.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn workdir(workdir: &Path) -> Result<Self, EngineError> {
        Ok(Self::Workdir(WorkdirGuard::new(workdir)?))
    }

    /// Construct a policy from an already validated [`WorkdirGuard`].
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub const fn from_guard(guard: WorkdirGuard) -> Self {
        Self::Workdir(guard)
    }

    /// Active [`WorkdirGuard`], when this policy confines paths to a workspace directory.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub const fn workdir_guard(&self) -> Option<&WorkdirGuard> {
        match self {
            Self::Workdir(guard) => Some(guard),
            Self::Unconstrained | Self::RejectExternal => None,
        }
    }

    /// Return `true` when path confinement or external-access denial is enabled.
    #[must_use]
    pub const fn is_constrained(&self) -> bool {
        !matches!(self, Self::Unconstrained)
    }

    /// Resolve and sanitize filesystem path parameters in command JSON before dispatch.
    ///
    /// Covers command path keys (`path`, `source_path`, `template_path`, aliases) and nested
    /// `link_spec.source.ExternalDelimitedFile.path` entries without disturbing non-path fields
    /// such as `table.importPlateMap`'s inline `template` payload.
    ///
    /// # Errors
    /// Returns [`EngineError`] with [`ErrorCode::PathEscape`] if any path escapes confinement or
    /// external access is disabled, or [`ErrorCode::BadInput`] for invalid path values.
    pub(crate) fn enforce_command_path_params(&self, params: &mut Value) -> Result<(), EngineError> {
        if !self.is_constrained() {
            return Ok(());
        }
        sanitize_param_value(self, params)
    }

    /// Resolve and normalize a stored external link path string under this policy.
    ///
    /// # Errors
    /// Returns [`EngineError`] if the link path escapes confinement, is invalid, or external
    /// filesystem access is disabled.
    pub(crate) fn normalize_stored_link_source(&self, source: &mut LinkSource) -> Result<(), EngineError> {
        if let LinkSource::ExternalDelimitedFile { path, .. } = source {
            validate_raw_path(path, "external link")?;
            match self {
                Self::Unconstrained => {}
                Self::RejectExternal => return Err(reject_external_err(path)),
                #[cfg(not(target_arch = "wasm32"))]
                Self::Workdir(guard) => {
                    let resolved = guard.resolve(path)?;
                    *path = resolved.to_string_lossy().into_owned();
                }
            }
        }
        Ok(())
    }

    /// Read a UTF-8 text file subject to this policy, regular-file validation, and `max_bytes` cap.
    ///
    /// # Errors
    /// Returns [`EngineError`] with [`ErrorCode::PathEscape`] on policy/escape violation,
    /// [`ErrorCode::BadInput`] for empty paths or non-regular files (directories, FIFOs, device
    /// nodes), [`ErrorCode::ResourceLimit`] if size exceeds `max_bytes`, [`ErrorCode::IoError`] if
    /// reading fails, or [`ErrorCode::UnsupportedFormat`] on `wasm32`.
    pub(crate) fn read_utf8_file(&self, raw_path: &str, label: &str, max_bytes: usize) -> Result<String, EngineError> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = max_bytes;
            if matches!(self, Self::RejectExternal) {
                return Err(reject_external_err(raw_path));
            }
            Err(EngineError::with_code(
                ErrorCode::UnsupportedFormat,
                format!("reading {label} `{raw_path}` from filesystem is unavailable on wasm32"),
            )
            .with_remediation("Supply inline text content or run on a native target.")
            .with_details(json!({ "path": raw_path })))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let resolved = self.resolve_target_path(raw_path, label)?;
            lumencraft_platform::fs::read_regular_file_utf8(&resolved, max_bytes).map_err(|err| map_read_platform_err(err, raw_path, label))
        }
    }

    /// Read raw bytes from a regular file subject to this policy, regular-file validation, and `max_bytes` cap.
    ///
    /// # Errors
    /// Returns [`EngineError`] with [`ErrorCode::PathEscape`] on policy/escape violation,
    /// [`ErrorCode::BadInput`] for empty paths or non-regular files, [`ErrorCode::ResourceLimit`]
    /// if size exceeds `max_bytes`, [`ErrorCode::IoError`] if reading fails, or
    /// [`ErrorCode::UnsupportedFormat`] on `wasm32`.
    pub(crate) fn read_binary_file(&self, raw_path: &str, label: &str, max_bytes: usize) -> Result<Vec<u8>, EngineError> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = max_bytes;
            if matches!(self, Self::RejectExternal) {
                return Err(reject_external_err(raw_path));
            }
            Err(EngineError::with_code(
                ErrorCode::UnsupportedFormat,
                format!("reading {label} `{raw_path}` from filesystem is unavailable on wasm32"),
            )
            .with_remediation("Supply `bytes_base64` or run on a native target.")
            .with_details(json!({ "path": raw_path })))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let resolved = self.resolve_target_path(raw_path, label)?;
            lumencraft_platform::fs::read_regular_file_bytes(&resolved, max_bytes).map_err(|err| map_read_platform_err(err, raw_path, label))
        }
    }

    /// Atomically write `bytes` to `raw_path`, creating parent directories when needed and enforcing
    /// this policy before touching the filesystem.
    ///
    /// # Errors
    /// Returns [`EngineError`] with [`ErrorCode::PathEscape`] on policy/escape violation,
    /// [`ErrorCode::BadInput`] if `raw_path` is empty or invalid, [`ErrorCode::IoError`] on
    /// filesystem write failure, or [`ErrorCode::UnsupportedFormat`] on `wasm32`.
    pub(crate) fn write_atomic_bytes(&self, raw_path: &str, bytes: &[u8]) -> Result<(), EngineError> {
        validate_raw_path(raw_path, "output path")?;
        #[cfg(target_arch = "wasm32")]
        {
            let _ = bytes;
            if matches!(self, Self::RejectExternal) {
                return Err(reject_external_err(raw_path));
            }
            Err(EngineError::with_code(ErrorCode::UnsupportedFormat, format!("writing `{raw_path}` to local filesystem is unsupported on wasm32"))
                .with_details(json!({ "path": raw_path })))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let resolved = self.resolve_target_path(raw_path, "output path")?;
            write_resolved_atomic(&resolved, raw_path, bytes)
        }
    }

    /// Resolve a `project.open` target path and determine whether it targets an unpacked directory.
    ///
    /// Rejects escapes, broken symlinks, and non-regular archive files before opening.
    pub(crate) fn resolve_project_open_target(&self, raw_path: &str, explicit_unpacked: bool) -> Result<(PathBuf, bool), EngineError> {
        let resolved = self.resolve_target_path(raw_path, "project")?;
        #[cfg(target_arch = "wasm32")]
        {
            let use_unpacked = explicit_unpacked || has_unpacked_dir_suffix(&resolved);
            Ok((resolved, use_unpacked))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let use_unpacked =
                explicit_unpacked || has_unpacked_dir_suffix(&resolved) || lumencraft_format::is_unpacked_project_dir(&resolved) || resolved.is_dir();
            if use_unpacked {
                ensure_unpacked_dir_if_present(&resolved, raw_path)?;
            } else {
                ensure_regular_file(&resolved).map_err(|err| map_read_platform_err(err, raw_path, "project file"))?;
            }
            Ok((resolved, use_unpacked))
        }
    }

    /// Resolve a `project.save` target path and determine whether it targets an unpacked directory.
    pub(crate) fn resolve_project_save_target(&self, raw_path: &str, explicit_unpacked: bool) -> Result<(PathBuf, bool), EngineError> {
        let resolved = self.resolve_target_path(raw_path, "output path")?;
        #[cfg(target_arch = "wasm32")]
        {
            let use_unpacked = explicit_unpacked || has_unpacked_dir_suffix(&resolved);
            Ok((resolved, use_unpacked))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let use_unpacked =
                explicit_unpacked || has_unpacked_dir_suffix(&resolved) || lumencraft_format::is_unpacked_project_dir(&resolved) || resolved.is_dir();
            if use_unpacked {
                ensure_unpacked_dir_if_present(&resolved, raw_path)?;
            }
            Ok((resolved, use_unpacked))
        }
    }

    fn resolve_target_path(&self, raw_path: &str, label: &str) -> Result<PathBuf, EngineError> {
        validate_raw_path(raw_path, label)?;
        match self {
            Self::Unconstrained => Ok(PathBuf::from(raw_path)),
            #[cfg(not(target_arch = "wasm32"))]
            Self::Workdir(guard) => guard.resolve(raw_path).map_err(EngineError::from),
            Self::RejectExternal => Err(reject_external_err(raw_path)),
        }
    }
}

fn validate_raw_path(raw_path: &str, label: &str) -> Result<(), EngineError> {
    if raw_path.trim().is_empty() {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("{label} path must not be empty")));
    }
    if raw_path.contains('\0') {
        return Err(
            EngineError::with_code(ErrorCode::BadInput, format!("{label} path must not contain NUL bytes")).with_details(json!({ "path": raw_path }))
        );
    }
    Ok(())
}

fn reject_external_err(raw_path: &str) -> EngineError {
    EngineError::with_code(ErrorCode::PathEscape, format!("external filesystem path `{raw_path}` is disabled by engine path policy"))
        .with_remediation("Supply inline data/bytes or run with an unconstrained or workdir-confined filesystem session.")
        .with_details(json!({ "path": raw_path }))
}

#[cfg(not(target_arch = "wasm32"))]
fn ensure_unpacked_dir_if_present(resolved: &Path, raw_path: &str) -> Result<(), EngineError> {
    if let Ok(meta) = fs::metadata(resolved)
        && !meta.is_dir()
    {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("unpacked project path `{raw_path}` must be a directory"))
            .with_details(json!({ "path": raw_path })));
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn read_io_err(raw_path: &str, label: &str, detail: &str) -> EngineError {
    EngineError::with_code(ErrorCode::IoError, format!("failed reading {label} `{raw_path}`: {detail}"))
        .with_remediation("Verify the path exists and is readable.")
        .with_retryable(true)
        .with_details(json!({ "path": raw_path }))
}

#[cfg(not(target_arch = "wasm32"))]
fn map_read_platform_err(err: PlatformError, raw_path: &str, label: &str) -> EngineError {
    match err {
        PlatformError::Io { message, .. } => read_io_err(raw_path, label, &message),
        PlatformError::InvalidPath { reason } => {
            EngineError::with_code(ErrorCode::BadInput, format!("invalid {label} `{raw_path}`: {reason}")).with_details(json!({ "path": raw_path }))
        }
        other => EngineError::from(other),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn write_resolved_atomic(resolved: &Path, display_path: &str, bytes: &[u8]) -> Result<(), EngineError> {
    if let Some(parent) = resolved.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|err| {
            EngineError::with_code(ErrorCode::IoError, format!("failed to create output directory `{}`: {err}", parent.display()))
                .with_remediation("Check parent directory permissions and path validity.")
                .with_retryable(true)
                .with_details(json!({ "path": display_path }))
        })?;
    }
    lumencraft_platform::fs::write_atomic(resolved, bytes).map_err(|err| {
        EngineError::with_code(ErrorCode::IoError, format!("failed writing file `{display_path}`: {err}"))
            .with_remediation("Ensure the target directory is writable and has sufficient disk space.")
            .with_retryable(true)
            .with_details(json!({ "path": display_path }))
    })
}

fn has_unpacked_dir_suffix(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.to_ascii_lowercase().ends_with(".lumencraft.d"))
}

fn sanitize_param_value(policy: &ExternalPathPolicy, val: &mut Value) -> Result<(), EngineError> {
    match val {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if ENGINE_PATH_PARAM_KEYS.contains(&key.as_str()) {
                    sanitize_path_field(policy, key, child)?;
                } else {
                    sanitize_param_value(policy, child)?;
                }
            }
            Ok(())
        }
        Value::Array(items) => {
            for item in items {
                sanitize_param_value(policy, item)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn sanitize_path_field(policy: &ExternalPathPolicy, key: &str, val: &mut Value) -> Result<(), EngineError> {
    match val {
        Value::Null => Ok(()),
        Value::String(raw) => {
            validate_raw_path(raw, key)?;
            match policy {
                ExternalPathPolicy::Unconstrained => Ok(()),
                ExternalPathPolicy::RejectExternal => Err(reject_external_err(raw)),
                #[cfg(not(target_arch = "wasm32"))]
                ExternalPathPolicy::Workdir(guard) => {
                    let resolved = guard.resolve(raw)?;
                    *raw = resolved.to_string_lossy().into_owned();
                    Ok(())
                }
            }
        }
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("parameter `{key}` must be a string path"))),
    }
}
