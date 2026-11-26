//! Shared file-tree walking, HTML/TOML parsing, text-normalization, identifier matching, and test
//! fixture helpers for `xtask` scanners and static-site checks.

use std::fmt::Display;
use std::path::{Path, PathBuf};

use crate::rust_lex::is_ident_byte;
use crate::text_decode::unescape_html_attr;

pub(crate) const ZIP_CONTAINER_EXTS: &[&str] = &["zip", "curvebook", "cbtemplate", "prism", "pzf", "xlsx"];

pub(crate) fn is_regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file())
}
pub(crate) fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_dir())
}
pub(crate) fn read_dir_paths_sorted(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("read_dir {}: {e}", dir.display()))?
        .map(|entry| entry.map(|e| e.path()).map_err(|e| format!("dir entry under {}: {e}", dir.display())))
        .collect::<Result<_, _>>()?;
    paths.sort();
    Ok(paths)
}
pub(crate) fn rel_slash(root: &Path, path: &Path) -> Result<String, String> {
    Ok(path.strip_prefix(root).map_err(|e| format!("strip prefix {}: {e}", path.display()))?.to_string_lossy().replace('\\', "/"))
}
pub(crate) fn rel_slash_lossy(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}
pub(crate) fn lower_ext(path: &str) -> String {
    Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default()
}
pub(crate) fn has_ext_ci(path: &Path, exts: &[&str]) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|ext| exts.iter().any(|c| ext.eq_ignore_ascii_case(c)))
}
pub(crate) fn looks_like_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") || bytes.starts_with(b"PK\x07\x08") || crate::zip_read::has_zip_eocd(bytes)
}

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
        if meta.is_dir() && !skip_dir(&path) {
            walk_file_paths(&path, skip_dir, keep_file, out)?;
        } else if meta.is_file() && keep_file(&path) {
            out.push(path);
        }
    }
    Ok(())
}

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

pub(crate) fn collect_dist_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    walk_rel_files(root, dir, &|_| false, &|_| true, out)
}

#[cfg_attr(not(feature = "docs-site"), allow(dead_code))]
pub(crate) fn collect_files_no_symlinks(root: &Path, dir: &Path, symlink_msg: &str, out: &mut Vec<String>) -> Result<(), String> {
    for path in read_dir_paths_sorted(dir)? {
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("stat {}: {e}", path.display()))?;
        if meta.file_type().is_symlink() {
            return Err(format!("{}: {symlink_msg}", path.display()));
        }
        if meta.file_type().is_dir() {
            collect_files_no_symlinks(root, &path, symlink_msg, out)?;
        } else if meta.file_type().is_file() {
            out.push(rel_slash(root, &path)?);
        }
    }
    Ok(())
}

#[cfg_attr(not(feature = "docs-site"), allow(dead_code))]
pub(crate) fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n")
}
pub(crate) fn unquote_scalar(raw: &str) -> &str {
    let t = raw.trim();
    t.strip_prefix('"').and_then(|s| s.strip_suffix('"')).or_else(|| t.strip_prefix('\'').and_then(|s| s.strip_suffix('\''))).unwrap_or(t)
}

pub(crate) fn strip_hash_comment(line: &str, require_ws_before_hash: bool) -> &str {
    let (bytes, mut quote, mut esc) = (line.as_bytes(), 0u8, false);
    for (i, &b) in bytes.iter().enumerate() {
        if quote == b'"' {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                quote = 0;
            }
        } else if quote == b'\'' {
            if b == b'\'' {
                quote = 0;
            }
        } else if matches!(b, b'"' | b'\'') {
            quote = b;
        } else if b == b'#' && (!require_ws_before_hash || i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            return line.get(..i).unwrap_or(line);
        }
    }
    line
}
pub(crate) fn strip_toml_comment(line: &str) -> &str {
    strip_hash_comment(line, false)
}

pub(crate) fn next_ident_pattern_match<'a>(line: &str, from: usize, patterns: &[&'a str]) -> Option<(usize, &'a str)> {
    let (bytes, mut best) = (line.as_bytes(), None::<(usize, &'a str)>);
    for &pat in patterns {
        let mut search = from;
        while let Some(rel) = line.get(search..).and_then(|s| s.find(pat)) {
            let (pos, end) = (search + rel, search + rel + pat.len());
            if (pos == 0 || !is_ident_byte(bytes[pos - 1])) && (end == bytes.len() || !is_ident_byte(bytes[end])) {
                if best.is_none_or(|(bp, bpat)| pos < bp || (pos == bp && pat.len() > bpat.len())) {
                    best = Some((pos, pat));
                }
                break;
            }
            search = pos + line.get(pos..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
        }
    }
    best
}

pub(crate) fn format_error_list<T: Display>(heading: &str, items: &[T]) -> String {
    let rendered: Vec<String> = items.iter().map(ToString::to_string).collect();
    format!("{} {heading}:\n  {}", items.len(), rendered.join("\n  "))
}

// ---------- Shared HTML5 tag/comment/attribute scanning ----------

pub(crate) struct HtmlAttr<'a> {
    pub(crate) key: &'a str,
    pub(crate) value: String,
    pub(crate) value_span: Option<(usize, usize)>,
}

pub(crate) fn find_html_comment_end(after_open: &str) -> Option<usize> {
    let mut s = 0u8;
    for (i, &b) in after_open.as_bytes().iter().enumerate() {
        s = match (s, b) {
            (0 | 1 | 4 | 5, b'>') => return Some(i + 1),
            (0, b'-') => 1,
            (1 | 3 | 4, b'-') => 4,
            (2 | 5, b'-') => 3,
            (4, b'!') => 5,
            _ => 2,
        };
    }
    None
}

#[must_use]
pub(crate) fn is_html_tag_start(after_lt: &str) -> bool {
    matches!(after_lt.as_bytes().first(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'/' | b'!' | b'?'))
}

#[must_use]
pub(crate) fn find_html_raw_close_tag_start(html: &str, body_start: usize, tag_lower: &str) -> Option<usize> {
    let (b, t) = (html.as_bytes(), tag_lower.as_bytes());
    (body_start..=b.len().saturating_sub(2 + t.len())).find(|&i| {
        b[i] == b'<'
            && b[i + 1] == b'/'
            && b[i + 2..i + 2 + t.len()].eq_ignore_ascii_case(t)
            && matches!(b.get(i + 2 + t.len()), Some(b'>' | b'/' | b' ' | b'\t' | b'\r' | b'\n' | b'\x0c'))
    })
}

pub(crate) fn skip_html_raw_or_rcdata_element(html: &str, body_start: usize, tag_lower: &str) -> Result<(usize, usize), String> {
    let body_end = find_html_raw_close_tag_start(html, body_start, tag_lower).ok_or_else(|| format!("unclosed `<{tag_lower}>` element"))?;
    let close_gt = html.get(body_end + 1..).and_then(find_html_tag_close).ok_or_else(|| format!("unclosed `</{tag_lower}>` closing tag"))?;
    Ok((body_end, body_end + close_gt + 2))
}

pub(crate) fn find_html_tag_close(rest: &str) -> Option<usize> {
    let bytes = rest.as_bytes();
    if matches!(bytes.first(), Some(b'!' | b'?')) || (bytes.first() == Some(&b'/') && !bytes.get(1).is_some_and(u8::is_ascii_alphabetic)) {
        return rest.find('>');
    }
    let (mut s, mut i) = (0u8, usize::from(bytes.first() == Some(&b'/')));
    while i < bytes.len() {
        s = match (s, bytes[i]) {
            (5, b'"') | (6, b'\'') => 1,
            (5 | 6, _) => s,
            (_, b'>') => return Some(i),
            (4, b'"') => 5,
            (4, b'\'') => 6,
            (4, c) if c.is_ascii_whitespace() => 4,
            (4, _) => 7,
            (7, c) if c.is_ascii_whitespace() => 1,
            (7, _) => 7,
            (2 | 3, b'=') => 4,
            (0..=3, b'/') => 1,
            (2, c) if c.is_ascii_whitespace() => 3,
            (0, c) if c.is_ascii_whitespace() => 1,
            (1 | 3, c) if c.is_ascii_whitespace() => s,
            (1 | 3, _) => 2,
            _ => s,
        };
        i += 1;
    }
    None
}

#[must_use]
pub(crate) fn extract_html_tag_name(trimmed_tag: &str) -> &str {
    let body = trimmed_tag.strip_prefix('/').unwrap_or(trimmed_tag).trim_start();
    body.get(..body.bytes().position(|b| b.is_ascii_whitespace() || b == b'/').unwrap_or(body.len())).unwrap_or("")
}

pub(crate) fn parse_html_tag_attr_spans(tag_body: &str) -> Vec<HtmlAttr<'_>> {
    let (b, mut attrs) = (tag_body.as_bytes(), Vec::new());
    let skip = |i: &mut usize, pred: fn(&u8) -> bool| {
        while *i < b.len() && pred(&b[*i]) {
            *i += 1;
        }
    };
    let mut i = b.iter().position(|c| c.is_ascii_whitespace() || *c == b'/').unwrap_or(b.len());
    while i < b.len() {
        skip(&mut i, |c| c.is_ascii_whitespace() || *c == b'/');
        if i >= b.len() {
            break;
        }
        let ks = i;
        i += 1;
        skip(&mut i, |c| !c.is_ascii_whitespace() && !matches!(c, b'=' | b'/'));
        let key = tag_body.get(ks..i).unwrap_or("");
        skip(&mut i, u8::is_ascii_whitespace);
        if b.get(i) != Some(&b'=') {
            if !key.is_empty() {
                attrs.push(HtmlAttr { key, value: String::new(), value_span: None });
            }
            continue;
        }
        i += 1;
        skip(&mut i, u8::is_ascii_whitespace);
        let (ts, te, vs, ve) = if matches!(b.get(i), Some(b'"' | b'\'')) {
            let (q, ts, vs) = (b[i], i, i + 1);
            i = vs;
            while i < b.len() && b[i] != q {
                i += 1;
            }
            let ve = i;
            i = (i + 1).min(b.len());
            (ts, i, vs, ve)
        } else {
            let s = i;
            skip(&mut i, |c| !c.is_ascii_whitespace());
            (s, i, s, i)
        };
        attrs.push(HtmlAttr { key, value: unescape_html_attr(tag_body.get(vs..ve).unwrap_or("")), value_span: Some((ts, te)) });
    }
    attrs
}

pub(crate) fn parse_html_tag_attributes(trimmed_tag: &str) -> Vec<(String, String)> {
    parse_html_tag_attr_spans(trimmed_tag).into_iter().map(|a| (a.key.to_owned(), a.value)).collect()
}

#[must_use]
pub(crate) fn parse_meta_refresh_url(content: &str) -> Option<&str> {
    let rest = content.trim_ascii_start();
    let digits = rest.bytes().take_while(|b| b.is_ascii_digit() || *b == b'.').count();
    let after_num = (digits > 0).then(|| rest.get(digits..))??;
    let trimmed = after_num.trim_ascii_start();
    let after_sep = trimmed.strip_prefix([';', ',']).map(str::trim_ascii_start).or_else(|| (trimmed.len() < after_num.len()).then_some(trimmed))?;
    let url = after_sep
        .get(..3)
        .filter(|s| s.eq_ignore_ascii_case("url"))
        .and_then(|_| after_sep.get(3..)?.trim_ascii_start().strip_prefix('='))
        .map_or(after_sep, str::trim_ascii_start);
    let target = url
        .strip_prefix('"')
        .map(|s| s.split('"').next().unwrap_or(s))
        .or_else(|| url.strip_prefix('\'').map(|s| s.split('\'').next().unwrap_or(s)))
        .unwrap_or_else(|| url.trim_ascii_end());
    (!target.is_empty()).then_some(target)
}

pub(crate) fn walk_html_open_tags(html: &str, mut on_tag: impl FnMut(&str, &str) -> Result<(), String>) -> Result<(), String> {
    let mut rest = html;
    while let Some((_, after_lt)) = rest.split_once('<') {
        if !is_html_tag_start(after_lt) {
            rest = after_lt;
            continue;
        }
        if let Some(after_c) = after_lt.strip_prefix("!--") {
            rest = find_html_comment_end(after_c).and_then(|e| after_c.get(e..)).ok_or_else(|| String::from("unclosed HTML comment `<!--`"))?;
            continue;
        }
        let (tag_body, tail) = find_html_tag_close(after_lt)
            .and_then(|e| after_lt.split_at_checked(e))
            .ok_or_else(|| String::from("unclosed HTML tag or attribute quote"))?;
        rest = tail.strip_prefix('>').unwrap_or(tail);
        let trimmed = tag_body.trim();
        if trimmed.starts_with('/') {
            continue;
        }
        let tag_lower = extract_html_tag_name(trimmed).to_ascii_lowercase();
        on_tag(&tag_lower, trimmed)?;
        if matches!(tag_lower.as_str(), "title" | "textarea" | "script" | "style") {
            let (_, after_close) = skip_html_raw_or_rcdata_element(rest, 0, &tag_lower)?;
            rest = rest.get(after_close..).unwrap_or("");
        }
    }
    Ok(())
}

// ---------- Compact TOML entry scanner (Cargo.toml / deny.toml) ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TomlVal {
    Str(String),
    Array(Vec<TomlVal>),
    InlineTable(Vec<(Vec<String>, TomlVal)>),
    Other(String),
}

impl TomlVal {
    pub(crate) fn as_str(&self) -> Option<&str> {
        if let Self::Str(s) = self { Some(s) } else { None }
    }
    pub(crate) fn collect_strings<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Self::Str(s) => out.push(s),
            Self::Array(items) => items.iter().for_each(|v| v.collect_strings(out)),
            Self::InlineTable(pairs) => pairs.iter().for_each(|(_, v)| v.collect_strings(out)),
            Self::Other(_) => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TomlEntry {
    Section { line: usize, path: Vec<String> },
    Leaf { line: usize, path: Vec<String>, value: TomlVal },
}

pub(crate) fn parse_toml_entries(src: &str) -> Vec<TomlEntry> {
    let (mut p, mut out, mut cur_sec) = (TomlParser { src, bytes: src.as_bytes(), pos: 0, line: 1 }, Vec::new(), Vec::new());
    while {
        p.skip_ws(true);
        p.pos < p.bytes.len()
    } {
        let line = p.line;
        if p.eat(b'[') {
            let arr = p.eat(b'[');
            let path = p.read_dotted_key();
            p.skip_ws(false);
            if p.eat(b']') {
                if arr {
                    p.eat(b']');
                }
            } else {
                p.skip_to_eol();
            }
            cur_sec = path.clone();
            if !path.is_empty() {
                out.push(TomlEntry::Section { line, path });
            }
        } else {
            let key_segs = p.read_dotted_key();
            p.skip_ws(false);
            if !key_segs.is_empty() && p.eat(b'=') {
                let mut full = cur_sec.clone();
                full.extend(key_segs);
                flatten_toml_leaf(line, &full, p.read_value(0), &mut out);
            } else {
                p.skip_to_eol();
            }
        }
    }
    out
}

fn flatten_toml_leaf(line: usize, prefix: &[String], val: TomlVal, out: &mut Vec<TomlEntry>) {
    match val {
        TomlVal::InlineTable(pairs) if !pairs.is_empty() => {
            for (sub, v) in pairs {
                let mut next = prefix.to_vec();
                next.extend(sub);
                flatten_toml_leaf(line, &next, v, out);
            }
        }
        TomlVal::InlineTable(_) => out.push(TomlEntry::Leaf { line, path: prefix.to_vec(), value: TomlVal::Other(String::new()) }),
        other => out.push(TomlEntry::Leaf { line, path: prefix.to_vec(), value: other }),
    }
}

struct TomlParser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: usize,
}

impl TomlParser<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }
    fn advance(&mut self) -> Option<u8> {
        let b = *self.bytes.get(self.pos)?;
        (self.pos, self.line) = (self.pos + 1, self.line + usize::from(b == b'\n'));
        Some(b)
    }
    fn eat(&mut self, b: u8) -> bool {
        (self.peek() == Some(b)).then(|| self.advance()).is_some()
    }
    fn skip_to_eol(&mut self) {
        while self.advance().is_some_and(|b| b != b'\n') {}
    }
    fn skip_ws(&mut self, nl: bool) {
        while let Some(b) = self.peek() {
            if matches!(b, b' ' | b'\t') || (nl && matches!(b, b'\r' | b'\n')) {
                self.advance();
            } else if b == b'#' {
                while self.peek().is_some_and(|c| c != b'\n') {
                    self.advance();
                }
                if !nl {
                    break;
                }
            } else {
                break;
            }
        }
    }
    fn read_dotted_key(&mut self) -> Vec<String> {
        let mut segs = Vec::new();
        loop {
            self.skip_ws(false);
            let seg = if matches!(self.peek(), Some(b'"' | b'\'')) {
                self.read_string()
            } else {
                let s = self.pos;
                while self.peek().is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')) {
                    self.pos += 1;
                }
                (self.pos > s).then(|| self.src.get(s..self.pos).unwrap_or("").to_owned())
            };
            let Some(seg) = seg else { break };
            segs.push(seg);
            self.skip_ws(false);
            if !self.eat(b'.') {
                break;
            }
        }
        segs
    }
    fn read_string(&mut self) -> Option<String> {
        let q = self.peek().filter(|b| matches!(b, b'"' | b'\''))?;
        self.advance();
        let mut out = String::new();
        while let Some(b) = self.peek() {
            if b == q || b == b'\n' {
                self.eat(q);
                break;
            }
            if q == b'"' && self.eat(b'\\') {
                let Some(esc) = self.advance() else { break };
                match esc {
                    b'b' => out.push('\u{0008}'),
                    b't' => out.push('\t'),
                    b'n' => out.push('\n'),
                    b'f' => out.push('\u{000C}'),
                    b'r' => out.push('\r'),
                    b'u' | b'U' => {
                        let n = if esc == b'u' { 4 } else { 8 };
                        if let Some(c) = self.src.get(self.pos..self.pos + n).and_then(|h| u32::from_str_radix(h, 16).ok()).and_then(char::from_u32) {
                            out.push(c);
                            self.pos += n;
                        }
                    }
                    other => out.push(char::from(other)),
                }
            } else if let Some(ch) = self.src.get(self.pos..).and_then(|s| s.chars().next()) {
                out.push(ch);
                self.pos += ch.len_utf8();
            } else {
                self.advance();
            }
        }
        Some(out)
    }
    fn read_value(&mut self, depth: usize) -> TomlVal {
        self.skip_ws(false);
        if depth < 32 {
            match self.peek() {
                Some(b'"' | b'\'') => return TomlVal::Str(self.read_string().unwrap_or_default()),
                Some(b'[') => {
                    self.advance();
                    let mut items = Vec::new();
                    while {
                        self.skip_ws(true);
                        self.pos < self.bytes.len() && self.peek() != Some(b']')
                    } {
                        let before = self.pos;
                        items.push(self.read_value(depth + 1));
                        self.skip_ws(true);
                        if !self.eat(b',') && self.pos == before {
                            self.advance();
                        }
                    }
                    self.eat(b']');
                    return TomlVal::Array(items);
                }
                Some(b'{') => {
                    self.advance();
                    let mut pairs = Vec::new();
                    while {
                        self.skip_ws(true);
                        self.pos < self.bytes.len() && self.peek() != Some(b'}')
                    } {
                        let before = self.pos;
                        let keys = self.read_dotted_key();
                        self.skip_ws(true);
                        if !keys.is_empty() && self.eat(b'=') {
                            self.skip_ws(true);
                            pairs.push((keys, self.read_value(depth + 1)));
                        }
                        self.skip_ws(true);
                        if !self.eat(b',') && self.pos == before {
                            self.advance();
                        }
                    }
                    self.eat(b'}');
                    return TomlVal::InlineTable(pairs);
                }
                _ => {}
            }
        }
        let s = self.pos;
        while self.peek().is_some_and(|b| !matches!(b, b',' | b']' | b'}' | b'#' | b'\n' | b'\r')) {
            self.pos += 1;
        }
        TomlVal::Other(self.src.get(s..self.pos).unwrap_or("").trim().to_owned())
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

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
            for args in [&["init", "-q"][..], &["config", "user.name", "Test"], &["config", "user.email", "test@example.com"]] {
                dir.run_git(args);
            }
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

    pub(crate) fn to_utf16_bytes(text: &str, little_endian: bool, include_bom: bool) -> Vec<u8> {
        let mut out = Vec::with_capacity(text.len() * 2 + 2);
        if include_bom {
            out.extend_from_slice(if little_endian { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
        }
        for u in text.encode_utf16() {
            out.extend_from_slice(&if little_endian { u.to_le_bytes() } else { u.to_be_bytes() });
        }
        out
    }
}
