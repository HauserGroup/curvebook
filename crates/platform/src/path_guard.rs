//! `--workdir` canonical path containment and recursive JSON command parameter sanitization.

#[cfg(not(target_arch = "wasm32"))]
use std::ffi::OsString;
#[cfg(not(target_arch = "wasm32"))]
use std::fs;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Component, Path, PathBuf};

#[cfg(not(target_arch = "wasm32"))]
use serde_json::Value;

#[cfg(not(target_arch = "wasm32"))]
use crate::PlatformError;

/// JSON parameter keys interpreted as filesystem paths inside engine commands.
///
/// Bare `csv` and `template` are deliberately absent: `table.importCsv` and the paste command
/// read `csv` as inline CSV text, and `table.importPlateMap` reads `template` as an inline
/// `PlateMapTemplate` JSON object or inline `.lctemplate` JSON string. File-path parameters for
/// templates use `path`, `template_path`, or `templatePath`, while batch manifest `inputs[].csv`
/// and root `template` paths are resolved separately by batch manifest loaders.
pub const PATH_PARAM_KEYS: &[&str] = &[
    "path",
    "file",
    "file_path",
    "filePath",
    "out",
    "output",
    "output_path",
    "outputPath",
    "in",
    "input",
    "input_path",
    "inputPath",
    "source_path",
    "sourcePath",
    "target_path",
    "targetPath",
    "template_path",
    "templatePath",
    "project",
    "project_path",
    "projectPath",
    "csv_path",
    "csvPath",
    "ttf_font",
    "ttfFont",
    "font_path",
    "fontPath",
];

/// Canonicalized `--workdir` guard rejecting parent traversal (`..`), outside paths, and symlink escapes.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkdirGuard {
    canonical_root: PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl WorkdirGuard {
    /// Validate and canonicalize `workdir`.
    ///
    /// # Errors
    /// Returns [`PlatformError::InvalidPath`] if `workdir` is inaccessible or not a directory, or
    /// [`PlatformError::Io`] if canonicalization fails.
    pub fn new(workdir: &Path) -> Result<Self, PlatformError> {
        let meta = fs::metadata(workdir)
            .map_err(|err| PlatformError::InvalidPath { reason: format!("workdir `{}` is inaccessible: {err}", workdir.display()) })?;
        if !meta.is_dir() {
            return Err(PlatformError::InvalidPath { reason: format!("workdir `{}` must be a directory", workdir.display()) });
        }
        let canonical_root = fs::canonicalize(workdir)
            .map_err(|err| PlatformError::Io { context: format!("canonicalize workdir `{}`", workdir.display()), message: err.to_string() })?;
        Ok(Self { canonical_root })
    }

    /// Borrow the canonical root directory guarded by this instance.
    #[must_use]
    pub fn canonical_root(&self) -> &Path {
        &self.canonical_root
    }

    /// Resolve and verify that `raw_path` stays strictly inside [`Self::canonical_root`].
    ///
    /// # Errors
    /// Returns [`PlatformError::InvalidPath`] if `raw_path` is empty or contains NUL bytes, or
    /// [`PlatformError::PathEscape`] if `raw_path` traverses `..` or resolves outside the workdir.
    pub fn resolve(&self, raw_path: &str) -> Result<PathBuf, PlatformError> {
        validate_raw_path(raw_path)?;
        let candidate = Path::new(raw_path);
        reject_parent_components(candidate, raw_path, &self.canonical_root)?;

        let joined = if candidate.is_absolute() { candidate.to_path_buf() } else { self.canonical_root.join(candidate) };
        let resolved = resolve_preserving_nonexistent_tail(&joined, raw_path, &self.canonical_root)?;
        if !resolved.starts_with(&self.canonical_root) {
            return Err(PlatformError::PathEscape { path: raw_path.to_owned(), workdir: self.canonical_root.display().to_string() });
        }
        Ok(resolved)
    }

    /// Resolve `raw_path` inside [`Self::canonical_root`] and verify it names an existing regular file.
    ///
    /// Inspects file metadata without opening the target, rejecting directories, FIFOs, sockets, and
    /// device nodes cleanly before any read begins.
    ///
    /// # Errors
    /// Returns [`PlatformError::InvalidPath`] for blank paths, NUL bytes, or non-regular files,
    /// [`PlatformError::PathEscape`] if `raw_path` traverses or symlinks outside the workdir, or
    /// [`PlatformError::Io`] if metadata inspection fails.
    pub fn resolve_existing_file(&self, raw_path: &str) -> Result<PathBuf, PlatformError> {
        let resolved = self.resolve(raw_path)?;
        ensure_regular_file(&resolved)?;
        Ok(resolved)
    }
}

/// Verify via non-blocking metadata inspection that `path` points to an existing regular file.
///
/// Follows symlinks at OS metadata level without opening FIFOs or special device files.
///
/// # Errors
/// Returns [`PlatformError::Io`] if `path` cannot be inspected, or [`PlatformError::InvalidPath`]
/// if `path` refers to a directory, FIFO, socket, or device node rather than a regular file.
#[cfg(not(target_arch = "wasm32"))]
pub fn ensure_regular_file(path: &Path) -> Result<(), PlatformError> {
    let meta =
        fs::metadata(path).map_err(|err| PlatformError::Io { context: format!("inspect file `{}`", path.display()), message: err.to_string() })?;
    if !meta.is_file() {
        return Err(PlatformError::InvalidPath { reason: format!("path `{}` must be a regular file", path.display()) });
    }
    Ok(())
}

/// Strategy for resolving file paths in headless scripts, manifests, and guarded host adapters.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathResolver {
    /// Strict containment inside `--workdir`.
    Sandboxed(WorkdirGuard),
    /// Unsandboxed resolution relative to `base_dir` (when `--workdir` is omitted).
    Unconstrained { base_dir: PathBuf },
}

#[cfg(not(target_arch = "wasm32"))]
impl PathResolver {
    /// Resolve a filesystem path string according to the active containment policy.
    ///
    /// # Errors
    /// Returns [`PlatformError::InvalidPath`] or [`PlatformError::PathEscape`] on invalid or
    /// escaping paths.
    pub fn resolve(&self, raw_path: &str) -> Result<PathBuf, PlatformError> {
        match self {
            Self::Sandboxed(guard) => guard.resolve(raw_path),
            Self::Unconstrained { base_dir } => {
                validate_raw_path(raw_path)?;
                let path = Path::new(raw_path);
                if path.is_absolute() { Ok(path.to_path_buf()) } else { Ok(base_dir.join(path)) }
            }
        }
    }

    /// Recursively inspect and rewrite any filesystem-path fields in a JSON command parameter tree.
    ///
    /// # Errors
    /// Returns [`PlatformError`] if a path field has a non-string non-null value or violates
    /// containment rules.
    pub fn sanitize_params(&self, params: &mut Value) -> Result<(), PlatformError> {
        match params {
            Value::Object(map) => {
                for (key, val) in map.iter_mut() {
                    if is_path_key(key) {
                        sanitize_path_value(self, key, val)?;
                    } else {
                        self.sanitize_params(val)?;
                    }
                }
                Ok(())
            }
            Value::Array(items) => {
                for item in items {
                    self.sanitize_params(item)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn validate_raw_path(raw_path: &str) -> Result<(), PlatformError> {
    if raw_path.trim().is_empty() {
        return Err(PlatformError::InvalidPath { reason: String::from("empty path is not permitted") });
    }
    if raw_path.contains('\0') {
        return Err(PlatformError::InvalidPath { reason: String::from("path must not contain NUL bytes") });
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn is_path_key(key: &str) -> bool {
    PATH_PARAM_KEYS.contains(&key)
}

#[cfg(not(target_arch = "wasm32"))]
fn sanitize_path_value(resolver: &PathResolver, key: &str, val: &mut Value) -> Result<(), PlatformError> {
    match val {
        Value::String(raw) => {
            let resolved = resolver.resolve(raw)?;
            *raw = resolved.to_string_lossy().into_owned();
            Ok(())
        }
        Value::Null => Ok(()),
        _ => Err(PlatformError::InvalidPath { reason: format!("parameter `{key}` must be a string path") }),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn reject_parent_components(path: &Path, raw: &str, canonical_root: &Path) -> Result<(), PlatformError> {
    for comp in path.components() {
        if matches!(comp, Component::ParentDir) {
            return Err(PlatformError::PathEscape { path: raw.to_owned(), workdir: canonical_root.display().to_string() });
        }
    }
    Ok(())
}

/// Canonicalize the deepest existing ancestor of `joined` (rejecting broken symlink escapes too),
/// then append the normalized non-existing suffix.
#[cfg(not(target_arch = "wasm32"))]
fn resolve_preserving_nonexistent_tail(joined: &Path, raw: &str, canonical_root: &Path) -> Result<PathBuf, PlatformError> {
    let mut existing = joined.to_path_buf();
    let mut tail_parts: Vec<OsString> = Vec::new();
    let workdir_str = || canonical_root.display().to_string();

    loop {
        match fs::symlink_metadata(&existing) {
            Ok(meta) => {
                if meta.file_type().is_symlink() && fs::canonicalize(&existing).is_err() {
                    return Err(PlatformError::PathEscape { path: raw.to_owned(), workdir: workdir_str() });
                }
                break;
            }
            Err(_) => {
                let Some(name) = existing.file_name().map(std::ffi::OsStr::to_os_string) else {
                    break;
                };
                let Some(parent) = existing.parent().map(Path::to_path_buf) else {
                    break;
                };
                tail_parts.push(name);
                existing = parent;
            }
        }
    }

    let mut base = fs::canonicalize(&existing).map_err(|_| PlatformError::PathEscape { path: raw.to_owned(), workdir: workdir_str() })?;
    for part in tail_parts.into_iter().rev() {
        base.push(part);
    }
    Ok(base)
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use serde_json::json;

    use super::{PathResolver, WorkdirGuard, ensure_regular_file};
    use crate::PlatformError;

    static TEST_SEQ: AtomicU64 = AtomicU64::new(1);

    fn temp_dir(label: &str) -> PathBuf {
        let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("lumencraft-platform-guard-{label}-{}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sandbox_rewrites_path_keys_and_preserves_inline_csv_text() {
        let dir = temp_dir("csv");
        let guard = WorkdirGuard::new(&dir).unwrap();
        let resolver = PathResolver::Sandboxed(guard);

        let inline = "x,y\n1,2\n../3,4\n";
        let mut params = json!({ "csv": inline, "path": "nested/out.csv", "template": null });
        resolver.sanitize_params(&mut params).unwrap();

        assert_eq!(params["csv"], json!(inline));
        let canonical = std::fs::canonicalize(&dir).unwrap();
        assert_eq!(params["path"], json!(canonical.join("nested").join("out.csv").to_string_lossy()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn preserves_inline_plate_template_payloads_while_guarding_template_paths() {
        let dir = temp_dir("template-param");
        let guard = WorkdirGuard::new(&dir).unwrap();
        let canonical = std::fs::canonicalize(&dir).unwrap();

        let tpl_obj = json!({ "name": "96-well ../layout", "geometry": "96", "wells": [] });
        let tpl_str = r#"{"format":"lumencraft-archive","kind":"template","path":"../ignored"}"#;

        for resolver in [PathResolver::Unconstrained { base_dir: dir.clone() }, PathResolver::Sandboxed(guard)] {
            let mut obj_params = json!({
                "template": tpl_obj.clone(),
                "template_path": "templates/plate.lctemplate",
            });
            resolver.sanitize_params(&mut obj_params).unwrap();
            assert_eq!(obj_params["template"], tpl_obj);
            match &resolver {
                PathResolver::Unconstrained { .. } => {
                    assert_eq!(obj_params["template_path"], json!(dir.join("templates").join("plate.lctemplate").to_string_lossy()));
                }
                PathResolver::Sandboxed(_) => {
                    assert_eq!(obj_params["template_path"], json!(canonical.join("templates").join("plate.lctemplate").to_string_lossy()));
                }
            }

            let mut str_params = json!({ "template": tpl_str });
            resolver.sanitize_params(&mut str_params).unwrap();
            assert_eq!(str_params["template"], json!(tpl_str));
        }

        let escape_guard = WorkdirGuard::new(&dir).unwrap();
        let escape_resolver = PathResolver::Sandboxed(escape_guard);
        let mut escape_params = json!({ "template": tpl_obj, "templatePath": "../outside.lctemplate" });
        assert!(matches!(escape_resolver.sanitize_params(&mut escape_params), Err(PlatformError::PathEscape { .. })));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_empty_parent_traversal_and_outside_paths() {
        let dir = temp_dir("escape");
        let outside = temp_dir("outside");
        let guard = WorkdirGuard::new(&dir).unwrap();

        assert!(matches!(guard.resolve(""), Err(PlatformError::InvalidPath { .. })));
        assert!(matches!(guard.resolve("   "), Err(PlatformError::InvalidPath { .. })));
        assert!(matches!(guard.resolve("../secret.txt"), Err(PlatformError::PathEscape { .. })));
        assert!(matches!(guard.resolve("sub/../../secret.txt"), Err(PlatformError::PathEscape { .. })));

        let outside_file = outside.join("escape.txt").to_string_lossy().into_owned();
        assert!(matches!(guard.resolve(&outside_file), Err(PlatformError::PathEscape { .. })));

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_and_dangling_symlink_escapes_even_with_nonexistent_tails() {
        use std::os::unix::fs::symlink;

        let dir = temp_dir("symlink-wd");
        let outside = temp_dir("symlink-outside");
        let guard = WorkdirGuard::new(&dir).unwrap();

        let dir_link = dir.join("out_link");
        symlink(&outside, &dir_link).unwrap();
        assert!(matches!(guard.resolve("out_link"), Err(PlatformError::PathEscape { .. })));
        assert!(matches!(guard.resolve("out_link/nested/new_file.txt"), Err(PlatformError::PathEscape { .. })));

        let dangling_link = dir.join("dangling_link");
        symlink(outside.join("does_not_exist"), &dangling_link).unwrap();
        assert!(matches!(guard.resolve("dangling_link"), Err(PlatformError::PathEscape { .. })));
        assert!(matches!(guard.resolve("dangling_link/child.txt"), Err(PlatformError::PathEscape { .. })));

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn resolve_existing_file_allows_regular_files_and_rejects_directories_and_non_regular_targets() {
        let dir = temp_dir("regular-file");
        let outside = temp_dir("regular-outside");
        let guard = WorkdirGuard::new(&dir).unwrap();

        let good_file = dir.join("inside.csv");
        std::fs::write(&good_file, "a,b\n1,2\n").unwrap();
        let resolved = guard.resolve_existing_file("inside.csv").unwrap();
        assert_eq!(resolved, std::fs::canonicalize(&good_file).unwrap());

        let subdir = dir.join("subdir");
        std::fs::create_dir_all(&subdir).unwrap();
        assert!(matches!(guard.resolve_existing_file("subdir"), Err(PlatformError::InvalidPath { .. })));
        assert!(matches!(guard.resolve_existing_file("missing.csv"), Err(PlatformError::Io { .. })));

        let outside_file = outside.join("secret.csv");
        std::fs::write(&outside_file, "secret").unwrap();
        assert!(matches!(guard.resolve_existing_file(&outside_file.to_string_lossy()), Err(PlatformError::PathEscape { .. })));

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link_to_outside = dir.join("escape.csv");
            symlink(&outside_file, &link_to_outside).unwrap();
            assert!(matches!(guard.resolve_existing_file("escape.csv"), Err(PlatformError::PathEscape { .. })));

            let dev_null = std::path::Path::new("/dev/null");
            if dev_null.exists() {
                assert!(matches!(ensure_regular_file(dev_null), Err(PlatformError::InvalidPath { .. })));
                let link_to_dev_null = dir.join("devnull_link");
                symlink(dev_null, &link_to_dev_null).unwrap();
                assert!(matches!(guard.resolve_existing_file("devnull_link"), Err(PlatformError::PathEscape { .. })));
            }

            let fifo = dir.join("pipe.csv");
            if let Ok(status) = std::process::Command::new("mkfifo").arg(&fifo).status()
                && status.success()
            {
                assert!(matches!(guard.resolve_existing_file("pipe.csv"), Err(PlatformError::InvalidPath { .. })));
            }
        }

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside);
    }
}
