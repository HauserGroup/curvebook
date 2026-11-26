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

/// Archive file extensions that use ZIP containers and must always fail closed on corrupt/uninspectable payloads.
pub(crate) const ZIP_CONTAINER_EXTS: &[&str] = &["zip", "curvebook", "cbtemplate", "prism", "pzf", "xlsx"];

/// Returns `true` when `bytes` begins with a ZIP local-file, empty-archive EOCD, or spanned-archive signature.
pub(crate) fn is_zip_magic(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") || bytes.starts_with(b"PK\x07\x08")
}

/// Returns `true` when `bytes` starts with a ZIP signature OR ends with a valid ZIP EOCD record
/// (catching PKWARE-valid preamble-prefixed ZIP containers).
pub(crate) fn looks_like_zip(bytes: &[u8]) -> bool {
    is_zip_magic(bytes) || crate::zip_read::has_zip_eocd(bytes)
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

/// Decoded TOML 1.0 value parsed for manifest policy scanners (`telemetry`, `layers`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TomlVal {
    Str(String),
    Array(Vec<TomlVal>),
    InlineTable(Vec<(Vec<String>, TomlVal)>),
    Other(String),
}

impl TomlVal {
    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Recursively collect all decoded string scalars contained in this value (scalar or array).
    pub(crate) fn collect_strings<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Self::Str(s) => out.push(s.as_str()),
            Self::Array(items) => {
                for item in items {
                    item.collect_strings(out);
                }
            }
            Self::InlineTable(pairs) => {
                for (_, val) in pairs {
                    val.collect_strings(out);
                }
            }
            Self::Other(_) => {}
        }
    }
}

/// Flattened TOML 1.0 entry (table header or leaf key-value with full section + dotted + inline path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TomlEntry {
    Section { line: usize, path: Vec<String> },
    Leaf { line: usize, path: Vec<String>, value: TomlVal },
}

/// Parse a TOML 1.0 document into normalized section headers and flattened leaf assignments,
/// supporting basic (`"..."`), literal (`'...'`), multiline basic (`"""..."""` with line-ending
/// backslash trimming), multiline literal (`'''...'''`), `\uXXXX`/`\UXXXXXXXX` escapes, dotted
/// keys, quoted keys, multiline arrays, and multiline/nested inline tables.
pub(crate) fn parse_toml_entries(src: &str) -> Vec<TomlEntry> {
    let mut parser = TomlParser { src, bytes: src.as_bytes(), pos: 0, line: 1 };
    let mut out = Vec::new();
    let mut cur_section: Vec<String> = Vec::new();
    while !parser.is_eof() {
        parser.skip_ws_and_comments(true);
        if parser.is_eof() {
            break;
        }
        let line = parser.line;
        if parser.peek() == Some(b'[') {
            parser.advance_one();
            let is_array_table = parser.peek() == Some(b'[');
            if is_array_table {
                parser.advance_one();
            }
            let path = parser.read_dotted_key();
            parser.skip_ws_and_comments(false);
            if parser.peek() == Some(b']') {
                parser.advance_one();
                if is_array_table && parser.peek() == Some(b']') {
                    parser.advance_one();
                }
            } else {
                parser.skip_to_eol();
            }
            cur_section = path.clone();
            if !path.is_empty() {
                out.push(TomlEntry::Section { line, path });
            }
            continue;
        }
        let key_segs = parser.read_dotted_key();
        parser.skip_ws_and_comments(false);
        if !key_segs.is_empty() && parser.peek() == Some(b'=') {
            parser.advance_one();
            let val = parser.read_value(0);
            let mut full_path = cur_section.clone();
            full_path.extend(key_segs);
            flatten_toml_leaf(line, &full_path, val, &mut out);
        } else {
            parser.skip_to_eol();
        }
    }
    out
}

fn flatten_toml_leaf(line: usize, prefix: &[String], val: TomlVal, out: &mut Vec<TomlEntry>) {
    match val {
        TomlVal::InlineTable(pairs) => {
            if pairs.is_empty() {
                out.push(TomlEntry::Leaf { line, path: prefix.to_vec(), value: TomlVal::Other(String::new()) });
                return;
            }
            for (sub_keys, sub_val) in pairs {
                let mut next = prefix.to_vec();
                next.extend(sub_keys);
                flatten_toml_leaf(line, &next, sub_val, out);
            }
        }
        other => out.push(TomlEntry::Leaf { line, path: prefix.to_vec(), value: other }),
    }
}

struct TomlParser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: usize,
}

impl<'a> TomlParser<'a> {
    fn is_eof(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn advance_one(&mut self) -> Option<u8> {
        let b = *self.bytes.get(self.pos)?;
        self.pos += 1;
        if b == b'\n' {
            self.line += 1;
        }
        Some(b)
    }

    fn skip_to_eol(&mut self) {
        while let Some(b) = self.advance_one() {
            if b == b'\n' {
                break;
            }
        }
    }

    fn skip_ws_and_comments(&mut self, allow_newlines: bool) {
        while let Some(b) = self.peek() {
            match b {
                b' ' | b'\t' => {
                    self.advance_one();
                }
                b'\r' | b'\n' if allow_newlines => {
                    self.advance_one();
                }
                b'#' => {
                    while let Some(c) = self.peek() {
                        if c == b'\n' {
                            break;
                        }
                        self.advance_one();
                    }
                    if !allow_newlines {
                        break;
                    }
                }
                _ => break,
            }
        }
    }

    fn read_dotted_key(&mut self) -> Vec<String> {
        let mut segs = Vec::new();
        loop {
            self.skip_ws_and_comments(false);
            let seg = match self.peek() {
                Some(b'"' | b'\'') => self.read_string(),
                Some(_) => self.read_bare_key(),
                None => None,
            };
            let Some(seg) = seg else {
                break;
            };
            segs.push(seg);
            self.skip_ws_and_comments(false);
            if self.peek() == Some(b'.') {
                self.advance_one();
            } else {
                break;
            }
        }
        segs
    }

    fn read_bare_key(&mut self) -> Option<String> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-') {
                self.pos += 1;
            } else {
                break;
            }
        }
        (self.pos > start).then(|| self.src.get(start..self.pos).unwrap_or("").to_owned())
    }

    fn read_string(&mut self) -> Option<String> {
        if self.bytes.get(self.pos..self.pos + 3) == Some(b"\"\"\"") {
            return Some(self.read_multiline_basic_string());
        }
        if self.bytes.get(self.pos..self.pos + 3) == Some(b"'''") {
            return Some(self.read_multiline_literal_string());
        }
        match self.peek()? {
            b'"' => Some(self.read_single_basic_string()),
            b'\'' => Some(self.read_single_literal_string()),
            _ => None,
        }
    }

    fn skip_immediate_newline(&mut self) {
        if self.peek() == Some(b'\n') {
            self.advance_one();
        } else if self.bytes.get(self.pos..self.pos + 2) == Some(b"\r\n") {
            self.advance_one();
            self.advance_one();
        }
    }

    fn read_single_literal_string(&mut self) -> String {
        self.advance_one(); // '
        let mut out = String::new();
        while !self.is_eof() {
            if self.peek() == Some(b'\'') {
                self.advance_one();
                break;
            }
            if self.peek() == Some(b'\n') {
                break;
            }
            self.push_next_char(&mut out);
        }
        out
    }

    fn read_multiline_literal_string(&mut self) -> String {
        self.pos += 3;
        self.skip_immediate_newline();
        let mut out = String::new();
        while !self.is_eof() {
            if self.bytes.get(self.pos..self.pos + 3) == Some(b"'''") {
                let mut quotes = 0usize;
                while self.bytes.get(self.pos + quotes) == Some(&b'\'') && quotes < 5 {
                    quotes += 1;
                }
                for _ in 0..quotes.saturating_sub(3) {
                    out.push('\'');
                }
                self.pos += quotes;
                break;
            }
            self.push_next_char(&mut out);
        }
        out
    }

    fn read_single_basic_string(&mut self) -> String {
        self.advance_one(); // "
        let mut out = String::new();
        while !self.is_eof() {
            match self.peek() {
                Some(b'"') => {
                    self.advance_one();
                    break;
                }
                Some(b'\n') => break,
                Some(b'\\') => {
                    self.advance_one();
                    self.decode_basic_escape(&mut out, false);
                }
                _ => self.push_next_char(&mut out),
            }
        }
        out
    }

    fn read_multiline_basic_string(&mut self) -> String {
        self.pos += 3;
        self.skip_immediate_newline();
        let mut out = String::new();
        while !self.is_eof() {
            if self.bytes.get(self.pos..self.pos + 3) == Some(b"\"\"\"") {
                let mut quotes = 0usize;
                while self.bytes.get(self.pos + quotes) == Some(&b'"') && quotes < 5 {
                    quotes += 1;
                }
                for _ in 0..quotes.saturating_sub(3) {
                    out.push('"');
                }
                self.pos += quotes;
                break;
            }
            if self.peek() == Some(b'\\') {
                self.advance_one();
                self.decode_basic_escape(&mut out, true);
                continue;
            }
            self.push_next_char(&mut out);
        }
        out
    }

    fn decode_basic_escape(&mut self, out: &mut String, allow_line_trim: bool) {
        if allow_line_trim {
            let mut look = self.pos;
            while matches!(self.bytes.get(look), Some(b' ' | b'\t' | b'\r')) {
                look += 1;
            }
            if self.bytes.get(look) == Some(&b'\n') {
                while self.pos <= look {
                    self.advance_one();
                }
                while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
                    self.advance_one();
                }
                return;
            }
        }
        let Some(esc) = self.advance_one() else {
            return;
        };
        match esc {
            b'b' => out.push('\u{0008}'),
            b't' => out.push('\t'),
            b'n' => out.push('\n'),
            b'f' => out.push('\u{000C}'),
            b'r' => out.push('\r'),
            b'"' => out.push('"'),
            b'\\' => out.push('\\'),
            b'u' | b'U' => {
                let digits = if esc == b'u' { 4 } else { 8 };
                if let Some(hex) = self.src.get(self.pos..self.pos + digits)
                    && let Ok(code) = u32::from_str_radix(hex, 16)
                    && let Some(ch) = char::from_u32(code)
                {
                    out.push(ch);
                    self.pos += digits;
                }
            }
            other => out.push(char::from(other)),
        }
    }

    fn push_next_char(&mut self, out: &mut String) {
        if let Some(ch) = self.src.get(self.pos..).and_then(|s| s.chars().next()) {
            out.push(ch);
            self.pos += ch.len_utf8();
            if ch == '\n' {
                self.line += 1;
            }
        } else {
            self.advance_one();
        }
    }

    fn read_value(&mut self, depth: usize) -> TomlVal {
        self.skip_ws_and_comments(false);
        if depth >= 32 {
            return self.read_other_scalar();
        }
        match self.peek() {
            Some(b'"' | b'\'') => TomlVal::Str(self.read_string().unwrap_or_default()),
            Some(b'[') => {
                self.advance_one();
                let mut items = Vec::new();
                loop {
                    self.skip_ws_and_comments(true);
                    if self.is_eof() || self.peek() == Some(b']') {
                        self.advance_one();
                        break;
                    }
                    let before = self.pos;
                    items.push(self.read_value(depth + 1));
                    self.skip_ws_and_comments(true);
                    if self.peek() == Some(b',') {
                        self.advance_one();
                    } else if self.peek() == Some(b']') {
                        self.advance_one();
                        break;
                    } else if self.pos == before {
                        self.advance_one();
                    }
                }
                TomlVal::Array(items)
            }
            Some(b'{') => {
                self.advance_one();
                let mut pairs = Vec::new();
                loop {
                    self.skip_ws_and_comments(true);
                    if self.is_eof() || self.peek() == Some(b'}') {
                        self.advance_one();
                        break;
                    }
                    let before = self.pos;
                    let keys = self.read_dotted_key();
                    self.skip_ws_and_comments(true);
                    if !keys.is_empty() && self.peek() == Some(b'=') {
                        self.advance_one();
                        self.skip_ws_and_comments(true);
                        let val = self.read_value(depth + 1);
                        pairs.push((keys, val));
                    }
                    self.skip_ws_and_comments(true);
                    if self.peek() == Some(b',') {
                        self.advance_one();
                    } else if self.peek() == Some(b'}') {
                        self.advance_one();
                        break;
                    } else if self.pos == before {
                        self.advance_one();
                    }
                }
                TomlVal::InlineTable(pairs)
            }
            _ => self.read_other_scalar(),
        }
    }

    fn read_other_scalar(&mut self) -> TomlVal {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if matches!(b, b',' | b']' | b'}' | b'#' | b'\n' | b'\r') {
                break;
            }
            self.pos += 1;
        }
        TomlVal::Other(self.src.get(start..self.pos).unwrap_or("").trim().to_owned())
    }
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
