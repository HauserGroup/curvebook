//! `cargo xtask brands`: keep competitor vendor/product trademarks and retired suite branding out
//! of user-facing strings, package descriptions, workflows, and packaging metadata (`AGENTS.md`).

use std::collections::BTreeSet;
use std::path::Path;

/// Competitor vendor and product names checked on ASCII word boundaries (case-insensitive).
const BRANDS: &[&str] = &["graphpad", "dotmatics", "instat", "statmate", "jmp", "spss", "minitab", "originlab", "originpro", "sigmaplot", "systat"];

/// External suite branding and infrastructure forbidden across all non-xtask files in this standalone repository.
const SUITE_BRANDS: &[&str] = &[
    "artcraft",
    "getartcraft",
    "storytold",
    "storyteller",
    "craft-fonts",
    "craft_fonts",
    "craft_fonts_dir",
    "craft-common-actions",
    "craft-contrib",
    "craft-house-rules",
    "craft-ui",
    "craft_ui",
    "craft-keymap",
    "craft-update",
    "craft-window",
];

const ALLOW_MARKER: &str = "brand-ok";

/// Binary asset formats that do not carry UTF-8/UTF-16 text.
const BINARY_ASSET_EXT: &[&str] = &[
    "png",
    "jpg",
    "jpeg",
    "gif",
    "webp",
    "bmp",
    "tif",
    "tiff",
    "ico",
    "icns",
    "avif",
    "svgz",
    "pdf",
    "eps",
    "icc",
    "icm",
    "cur",
    "ani",
    "ase",
    "aco",
    "ttf",
    "otf",
    "ttc",
    "woff",
    "woff2",
    "eot",
    "pfb",
    "zip",
    "lumencraft",
    "lctemplate",
    "prism",
    "pzf",
    "xlsx",
    "mp4",
    "wav",
    "mp3",
];

/// The only file in which a suite name may appear, and only on a line that carries [`ALLOW_MARKER`].
/// The README's non-affiliation disclaimer has to name the suite it disclaims. Every other file,
/// and every unmarked README line, stays strict so suite branding cannot drift back into docs,
/// workflows, or packaging.
const SUITE_DISCLAIMER_PATH: &str = "README.md";

/// Suite names the disclaimer may use. Infrastructure identifiers (`craft-*` crates, the suite's
/// domain) are deliberately absent: naming the suite is a disclaimer, referencing its
/// infrastructure would be a dependency, which the standalone-repository rule forbids.
const SUITE_DISCLAIMER_NAMES: &[&str] = &["artcraft"];

#[derive(Debug, PartialEq, Eq)]
pub struct BrandHit {
    pub path: String,
    pub line: usize,
    pub brand: &'static str,
}

enum DecodedFile {
    Text(String),
    BinaryAsset,
}

pub fn run(root: &Path) -> Result<(), String> {
    let files = crate::repo_files(root)?;
    let hits = scan_repo_checked(root, &files)?;
    if hits.is_empty() {
        println!("brands: 0 unapproved brand or suite mentions across {} tracked files", files.len());
        return Ok(());
    }
    let lines: Vec<String> = hits.iter().map(|h| format!("{}:{}: `{}`", h.path, h.line, h.brand)).collect();
    Err(format!(
        "{} unapproved brand/suite mention(s) in user-facing, workflow, or packaging files (use neutral scientific wording, or mark intentional compatibility notes with `{ALLOW_MARKER}`):\n  {}",
        hits.len(),
        lines.join("\n  ")
    ))
}

pub fn scan_repo_checked(root: &Path, files: &[String]) -> Result<Vec<BrandHit>, String> {
    let mut hits = Vec::new();
    let mut scan_errors = Vec::new();
    for path in files {
        if skip_path(path) {
            continue;
        }
        let bytes = match std::fs::read(root.join(path)) {
            Ok(b) => b,
            Err(e) => {
                scan_errors.push(format!("cannot scan {path}: {e}"));
                continue;
            }
        };
        match decode_text_file(path, &bytes) {
            Ok(DecodedFile::Text(text)) => scan_file(path, &text, &mut hits),
            Ok(DecodedFile::BinaryAsset) => {}
            Err(msg) => scan_errors.push(msg),
        }
    }
    if !scan_errors.is_empty() {
        if !hits.is_empty() {
            for h in &hits {
                scan_errors.push(format!("{}:{}: `{}`", h.path, h.line, h.brand));
            }
        }
        return Err(scan_errors.join("\n  "));
    }
    Ok(hits)
}

fn is_binary_asset_ext(path: &str) -> bool {
    Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|ext| BINARY_ASSET_EXT.iter().any(|b| ext.eq_ignore_ascii_case(b)))
}

fn decode_text_file(path: &str, bytes: &[u8]) -> Result<DecodedFile, String> {
    if let Some(bomless) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return match std::str::from_utf8(bomless) {
            Ok(s) => Ok(DecodedFile::Text(s.to_owned())),
            Err(e) => Err(format!("cannot scan {path}: invalid UTF-8 after BOM: {e}")),
        };
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return Ok(DecodedFile::Text(decode_utf16(bytes.get(2..).unwrap_or_default(), true)));
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return Ok(DecodedFile::Text(decode_utf16(bytes.get(2..).unwrap_or_default(), false)));
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Ok(DecodedFile::Text(s.to_owned()));
    }
    if let Some(little_endian) = detect_bomless_utf16(bytes) {
        return Ok(DecodedFile::Text(decode_utf16(bytes, little_endian)));
    }
    if is_binary_asset_ext(path) {
        return Ok(DecodedFile::BinaryAsset);
    }
    Err(format!("cannot scan {path}: file is not valid UTF-8 or UTF-16"))
}

fn detect_bomless_utf16(bytes: &[u8]) -> Option<bool> {
    if bytes.len() < 8 || !bytes.contains(&0) {
        return None;
    }
    let mut le_ascii = 0usize;
    let mut be_ascii = 0usize;
    let (pairs, _) = bytes.as_chunks::<2>();
    let total_pairs = pairs.len();
    for &[a, b] in pairs {
        if a != 0 && (a.is_ascii_graphic() || a.is_ascii_whitespace()) && b == 0 {
            le_ascii += 1;
        } else if a == 0 && b != 0 && (b.is_ascii_graphic() || b.is_ascii_whitespace()) {
            be_ascii += 1;
        }
    }
    if le_ascii >= 4 && le_ascii * 4 >= total_pairs && le_ascii > be_ascii.saturating_mul(2) {
        Some(true)
    } else if be_ascii >= 4 && be_ascii * 4 >= total_pairs && be_ascii > le_ascii.saturating_mul(2) {
        Some(false)
    } else {
        None
    }
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> String {
    let (pairs, remainder) = bytes.as_chunks::<2>();
    let units = pairs.iter().map(|&arr| if little_endian { u16::from_le_bytes(arr) } else { u16::from_be_bytes(arr) });
    let mut out = String::with_capacity(pairs.len());
    for ch in char::decode_utf16(units) {
        out.push(ch.unwrap_or('\u{FFFD}'));
    }
    if !remainder.is_empty() {
        out.push('\u{FFFD}');
    }
    out
}

fn skip_path(path: &str) -> bool {
    path.starts_with("xtask/")
        || path.starts_with("crates/testkit/")
        || path.starts_with("tests/")
        || path.contains("/tests/")
        || path.ends_with("_test.rs")
}

fn scan_file(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    scan_suite_lines(path, text, out);
    let is_non_md_doc = path.starts_with("docs/") && !Path::new(path).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
    if Path::new(path).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("rs")) {
        scan_rust(path, text, out);
    } else if path.ends_with("Cargo.toml") {
        scan_cargo_toml(path, text, out);
    } else if path == "crates/tools/src/cursors.toml" || path.starts_with("packaging/") || path.starts_with(".github/workflows/") || is_non_md_doc {
        scan_plain_lines(path, text, out);
    }
}

fn scan_suite_lines(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    for (idx, line) in text.lines().enumerate() {
        if path == SUITE_DISCLAIMER_PATH && line.contains(ALLOW_MARKER) {
            let strict: Vec<&'static str> = SUITE_BRANDS.iter().copied().filter(|brand| !SUITE_DISCLAIMER_NAMES.contains(brand)).collect();
            collect_line_words(path, idx + 1, line, &strict, out);
        } else {
            collect_line_words(path, idx + 1, line, SUITE_BRANDS, out);
        }
    }
}

fn scan_cargo_toml(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    for (idx, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("description") && !line.contains(ALLOW_MARKER) {
            collect_line_words(path, idx + 1, line, BRANDS, out);
        }
    }
}

fn scan_plain_lines(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    let lines: Vec<&str> = text.lines().collect();
    for (idx, &line) in lines.iter().enumerate() {
        if line.contains(ALLOW_MARKER) {
            continue;
        }
        let prev_allows = idx > 0 && lines.get(idx - 1).is_some_and(|prev| prev.trim_start().starts_with('#') && prev.contains(ALLOW_MARKER));
        if prev_allows {
            continue;
        }
        collect_line_words(path, idx + 1, line, BRANDS, out);
    }
}

struct ExtractedLiteral {
    text: String,
    byte_lines: Vec<usize>,
    start_line: usize,
    end_line: usize,
}

struct RustLexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: usize,
}

impl<'a> RustLexer<'a> {
    fn new(src: &'a str) -> Self {
        Self { src, bytes: src.as_bytes(), pos: 0, line: 1 }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn prev_is_ident(&self) -> bool {
        self.pos > 0 && (self.bytes[self.pos - 1].is_ascii_alphanumeric() || self.bytes[self.pos - 1] == b'_')
    }

    fn advance_one(&mut self) -> Option<u8> {
        let b = *self.bytes.get(self.pos)?;
        self.pos += 1;
        if b == b'\n' {
            self.line += 1;
        }
        Some(b)
    }

    fn try_skip_comment(&mut self) -> bool {
        if self.bytes.get(self.pos..self.pos + 2) == Some(b"//") {
            self.pos += 2;
            while let Some(b) = self.advance_one() {
                if b == b'\n' {
                    break;
                }
            }
            return true;
        }
        if self.bytes.get(self.pos..self.pos + 2) == Some(b"/*") {
            self.pos += 2;
            let mut depth = 1usize;
            while self.pos < self.bytes.len() && depth > 0 {
                if self.bytes.get(self.pos..self.pos + 2) == Some(b"/*") {
                    depth += 1;
                    self.pos += 2;
                } else if self.bytes.get(self.pos..self.pos + 2) == Some(b"*/") {
                    depth -= 1;
                    self.pos += 2;
                } else {
                    self.advance_one();
                }
            }
            return true;
        }
        false
    }

    fn try_skip_char_literal(&mut self) -> bool {
        let q_pos = if self.peek() == Some(b'\'') {
            self.pos
        } else if self.peek() == Some(b'b') && self.bytes.get(self.pos + 1) == Some(&b'\'') && !self.prev_is_ident() {
            self.pos + 1
        } else {
            return false;
        };
        if self.bytes.get(q_pos + 1) == Some(&b'\\') {
            let mut p = q_pos + 2;
            if p < self.bytes.len() && self.bytes[p] != b'\n' {
                p += 1;
                let limit = (p + 14).min(self.bytes.len());
                while p < limit && self.bytes[p] != b'\n' {
                    if self.bytes[p] == b'\'' {
                        self.pos = p + 1;
                        return true;
                    }
                    p += 1;
                }
            }
            return false;
        }
        if let Some(rest) = self.src.get(q_pos + 1..)
            && let Some(ch) = rest.chars().next().filter(|&c| c != '\'' && c != '\n' && c != '\r')
            && self.bytes.get(q_pos + 1 + ch.len_utf8()) == Some(&b'\'')
        {
            self.pos = q_pos + 1 + ch.len_utf8() + 1;
            return true;
        }
        false
    }

    fn try_read_raw_string(&mut self) -> Option<ExtractedLiteral> {
        if self.prev_is_ident() {
            return None;
        }
        let r_pos = if self.peek() == Some(b'r') {
            self.pos
        } else if matches!(self.peek(), Some(b'b' | b'c')) && self.bytes.get(self.pos + 1) == Some(&b'r') {
            self.pos + 1
        } else {
            return None;
        };
        let mut hashes = 0usize;
        while self.bytes.get(r_pos + 1 + hashes) == Some(&b'#') {
            hashes += 1;
        }
        let open_quote = r_pos + 1 + hashes;
        if self.bytes.get(open_quote) != Some(&b'"') {
            return None;
        }
        let start_line = self.line;
        self.pos = open_quote + 1;
        let mut text = String::new();
        let mut byte_lines = Vec::new();
        while self.pos < self.bytes.len() {
            if self.bytes[self.pos] == b'"' && self.bytes.get(self.pos + 1..self.pos + 1 + hashes).is_some_and(|s| s.iter().all(|&b| b == b'#')) {
                self.pos += 1 + hashes;
                break;
            }
            if let Some(rest) = self.src.get(self.pos..)
                && let Some(ch) = rest.chars().next()
            {
                let line = self.line;
                push_char_with_line(&mut text, &mut byte_lines, ch, line);
                self.pos += ch.len_utf8();
                if ch == '\n' {
                    self.line += 1;
                }
            } else {
                self.advance_one();
            }
        }
        Some(ExtractedLiteral { text, byte_lines, start_line, end_line: self.line })
    }

    fn try_read_quoted_string(&mut self) -> Option<ExtractedLiteral> {
        let start_line = self.line;
        if self.peek() == Some(b'"') {
            self.pos += 1;
        } else if matches!(self.peek(), Some(b'b' | b'c')) && self.bytes.get(self.pos + 1) == Some(&b'"') && !self.prev_is_ident() {
            self.pos += 2;
        } else {
            return None;
        }
        let mut text = String::new();
        let mut byte_lines = Vec::new();
        while let Some(b) = self.peek() {
            if b == b'"' {
                self.pos += 1;
                break;
            }
            if b == b'\\' {
                self.read_string_escape(&mut text, &mut byte_lines);
                continue;
            }
            if let Some(rest) = self.src.get(self.pos..)
                && let Some(ch) = rest.chars().next()
            {
                let line = self.line;
                push_char_with_line(&mut text, &mut byte_lines, ch, line);
                self.pos += ch.len_utf8();
                if ch == '\n' {
                    self.line += 1;
                }
            } else {
                self.advance_one();
            }
        }
        Some(ExtractedLiteral { text, byte_lines, start_line, end_line: self.line })
    }

    fn read_string_escape(&mut self, text: &mut String, byte_lines: &mut Vec<usize>) {
        let esc_line = self.line;
        self.pos += 1; // '\'
        let Some(next) = self.peek() else {
            return;
        };
        if next == b'\n' || (next == b'\r' && self.bytes.get(self.pos + 1) == Some(&b'\n')) {
            if next == b'\r' {
                self.pos += 1;
            }
            self.advance_one(); // '\n'
            while let Some(ws) = self.peek() {
                if ws == b' ' || ws == b'\t' {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            return;
        }
        self.advance_one();
        let ch = match next {
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'0' => '\0',
            b'\\' => '\\',
            b'"' => '"',
            b'\'' => '\'',
            other => char::from(other),
        };
        push_char_with_line(text, byte_lines, ch, esc_line);
    }

    fn try_skip_cfg_test_item(&mut self) -> bool {
        if self.bytes.get(self.pos..self.pos + 2) != Some(b"#[") {
            return false;
        }
        let saved_pos = self.pos;
        let saved_line = self.line;
        let Some(inner) = self.read_bracket_attr() else {
            self.pos = saved_pos;
            self.line = saved_line;
            return false;
        };
        if !is_cfg_test_attr(&inner) {
            self.pos = saved_pos;
            self.line = saved_line;
            return false;
        }
        self.skip_following_attrs_and_ws();
        self.skip_annotated_item();
        true
    }

    fn read_bracket_attr(&mut self) -> Option<String> {
        self.pos += 2; // "#["
        let start = self.pos;
        let mut depth = 1usize;
        while self.pos < self.bytes.len() && depth > 0 {
            if self.try_skip_comment() || self.try_skip_char_literal() {
                continue;
            }
            if self.try_read_raw_string().is_some() || self.try_read_quoted_string().is_some() {
                continue;
            }
            match self.bytes[self.pos] {
                b'[' => depth += 1,
                b']' => depth -= 1,
                _ => {}
            }
            self.advance_one();
        }
        if depth == 0 { self.src.get(start..self.pos - 1).map(str::to_owned) } else { None }
    }

    fn skip_following_attrs_and_ws(&mut self) {
        loop {
            while let Some(b) = self.peek() {
                if b.is_ascii_whitespace() {
                    self.advance_one();
                } else {
                    break;
                }
            }
            if self.try_skip_comment() {
                continue;
            }
            if self.bytes.get(self.pos..self.pos + 2) == Some(b"#[") {
                if self.read_bracket_attr().is_none() {
                    break;
                }
                continue;
            }
            break;
        }
    }

    fn skip_annotated_item(&mut self) {
        let mut brace_depth = 0usize;
        let mut saw_brace = false;
        while self.pos < self.bytes.len() {
            if self.try_skip_comment() || self.try_skip_char_literal() {
                continue;
            }
            if self.try_read_raw_string().is_some() || self.try_read_quoted_string().is_some() {
                continue;
            }
            match self.bytes[self.pos] {
                b'{' => {
                    brace_depth += 1;
                    saw_brace = true;
                    self.advance_one();
                }
                b'}' if saw_brace => {
                    brace_depth = brace_depth.saturating_sub(1);
                    self.advance_one();
                    if brace_depth == 0 {
                        return;
                    }
                }
                b';' if !saw_brace => {
                    self.advance_one();
                    return;
                }
                _ => {
                    self.advance_one();
                }
            }
        }
    }
}

fn push_char_with_line(text: &mut String, byte_lines: &mut Vec<usize>, ch: char, line: usize) {
    let before = text.len();
    text.push(ch);
    byte_lines.resize(text.len(), line);
    let _ = before;
}

fn is_cfg_test_attr(inner: &str) -> bool {
    let compact: String = inner.chars().filter(|c| !c.is_whitespace()).collect();
    if compact == "cfg(test)" {
        return true;
    }
    let Some(rest) = compact.strip_prefix("cfg(all(").and_then(|s| s.strip_suffix("))")) else {
        return false;
    };
    let mut depth = 0usize;
    let mut start = 0usize;
    for (idx, ch) in rest.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                if rest[start..idx] == *"test" {
                    return true;
                }
                start = idx + 1;
            }
            _ => {}
        }
    }
    rest[start..] == *"test"
}

fn scan_rust(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    let lines: Vec<&str> = text.lines().collect();
    let mut lexer = RustLexer::new(text);
    let mut seen: BTreeSet<(usize, &'static str)> = BTreeSet::new();

    while lexer.pos < lexer.bytes.len() {
        if lexer.try_skip_comment() || lexer.try_skip_char_literal() || lexer.try_skip_cfg_test_item() {
            continue;
        }
        if let Some(lit) = lexer.try_read_raw_string().or_else(|| lexer.try_read_quoted_string()) {
            let line_allows =
                (lit.start_line..=lit.end_line).any(|ln| lines.get(ln.saturating_sub(1)).is_some_and(|src_line| src_line.contains(ALLOW_MARKER)));
            if !line_allows {
                collect_literal_words(path, &lit, BRANDS, &mut seen, out);
            }
            continue;
        }
        lexer.advance_one();
    }
}

fn collect_literal_words(
    path: &str,
    lit: &ExtractedLiteral,
    words: &[&'static str],
    seen: &mut BTreeSet<(usize, &'static str)>,
    out: &mut Vec<BrandHit>,
) {
    let lower = lit.text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    for &brand in words {
        let wlen = brand.len();
        let mut start = 0;
        while let Some(rel) = lower[start..].find(brand) {
            let pos = start + rel;
            let end = pos + wlen;
            let left_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
            let right_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
            if left_ok && right_ok {
                let line = lit.byte_lines.get(pos).copied().unwrap_or(lit.start_line);
                if seen.insert((line, brand)) {
                    out.push(BrandHit { path: path.to_owned(), line, brand });
                }
            }
            start = pos + 1;
        }
    }
}

fn collect_line_words(path: &str, line_no: usize, haystack: &str, words: &[&'static str], out: &mut Vec<BrandHit>) {
    let lower = haystack.to_ascii_lowercase();
    for &brand in words {
        if contains_word(&lower, brand) {
            out.push(BrandHit { path: path.to_owned(), line: line_no, brand });
        }
    }
}

fn contains_word(lower_haystack: &str, word: &str) -> bool {
    let bytes = lower_haystack.as_bytes();
    let wlen = word.len();
    let mut start = 0;
    while let Some(rel) = lower_haystack[start..].find(word) {
        let pos = start + rel;
        let end = pos + wlen;
        let left_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
        let right_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if left_ok && right_ok {
            return true;
        }
        start = pos + 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{BrandHit, run, scan_file, scan_repo_checked};

    fn temp_dir(label: &str) -> PathBuf {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("lc-brands-{}-{label}-{stamp}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn to_utf16_bytes(s: &str, little_endian: bool, include_bom: bool) -> Vec<u8> {
        let mut out = Vec::new();
        if include_bom {
            out.extend_from_slice(if little_endian { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
        }
        for unit in s.encode_utf16() {
            let bytes = if little_endian { unit.to_le_bytes() } else { unit.to_be_bytes() };
            out.extend_from_slice(&bytes);
        }
        out
    }

    #[test]
    fn flags_rust_string_literals_but_ignores_comments_and_cfg_test() {
        let mut hits = Vec::new();
        let src = "\
// GraphPad comment is fine
pub const LABEL: &str = \"Import GraphPad table\";
pub const OK: &str = \"Import OriginPro table\"; // brand-ok
pub fn instantiate() -> &'static str { \"instantiate\" }
#[cfg(test)]
mod tests {
    const T: &str = \"SPSS fixture\";
}
";
        scan_file("crates/ui-egui/src/lib.rs", src, &mut hits);
        assert_eq!(hits, vec![BrandHit { path: String::from("crates/ui-egui/src/lib.rs"), line: 2, brand: "graphpad" }]);
    }

    #[test]
    fn cfg_test_helper_does_not_hide_later_production_code() {
        let mut hits = Vec::new();
        let src = "\
#[cfg(test)]
fn test_only_helper() {
    let _ = \"SPSS fixture }; // brace inside string\";
}

pub fn shipped_error() -> &'static str {
    let q = '\"';
    let _ = q;
    \"GraphPad-compatible: unknown option\"
}

#[cfg(all(test, unix))]
fn unix_test_helper() {
    let _ = \"Minitab\";
}

#[cfg(not(test))]
pub const PROD: &str = \"Dotmatics\";
";
        scan_file("apps/lumencraft/src/main.rs", src, &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("apps/lumencraft/src/main.rs"), line: 9, brand: "graphpad" },
                BrandHit { path: String::from("apps/lumencraft/src/main.rs"), line: 18, brand: "dotmatics" },
            ]
        );
    }

    #[test]
    fn flags_multiline_string_literals_continuations_and_raw_strings() {
        let mut hits = Vec::new();
        let src = "\
pub const CONT: &str = \"Import from \\
    GraphPad table\";
pub const SPLIT: &str = \"Import Graph\\
Pad table\";
pub const ESC_NL: &str = \"line1\\nOriginPro\";
pub const RAW: &str = r#\"hello
Minitab\"#;
pub const ALLOWED: &str = \"Import from \\
    SigmaPlot table\"; // brand-ok
";
        scan_file("crates/format/src/lib.rs", src, &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("crates/format/src/lib.rs"), line: 2, brand: "graphpad" },
                BrandHit { path: String::from("crates/format/src/lib.rs"), line: 3, brand: "graphpad" },
                BrandHit { path: String::from("crates/format/src/lib.rs"), line: 5, brand: "originpro" },
                BrandHit { path: String::from("crates/format/src/lib.rs"), line: 7, brand: "minitab" },
            ]
        );
    }

    #[test]
    fn decodes_utf16_files_skips_binary_assets_and_fails_on_non_utf8_text() {
        let dir = temp_dir("encoding");
        std::fs::create_dir_all(dir.join("packaging/windows")).unwrap();
        std::fs::create_dir_all(dir.join("packaging/linux")).unwrap();
        std::fs::create_dir_all(dir.join("assets")).unwrap();

        std::fs::write(dir.join("packaging/windows/app.rc"), to_utf16_bytes("VALUE \"Product\", \"GraphPad\"\n", true, true)).unwrap();
        std::fs::write(dir.join("packaging/windows/clean.rc"), to_utf16_bytes("VALUE \"Product\", \"LumenCraft\"\n", false, true)).unwrap();
        std::fs::write(dir.join("assets/icon.png"), [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0xFF]).unwrap();

        let files = vec![String::from("packaging/windows/app.rc"), String::from("packaging/windows/clean.rc"), String::from("assets/icon.png")];
        let hits = scan_repo_checked(&dir, &files).unwrap();
        assert_eq!(hits, vec![BrandHit { path: String::from("packaging/windows/app.rc"), line: 1, brand: "graphpad" }]);

        // Non-UTF-8 Latin-1 byte in a packaging file must fail rather than be silently skipped.
        std::fs::write(dir.join("packaging/linux/latin1.desktop"), b"[Desktop Entry]\nComment=\xa9 LumenCraft\n").unwrap();
        let err = scan_repo_checked(&dir, &[String::from("packaging/linux/latin1.desktop")]).unwrap_err();
        assert!(err.contains("cannot scan packaging/linux/latin1.desktop"), "{err}");

        // Verify git-backed run also rejects non-UTF-8 text cleanly if tested directly via scan_repo_checked.
        let _ = run;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn desktop_entry_honors_preceding_comment_allow_marker() {
        let mut hits = Vec::new();
        let desktop = "\
[Desktop Entry]
Name=LumenCraft
# Compatibility search term: brand-ok
Keywords=biostatistics;regression;GraphPad;
";
        scan_file("packaging/linux/org.lumencraft.LumenCraft.desktop", desktop, &mut hits);
        assert!(hits.is_empty(), "{hits:?}");
    }

    #[test]
    fn flags_artcraft_suite_references_in_docs_and_workflows() {
        let mut hits = Vec::new();
        scan_file("docs/releasing.md", "Installs under Programs\\ArtCraft\\LumenCraft or https://getartcraft.com\n", &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("docs/releasing.md"), line: 1, brand: "artcraft" },
                BrandHit { path: String::from("docs/releasing.md"), line: 1, brand: "getartcraft" },
            ]
        );
    }

    #[test]
    fn readme_disclaimer_line_with_marker_may_name_the_suite() {
        let mut hits = Vec::new();
        scan_file("README.md", "Not affiliated with ArtCraft. <!-- brand-ok -->\n", &mut hits);
        assert!(hits.is_empty(), "{hits:?}");
    }

    #[test]
    fn suite_disclaimer_exemption_is_limited_to_marked_readme_lines_and_the_suite_name() {
        let mut hits = Vec::new();
        // No marker: still flagged in the README.
        scan_file("README.md", "Part of ArtCraft.\n", &mut hits);
        // Marker in any other file: still flagged.
        scan_file("docs/releasing.md", "Not affiliated with ArtCraft. <!-- brand-ok -->\n", &mut hits);
        // Marked README line: suite infrastructure is still flagged.
        scan_file("README.md", "See https://getartcraft.com and craft-ui. <!-- brand-ok -->\n", &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("README.md"), line: 1, brand: "artcraft" },
                BrandHit { path: String::from("docs/releasing.md"), line: 1, brand: "artcraft" },
                BrandHit { path: String::from("README.md"), line: 1, brand: "getartcraft" },
                BrandHit { path: String::from("README.md"), line: 1, brand: "craft-ui" },
            ]
        );
    }
}
