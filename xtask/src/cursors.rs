//! `cargo xtask cursors`: verify bidirectional 1-to-1 correspondence between `Cursor` enum
//! variants in `crates/tools/src/lib.rs` and `[[cursor]]` entries in `crates/tools/src/cursors.toml`.

use std::collections::BTreeSet;
use std::path::Path;

const TOOLS_LIB: &str = "crates/tools/src/lib.rs";
const MANIFEST: &str = "crates/tools/src/cursors.toml";

pub fn run(root: &Path) -> Result<(), String> {
    let lib = std::fs::read_to_string(root.join(TOOLS_LIB)).map_err(|e| format!("{TOOLS_LIB}: {e}"))?;
    let toml = std::fs::read_to_string(root.join(MANIFEST)).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let count = check_sources(&lib, &toml)?;
    println!("cursors: all {count} Cursor variants verified 1-to-1 with {MANIFEST}");
    Ok(())
}

pub fn check_sources(lib_src: &str, toml_src: &str) -> Result<usize, String> {
    let variants = parse_cursor_variants(lib_src)?;
    let documented = parse_manifest_names(toml_src);

    let variant_set: BTreeSet<&str> = variants.iter().map(String::as_str).collect();
    let mut doc_set: BTreeSet<&str> = BTreeSet::new();
    let mut duplicates = Vec::new();
    for name in &documented {
        if !doc_set.insert(name.as_str()) {
            duplicates.push(name.as_str());
        }
    }

    let missing: Vec<&str> = variants.iter().map(String::as_str).filter(|v| !doc_set.contains(v)).collect();
    let stale: Vec<&str> = documented.iter().map(String::as_str).filter(|d| !variant_set.contains(d)).collect();

    let mut problems = Vec::new();
    if !missing.is_empty() {
        problems.push(format!("{} Cursor variant(s) missing from {MANIFEST}: {}", missing.len(), missing.join(", ")));
    }
    if !stale.is_empty() {
        problems.push(format!("{} stale {MANIFEST} entry(s) not in `Cursor`: {}", stale.len(), stale.join(", ")));
    }
    if !duplicates.is_empty() {
        problems.push(format!("{} duplicate {MANIFEST} entry(s): {}", duplicates.len(), duplicates.join(", ")));
    }

    if problems.is_empty() { Ok(variants.len()) } else { Err(problems.join("; ")) }
}

const fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

const fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

struct Scanner<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Scanner<'a> {
    fn new(src: &'a str) -> Self {
        Self { src, bytes: src.as_bytes(), pos: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn prev_is_ident(&self) -> bool {
        self.pos > 0 && is_ident_continue(self.bytes[self.pos - 1])
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            let start = self.pos;
            self.skip_ws();
            let _ = self.try_skip_comment();
            if self.pos == start {
                break;
            }
        }
    }

    fn try_skip_comment(&mut self) -> bool {
        if self.bytes.get(self.pos..self.pos + 2) == Some(b"//") {
            self.pos += 2;
            while let Some(b) = self.peek() {
                self.pos += 1;
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
                    self.pos += 1;
                }
            }
            return true;
        }
        false
    }

    fn try_skip_raw_string(&mut self) -> bool {
        if self.prev_is_ident() {
            return false;
        }
        let r_pos = if self.peek() == Some(b'r') {
            self.pos
        } else if matches!(self.peek(), Some(b'b' | b'c')) && self.bytes.get(self.pos + 1) == Some(&b'r') {
            self.pos + 1
        } else {
            return false;
        };
        let mut hashes = 0usize;
        while self.bytes.get(r_pos + 1 + hashes) == Some(&b'#') {
            hashes += 1;
        }
        let open_quote = r_pos + 1 + hashes;
        if self.bytes.get(open_quote) != Some(&b'"') {
            return false;
        }
        self.pos = open_quote + 1;
        while self.pos < self.bytes.len() {
            if self.bytes[self.pos] == b'"' && self.bytes.get(self.pos + 1..self.pos + 1 + hashes).is_some_and(|s| s.iter().all(|&b| b == b'#')) {
                self.pos += 1 + hashes;
                return true;
            }
            self.pos += 1;
        }
        true
    }

    fn try_skip_quoted_string(&mut self) -> bool {
        if self.peek() == Some(b'"') {
            self.pos += 1;
        } else if matches!(self.peek(), Some(b'b' | b'c')) && self.bytes.get(self.pos + 1) == Some(&b'"') && !self.prev_is_ident() {
            self.pos += 2;
        } else {
            return false;
        }
        while let Some(b) = self.peek() {
            self.pos += 1;
            if b == b'\\' {
                if self.pos < self.bytes.len() {
                    self.pos += 1;
                }
            } else if b == b'"' {
                break;
            }
        }
        true
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

    fn try_skip_non_code(&mut self) -> bool {
        self.try_skip_comment() || self.try_skip_raw_string() || self.try_skip_quoted_string() || self.try_skip_char_literal()
    }

    fn try_consume_keyword(&mut self, kw: &[u8]) -> bool {
        let end = self.pos + kw.len();
        if self.bytes.get(self.pos..end) == Some(kw) && !self.bytes.get(end).copied().is_some_and(is_ident_continue) {
            self.pos = end;
            true
        } else {
            false
        }
    }

    fn read_ident(&mut self) -> Option<String> {
        let start = self.pos;
        let first = self.peek()?;
        if !is_ident_start(first) {
            return None;
        }
        self.pos += 1;
        while let Some(b) = self.peek() {
            if is_ident_continue(b) {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.src.get(start..self.pos).map(str::to_owned)
    }

    fn skip_attribute(&mut self) -> Result<(), String> {
        // Consume `#` and `[` with balanced brackets.
        self.pos += 1;
        self.skip_ws_and_comments();
        if self.peek() != Some(b'[') {
            return Ok(());
        }
        self.pos += 1;
        let mut depth = 1usize;
        while self.pos < self.bytes.len() && depth > 0 {
            if self.try_skip_non_code() {
                continue;
            }
            match self.bytes[self.pos] {
                b'[' => depth += 1,
                b']' => depth -= 1,
                _ => {}
            }
            self.pos += 1;
        }
        if depth != 0 { Err(format!("{TOOLS_LIB}: unclosed attribute in `pub enum Cursor`")) } else { Ok(()) }
    }
}

fn find_cursor_enum_open_brace(src: &str) -> Result<usize, String> {
    let mut s = Scanner::new(src);
    while s.pos < s.bytes.len() {
        if s.try_skip_non_code() {
            continue;
        }
        if !s.prev_is_ident() && s.bytes[s.pos] == b'p' {
            let saved = s.pos;
            if s.try_consume_keyword(b"pub") {
                let after_pub = s.pos;
                s.skip_ws_and_comments();
                if s.pos > after_pub && s.try_consume_keyword(b"enum") {
                    let after_enum = s.pos;
                    s.skip_ws_and_comments();
                    if s.pos > after_enum && s.try_consume_keyword(b"Cursor") {
                        s.skip_ws_and_comments();
                        if s.peek() == Some(b'{') {
                            return Ok(s.pos + 1);
                        }
                        return Err(format!("{TOOLS_LIB}: unclosed `pub enum Cursor` (expected `{{`)"));
                    }
                }
            }
            s.pos = saved + 1;
            continue;
        }
        s.pos += 1;
    }
    Err(format!("{TOOLS_LIB}: `pub enum Cursor {{` not found"))
}

fn parse_cursor_variants(src: &str) -> Result<Vec<String>, String> {
    let body_start = find_cursor_enum_open_brace(src)?;
    let mut s = Scanner::new(src);
    s.pos = body_start;

    let mut brace_depth = 1usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut expecting_variant = true;
    let mut out = Vec::new();

    while s.pos < s.bytes.len() && brace_depth > 0 {
        if brace_depth == 1 && paren_depth == 0 && bracket_depth == 0 && expecting_variant {
            s.skip_ws_and_comments();
            if s.peek() == Some(b'#') {
                s.skip_attribute()?;
                continue;
            }
            if s.peek() == Some(b'}') {
                brace_depth = 0;
                break;
            }
            if let Some(ident) = s.read_ident() {
                out.push(ident);
                expecting_variant = false;
                continue;
            }
            if s.pos >= s.bytes.len() {
                break;
            }
        }

        if s.try_skip_non_code() {
            continue;
        }
        match s.bytes[s.pos] {
            b'{' => brace_depth += 1,
            b'}' => {
                brace_depth -= 1;
                if brace_depth == 0 {
                    break;
                }
            }
            b'(' => paren_depth += 1,
            b')' => paren_depth = paren_depth.saturating_sub(1),
            b'[' => bracket_depth += 1,
            b']' => bracket_depth = bracket_depth.saturating_sub(1),
            b',' if brace_depth == 1 && paren_depth == 0 && bracket_depth == 0 => {
                expecting_variant = true;
            }
            _ => {}
        }
        s.pos += 1;
    }

    if brace_depth != 0 {
        return Err(format!("{TOOLS_LIB}: missing `}}` on `pub enum Cursor`"));
    }
    Ok(out)
}

fn parse_manifest_names(toml: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in toml.lines() {
        let stripped = strip_toml_comment(line).trim();
        if let Some((key, val)) = stripped.split_once('=')
            && key.trim() == "name"
        {
            let name = val.trim().trim_matches('"');
            if !name.is_empty() {
                out.push(name.to_owned());
            }
        }
    }
    out
}

fn strip_toml_comment(line: &str) -> &str {
    let mut in_quote = false;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if in_quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_quote = false;
            }
        } else if ch == '"' {
            in_quote = true;
        } else if ch == '#' {
            return &line[..idx];
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::{check_sources, parse_cursor_variants, parse_manifest_names};

    #[test]
    fn extracts_variants_and_manifest_entries() {
        let src = "pub enum Cursor {\n    /// Doc\n    Select,\n    Crosshair,\n}\n";
        let toml = "[[cursor]]\nname = \"Select\"\n";
        assert_eq!(parse_cursor_variants(src).unwrap(), vec!["Select", "Crosshair"]);
        assert_eq!(parse_manifest_names(toml), vec!["Select"]);
    }

    #[test]
    fn anchors_on_exact_cursor_enum_and_handles_nested_braces_and_comments() {
        let src = "\
// pub enum Cursor { FakeComment }
pub enum CursorShape {
    Square,
}
pub enum Cursor {
    /// Doc comment mentioning {braces}
    #[default]
    Select,
    Resize {
        axis: Axis,
        bounds: (f64, f64),
    },
    Tuple(u32, u32),
    HandGrab = 4,
}
";
        assert_eq!(parse_cursor_variants(src).unwrap(), vec!["Select", "Resize", "Tuple", "HandGrab"]);
    }

    #[test]
    fn check_sources_rejects_missing_stale_and_duplicate_entries() {
        let src = "pub enum Cursor {\n    Select,\n    Lasso,\n}\n";
        let toml_missing = "[[cursor]]\nname = \"Select\"\n";
        let err_missing = check_sources(src, toml_missing).unwrap_err();
        assert!(err_missing.contains("missing") && err_missing.contains("Lasso"), "{err_missing}");

        let toml_stale = "[[cursor]]\nname = \"Select\"\n[[cursor]]\nname = \"Lasso\"\n[[cursor]]\nname = \"OldTool\"\n";
        let err_stale = check_sources(src, toml_stale).unwrap_err();
        assert!(err_stale.contains("stale") && err_stale.contains("OldTool"), "{err_stale}");

        let toml_dup = "[[cursor]]\nname = \"Select\"\n[[cursor]]\nname = \"Select\"\n[[cursor]]\nname = \"Lasso\"\n";
        let err_dup = check_sources(src, toml_dup).unwrap_err();
        assert!(err_dup.contains("duplicate") && err_dup.contains("Select"), "{err_dup}");

        let toml_ok = "[[cursor]]\nname = \"Select\"\nnamespace = \"ignored\"\n[[cursor]]\nname = \"Lasso\"\n";
        assert_eq!(check_sources(src, toml_ok).unwrap(), 2);
    }
}
