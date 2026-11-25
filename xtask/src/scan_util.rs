//! Shared file-tree walking and text-normalization helpers for `xtask` scanners.

use std::path::Path;

/// Recursively collect regular (non-symlink) files under `dir`, returning sorted `/`-separated paths relative to `root`.
pub(crate) fn collect_dist_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
    let mut paths = Vec::new();
    for entry in entries {
        paths.push(entry.map_err(|e| format!("dir entry under {}: {e}", dir.display()))?.path());
    }
    paths.sort();
    for path in paths {
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("stat {}: {e}", path.display()))?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            collect_dist_files(root, &path, out)?;
        } else if meta.is_file() {
            let rel = path.strip_prefix(root).map_err(|e| format!("strip prefix {}: {e}", path.display()))?;
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

/// Normalize CRLF line endings to LF.
pub(crate) fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n")
}
