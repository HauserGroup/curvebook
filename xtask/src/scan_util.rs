//! Shared file-tree walking, text-normalization, identifier matching, and test fixture helpers
//! for `xtask` scanners and static-site checks.

use std::fmt::Display;
use std::path::{Path, PathBuf};

use crate::rust_lex::is_ident_byte;

/// Returns `true` when `path` exists as a non-symlink regular file.
pub(crate) fn is_regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file())
}

/// Returns `true` when `path` exists as a non-symlink directory.
pub(crate) fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_dir())
}

/// Return all direct child paths of `dir` sorted lexicographically.
pub(crate) fn read_dir_paths_sorted(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
    let mut paths = Vec::new();
    for entry in entries {
        paths.push(entry.map_err(|e| format!("dir entry under {}: {e}", dir.display()))?.path());
    }
    paths.sort();
    Ok(paths)
}

/// Slash-normalized path relative to `root`, failing if `path` is not under `root`.
pub(crate) fn rel_slash(root: &Path, path: &Path) -> Result<String, String> {
    let rel = path.strip_prefix(root).map_err(|e| format!("strip prefix {}: {e}", path.display()))?;
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

/// Slash-normalized path relative to `root`, falling back to `path` itself if not prefixed.
pub(crate) fn rel_slash_lossy(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

/// Lowercased file extension of `path`, or an empty string if absent/non-UTF-8.
pub(crate) fn lower_ext(path: &str) -> String {
    Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default()
}

/// Returns `true` when `path`'s extension matches any entry in `exts` (ASCII case-insensitive).
pub(crate) fn has_ext_ci(path: &Path, exts: &[&str]) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|ext| exts.iter().any(|cand| ext.eq_ignore_ascii_case(cand)))
}

/// Returns `true` when `bytes` begins with a ZIP local-file or empty-archive EOCD signature.
pub(crate) fn is_zip_magic(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06")
}

/// Recursively collect non-symlink regular file paths under `dir` in sorted order.
pub(crate) fn walk_file_paths(
    dir: &Path,
    skip_dir: &impl Fn(&Path) -> bool,
    keep_file: &impl Fn(&Path) -> bool,
    out: &mut Vec<PathBuf>,
) -> Result<(), String> {
    for path in read_dir_paths_sorted(dir)? {
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("stat {}: {e}", path.display()))?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            if !skip_dir(&path) {
                walk_file_paths(&path, skip_dir, keep_file, out)?;
            }
        } else if meta.is_file() && keep_file(&path) {
            out.push(path);
        }
    }
    Ok(())
}

/// Recursively collect non-symlink regular files under `dir`, returning sorted `/`-separated paths
/// relative to `root`.
pub(crate) fn walk_rel_files(
    root: &Path,
    dir: &Path,
    skip_dir: &impl Fn(&Path) -> bool,
    keep_file: &impl Fn(&Path) -> bool,
    out: &mut Vec<String>,
) -> Result<(), String> {
    let mut paths = Vec::new();
    walk_file_paths(dir, skip_dir, keep_file, &mut paths)?;
    for path in paths {
        out.push(rel_slash(root, &path)?);
    }
    Ok(())
}

/// Recursively collect regular (non-symlink) files under `dir`, returning sorted `/`-separated paths relative to `root`.
pub(crate) fn collect_dist_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    walk_rel_files(root, dir, &|_| false, &|_| true, out)
}

/// Normalize CRLF line endings to LF.
pub(crate) fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n")
}

/// Strip surrounding matching single or double quotes after trimming whitespace.
pub(crate) fn unquote_scalar(raw: &str) -> &str {
    let trimmed = raw.trim();
    trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| trimmed.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(trimmed)
}

/// Strip a trailing `# ...` TOML comment while respecting single- and double-quoted strings.
pub(crate) fn strip_toml_comment(line: &str) -> &str {
    let mut in_dq = false;
    let mut in_sq = false;
    let mut escaped = false;
    for (i, ch) in line.char_indices() {
        if in_dq {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_dq = false;
            }
        } else if in_sq {
            if ch == '\'' {
                in_sq = false;
            }
        } else {
            match ch {
                '"' => in_dq = true,
                '\'' => in_sq = true,
                '#' => return line.get(..i).unwrap_or(line),
                _ => {}
            }
        }
    }
    line
}

/// Find the earliest identifier-bounded match of any pattern in `patterns` starting at byte offset
/// `from`, breaking ties by preferring the longest pattern.
pub(crate) fn next_ident_pattern_match<'a>(line: &str, from: usize, patterns: &[&'a str]) -> Option<(usize, &'a str)> {
    let bytes = line.as_bytes();
    let mut best: Option<(usize, &'a str)> = None;
    for &pat in patterns {
        let mut search = from;
        while let Some(rel) = line.get(search..).and_then(|s| s.find(pat)) {
            let pos = search + rel;
            let end = pos + pat.len();
            let left_ok = pos == 0 || !is_ident_byte(bytes[pos - 1]);
            let right_ok = end == bytes.len() || !is_ident_byte(bytes[end]);
            if left_ok && right_ok {
                let replace = match best {
                    None => true,
                    Some((best_pos, best_pat)) => pos < best_pos || (pos == best_pos && pat.len() > best_pat.len()),
                };
                if replace {
                    best = Some((pos, pat));
                }
                break;
            }
            search = pos + line.get(pos..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
        }
    }
    best
}

/// Format a list of gate violations as `<count> <heading>:\n  <item1>\n  ...`.
pub(crate) fn format_error_list<T: Display>(heading: &str, items: &[T]) -> String {
    let rendered: Vec<String> = items.iter().map(ToString::to_string).collect();
    format!("{} {heading}:\n  {}", items.len(), rendered.join("\n  "))
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    /// RAII temporary directory/repository fixture that cleans itself up on drop.
    pub(crate) struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        pub(crate) fn new(label: &str) -> Self {
            let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
            let path = std::env::temp_dir().join(format!("cb-xtask-{label}-{}-{nanos}-{seq}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        pub(crate) fn git_repo(label: &str) -> Self {
            let dir = Self::new(label);
            dir.run_git(&["init", "-q"]);
            dir.run_git(&["config", "user.name", "Test"]);
            dir.run_git(&["config", "user.email", "test@example.com"]);
            dir
        }

        pub(crate) fn path(&self) -> &Path {
            &self.path
        }

        pub(crate) fn join(&self, rel: impl AsRef<Path>) -> PathBuf {
            self.path.join(rel)
        }

        pub(crate) fn write(&self, rel: &str, content: &str) -> PathBuf {
            self.write_bytes(rel, content.as_bytes())
        }

        pub(crate) fn write_bytes(&self, rel: &str, bytes: &[u8]) -> PathBuf {
            let full = self.path.join(rel);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&full, bytes).unwrap();
            full
        }

        pub(crate) fn run_git(&self, args: &[&str]) {
            let out = Command::new("git").current_dir(&self.path).args(args).output().unwrap();
            assert!(out.status.success(), "git {:?} failed: {}", args, String::from_utf8_lossy(&out.stderr));
        }

        pub(crate) fn git_add_all(&self) {
            self.run_git(&["add", "-A"]);
        }
    }

    impl AsRef<Path> for TestDir {
        fn as_ref(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// Encode `text` as UTF-16LE or UTF-16BE with an optional BOM.
    pub(crate) fn to_utf16_bytes(text: &str, little_endian: bool, include_bom: bool) -> Vec<u8> {
        let mut out = Vec::with_capacity(text.len() * 2 + 2);
        if include_bom {
            out.extend_from_slice(if little_endian { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
        }
        for unit in text.encode_utf16() {
            out.extend_from_slice(&if little_endian { unit.to_le_bytes() } else { unit.to_be_bytes() });
        }
        out
    }
}
