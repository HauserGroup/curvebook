//! `--workdir` path containment and recursive script/manifest path sanitization.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::error::CliError;

/// JSON parameter keys interpreted as filesystem paths inside engine commands.
///
/// Bare `csv` is deliberately absent: `table.importCsv` and the paste command read `csv` as
/// inline CSV text, so rewriting it as a path would silently import the path string as data.
/// Manifest `inputs[].csv` paths are resolved separately in `parse_manifest_inputs`.
const PATH_PARAM_KEYS: &[&str] = &[
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
    "template",
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
#[derive(Debug, Clone)]
pub struct WorkdirGuard {
    canonical_root: PathBuf,
}

impl WorkdirGuard {
    /// Validate and canonicalize `--workdir`.
    pub fn new(workdir: &Path) -> Result<Self, CliError> {
        let meta = fs::metadata(workdir).map_err(|err| CliError::Usage(format!("--workdir `{}` is inaccessible: {err}", workdir.display())))?;
        if !meta.is_dir() {
            return Err(CliError::Usage(format!("--workdir `{}` must be a directory", workdir.display())));
        }
        let canonical_root =
            fs::canonicalize(workdir).map_err(|err| CliError::Usage(format!("--workdir `{}` could not be resolved: {err}", workdir.display())))?;
        Ok(Self { canonical_root })
    }

    /// Resolve and verify that `raw_path` stays strictly inside `--workdir`.
    pub fn resolve(&self, raw_path: &str) -> Result<PathBuf, CliError> {
        if raw_path.trim().is_empty() {
            return Err(CliError::PathEscape(String::from("empty path is not permitted inside --workdir")));
        }
        let candidate = Path::new(raw_path);
        reject_parent_components(candidate, raw_path)?;

        let joined = if candidate.is_absolute() { candidate.to_path_buf() } else { self.canonical_root.join(candidate) };

        let resolved = resolve_preserving_nonexistent_tail(&joined, raw_path)?;
        if !resolved.starts_with(&self.canonical_root) {
            return Err(CliError::PathEscape(format!("path `{raw_path}` escapes workdir `{}`", self.canonical_root.display())));
        }
        Ok(resolved)
    }
}

/// Strategy for resolving file paths in `run` and `batch`.
#[derive(Debug, Clone)]
pub enum PathResolver {
    /// Strict containment inside `--workdir` (rejects escapes with exit code `2`).
    Sandboxed(WorkdirGuard),
    /// Unsandboxed resolution relative to `base_dir` (when `--workdir` is omitted).
    Unconstrained { base_dir: PathBuf },
}

impl PathResolver {
    /// Resolve a filesystem path string according to the active containment policy.
    pub fn resolve(&self, raw_path: &str) -> Result<PathBuf, CliError> {
        match self {
            Self::Sandboxed(guard) => guard.resolve(raw_path),
            Self::Unconstrained { base_dir } => {
                if raw_path.trim().is_empty() {
                    return Err(CliError::Schema(String::from("empty path string")));
                }
                let p = Path::new(raw_path);
                if p.is_absolute() { Ok(p.to_path_buf()) } else { Ok(base_dir.join(p)) }
            }
        }
    }

    /// Recursively inspect and rewrite any filesystem-path fields in a JSON command parameter tree.
    pub fn sanitize_params(&self, params: &mut Value) -> Result<(), CliError> {
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

fn is_path_key(key: &str) -> bool {
    PATH_PARAM_KEYS.contains(&key)
}

fn sanitize_path_value(resolver: &PathResolver, key: &str, val: &mut Value) -> Result<(), CliError> {
    match val {
        Value::String(raw) => {
            let resolved = resolver.resolve(raw)?;
            *raw = resolved.to_string_lossy().into_owned();
            Ok(())
        }
        Value::Null => Ok(()),
        _ => Err(CliError::Schema(format!("parameter `{key}` must be a string path"))),
    }
}

fn reject_parent_components(path: &Path, raw: &str) -> Result<(), CliError> {
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                return Err(CliError::PathEscape(format!("parent directory `..` is forbidden in path `{raw}`")));
            }
            Component::Prefix(_) | Component::RootDir | Component::CurDir | Component::Normal(_) => {}
        }
    }
    Ok(())
}

/// Canonicalize the deepest existing ancestor of `joined` (rejecting broken symlink escapes too),
/// then append the normalized non-existing suffix.
fn resolve_preserving_nonexistent_tail(joined: &Path, raw: &str) -> Result<PathBuf, CliError> {
    let mut existing = joined.to_path_buf();
    let mut tail_parts: Vec<std::ffi::OsString> = Vec::new();

    loop {
        match fs::symlink_metadata(&existing) {
            Ok(meta) => {
                if meta.file_type().is_symlink() && fs::canonicalize(&existing).is_err() {
                    return Err(CliError::PathEscape(format!("dangling symlink in path `{raw}`")));
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

    let mut base =
        fs::canonicalize(&existing).map_err(|err| CliError::PathEscape(format!("could not canonicalize `{}`: {err}", existing.display())))?;
    for part in tail_parts.into_iter().rev() {
        base.push(part);
    }
    Ok(base)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{PathResolver, WorkdirGuard};

    #[test]
    fn sandbox_rewrites_path_keys_but_keeps_inline_csv_text() {
        let dir = std::env::temp_dir().join(format!("lumencraft_sandbox_csv_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let guard = WorkdirGuard::new(&dir).unwrap();
        let resolver = PathResolver::Sandboxed(guard);

        let inline = "x,y\n1,2\n../3,4\n";
        let mut params = json!({ "csv": inline, "path": "out.csv" });
        resolver.sanitize_params(&mut params).unwrap();

        assert_eq!(params["csv"], json!(inline), "inline CSV text must not be rewritten as a path");
        let canonical = std::fs::canonicalize(&dir).unwrap();
        assert_eq!(params["path"], json!(canonical.join("out.csv").to_string_lossy()));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
