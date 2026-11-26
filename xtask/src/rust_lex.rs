//! Shared Rust source lexical utilities, `#[cfg(test)]` masking, and string/comment
//! tokenization used across `xtask` policy gates (`brands`, `layers`, `telemetry`,
//! `citations`, `status`, `design_tokens`, `stats`).

#[must_use]
pub(crate) const fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
#[must_use]
pub(crate) fn prev_is_ident(bytes: &[u8], i: usize) -> bool {
    i > 0 && is_ident_byte(bytes[i - 1])
}
#[must_use]
pub(crate) fn count_newlines(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&b| b == b'\n').count()
}
#[must_use]
pub(crate) fn skip_line_comment_end(bytes: &[u8], start: usize) -> usize {
    bytes[start.saturating_add(2)..].iter().position(|&b| b == b'\n').map_or(bytes.len(), |r| start + 2 + r)
}
#[must_use]
pub(crate) fn skip_block_comment_end(bytes: &[u8], start: usize) -> usize {
    let (mut depth, mut i) = (1usize, start.saturating_add(2));
    while i < bytes.len() && depth > 0 {
        match bytes.get(i..i + 2) {
            Some(b"/*") => (depth, i) = (depth + 1, i + 2),
            Some(b"*/") => (depth, i) = (depth - 1, i + 2),
            _ => i += 1,
        }
    }
    i
}
#[must_use]
pub(crate) fn skip_quoted_string_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start.saturating_add(1);
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i = i.saturating_add(2).min(bytes.len()),
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// Parse a raw string literal (`r"..."`, `r#"..."#`, `br"..."`, `cr#"..."#`) at `start`.
#[must_use]
pub(crate) fn find_raw_string_span(bytes: &[u8], start: usize) -> Option<(usize, usize, usize)> {
    if prev_is_ident(bytes, start) {
        return None;
    }
    let r_pos = match *bytes.get(start)? {
        b'r' => start,
        b'b' | b'c' if bytes.get(start + 1) == Some(&b'r') => start + 1,
        _ => return None,
    };
    let hashes = bytes.get(r_pos + 1..)?.iter().take_while(|&&b| b == b'#').count();
    let open_q = r_pos + 1 + hashes;
    (bytes.get(open_q) == Some(&b'"')).then(|| {
        let prefix_end = open_q + 1;
        let close = (prefix_end..bytes.len())
            .find(|&i| bytes[i] == b'"' && bytes.get(i + 1..i + 1 + hashes).is_some_and(|s| s.iter().all(|&b| b == b'#')))
            .map_or(bytes.len(), |i| i + 1 + hashes);
        (prefix_end, close, hashes)
    })
}

fn raw_string_body_end(bytes: &[u8], prefix_end: usize, close_end: usize, hashes: usize) -> usize {
    let cl = 1 + hashes;
    let has_close = close_end >= prefix_end + cl
        && bytes.get(close_end - cl) == Some(&b'"')
        && bytes.get(close_end - hashes..close_end).is_some_and(|s| s.iter().all(|&b| b == b'#'));
    if has_close { close_end - cl } else { close_end }
}

/// Parse a Rust character or byte-character literal (`'x'`, `b'x'`, `'\n'`, `'\u{1234}'`) at `start`.
#[must_use]
pub(crate) fn find_char_literal_span(src: &str, start: usize) -> Option<(usize, usize)> {
    let bytes = src.as_bytes();
    let (q_pos, is_byte) = match *bytes.get(start)? {
        b'b' if bytes.get(start + 1) == Some(&b'\'') && !prev_is_ident(bytes, start) => (start + 1, true),
        b'\'' => (start, false),
        _ => return None,
    };
    let end_q = if bytes.get(q_pos + 1) == Some(&b'\\') {
        let p = q_pos + 2;
        match *bytes.get(p)? {
            b'\'' | b'"' | b'\\' | b'n' | b'r' | b't' | b'0' => Some(p + 1),
            b'x' if bytes.get(p + 1..p + 3).is_some_and(|s| s.iter().all(u8::is_ascii_hexdigit)) => Some(p + 3),
            b'u' if !is_byte && bytes.get(p + 1) == Some(&b'{') => {
                let rel = bytes.get(p + 2..)?.iter().position(|&b| b == b'}')?;
                let inner = bytes.get(p + 2..p + 2 + rel)?;
                (inner.iter().any(u8::is_ascii_hexdigit) && inner.iter().all(|&b| b.is_ascii_hexdigit() || b == b'_')).then_some(p + 3 + rel)
            }
            _ => None,
        }
    } else {
        src.get(q_pos + 1..).and_then(|r| r.chars().next()).filter(|&c| !matches!(c, '\'' | '\n' | '\r')).map(|ch| q_pos + 1 + ch.len_utf8())
    }?;
    (bytes.get(end_q) == Some(&b'\'')).then_some((q_pos, end_q))
}

fn push_blanked(bytes: &[u8], dst: &mut Vec<u8>) {
    dst.extend(bytes.iter().map(|&b| if b == b'\n' { b'\n' } else { b' ' }));
}
fn bytes_to_string_lossy(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// Blank comments and string/character literals while preserving exact byte offsets and line numbers.
#[must_use]
pub(crate) fn blank_comments_and_literals_both(src: &str, keep_delims: bool) -> (String, String) {
    let bytes = src.as_bytes();
    let (mut prod, mut keep, mut i) = (Vec::with_capacity(bytes.len()), Vec::with_capacity(bytes.len()), 0usize);
    let emit = |prod: &mut Vec<u8>, keep: &mut Vec<u8>, os: usize, bs: usize, be: usize, ce: usize| {
        keep.extend_from_slice(&bytes[os..ce]);
        if keep_delims {
            prod.extend_from_slice(&bytes[os..bs]);
            push_blanked(&bytes[bs..be], prod);
            prod.extend_from_slice(&bytes[be..ce]);
        } else {
            push_blanked(&bytes[os..ce], prod);
        }
    };
    while i < bytes.len() {
        if matches!(bytes.get(i..i + 2), Some(b"//" | b"/*")) {
            let end = if bytes[i + 1] == b'/' {
                let e = skip_line_comment_end(bytes, i);
                e + usize::from(bytes.get(e) == Some(&b'\n'))
            } else {
                skip_block_comment_end(bytes, i)
            };
            push_blanked(&bytes[i..end], &mut prod);
            push_blanked(&bytes[i..end], &mut keep);
            i = end;
        } else if let Some((pe, ce, h)) = find_raw_string_span(bytes, i) {
            emit(&mut prod, &mut keep, i, pe, raw_string_body_end(bytes, pe, ce, h), ce);
            i = ce;
        } else if let Some((qp, eq)) = find_char_literal_span(src, i) {
            emit(&mut prod, &mut keep, i, qp + 1, eq, eq + 1);
            i = eq + 1;
        } else if bytes[i] == b'"' || (matches!(bytes[i], b'b' | b'c') && bytes.get(i + 1) == Some(&b'"') && !prev_is_ident(bytes, i)) {
            let q = i + usize::from(bytes[i] != b'"');
            let ce = skip_quoted_string_end(bytes, q);
            emit(&mut prod, &mut keep, i, q + 1, ce.saturating_sub(usize::from(ce >= q + 2 && bytes[ce - 1] == b'"')), ce);
            i = ce;
        } else {
            prod.push(bytes[i]);
            keep.push(bytes[i]);
            i += 1;
        }
    }
    (bytes_to_string_lossy(prod), bytes_to_string_lossy(keep))
}

#[must_use]
pub(crate) fn blank_comments_and_literals(src: &str) -> String {
    blank_comments_and_literals_both(src, true).0
}

pub(crate) struct StrippedRust {
    pub(crate) prod_code: String,
    pub(crate) no_comments_code: String,
}

#[must_use]
pub(crate) fn strip_non_prod_rust_both(src: &str) -> StrippedRust {
    let (all, comments_only) = blank_comments_and_literals_both(src, true);
    let (mut prod_b, mut lits_b) = (all.into_bytes(), comments_only.into_bytes());
    let snap = prod_b.clone();
    mask_cfg_test_items(&snap, &mut [&mut prod_b, &mut lits_b]);
    StrippedRust { prod_code: bytes_to_string_lossy(prod_b), no_comments_code: bytes_to_string_lossy(lits_b) }
}
#[must_use]
pub(crate) fn strip_non_prod_rust(src: &str) -> String {
    strip_cfg_test_items(&blank_comments_and_literals(src))
}
#[must_use]
pub(crate) fn strip_cfg_test_items(code: &str) -> String {
    let mut out = code.as_bytes().to_vec();
    mask_cfg_test_items(code.as_bytes(), &mut [&mut out]);
    bytes_to_string_lossy(out)
}

#[must_use]
pub(crate) fn find_fn_body_braces(bytes: &[u8], mut p: usize) -> Option<(usize, usize)> {
    let (mut paren, mut bracket, mut angle, mut saw_params) = (0usize, 0usize, 0usize, false);
    while p < bytes.len() {
        let flat = paren == 0 && bracket == 0 && angle == 0;
        match bytes[p] {
            b';' if flat => return None,
            b'(' => (saw_params, paren) = (saw_params | flat, paren + 1),
            b')' => paren = paren.saturating_sub(1),
            b'[' => bracket += 1,
            b']' => bracket = bracket.saturating_sub(1),
            b'<' if paren == 0 && bracket == 0 => angle += 1,
            b'>' if paren == 0 && bracket == 0 && angle > 0 && bytes.get(p.wrapping_sub(1)) != Some(&b'-') => angle -= 1,
            b'{' => {
                let close = find_matching_delim(bytes, p, b'{', b'}')?;
                if saw_params && flat {
                    return Some((p, close));
                }
                p = close + 1;
                continue;
            }
            _ => {}
        }
        p += 1;
    }
    None
}

pub(crate) fn find_matching_delim(bytes: &[u8], open: usize, open_b: u8, close_b: u8) -> Option<usize> {
    let mut depth = 0usize;
    for (p, &b) in bytes.iter().enumerate().skip(open) {
        if b == open_b {
            depth += 1;
        } else if b == close_b {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(p);
            }
        }
    }
    None
}

/// Returns `true` iff a Rust file under `src/` is loaded exclusively through a `#[cfg(test)]` `mod <name>;`.
pub(crate) fn is_cfg_test_only_submodule(root: &std::path::Path, rel: &str) -> bool {
    let norm = rel.replace('\\', "/");
    let Some((crate_prefix, inside_src)) = norm.split_once("/src/") else { return false };
    let (parts, src_root) = (inside_src.split('/').collect::<Vec<_>>(), format!("{crate_prefix}/src"));
    for depth in (0..parts.len()).rev() {
        let mod_name = if depth + 1 == parts.len() {
            let Some(stem) = parts[depth].strip_suffix(".rs") else { return false };
            if matches!(stem, "lib" | "main" | "mod") {
                continue;
            }
            stem
        } else {
            parts[depth]
        };
        let dir = if depth == 0 { src_root.clone() } else { format!("{src_root}/{}", parts[..depth].join("/")) };
        let parents = if depth == 0 { [format!("{dir}/lib.rs"), format!("{dir}/main.rs")] } else { [format!("{dir}/mod.rs"), format!("{dir}.rs")] };
        let (mut saw_test, mut saw_prod) = (false, false);
        for full in parents.into_iter().map(|c| root.join(c)).filter(|p| crate::scan_util::is_regular_file(p)) {
            let Ok(src) = std::fs::read_to_string(&full) else { return false };
            let blanked = blank_comments_and_literals(&src);
            if has_external_mod_decl(&blanked, mod_name) {
                let in_prod = has_external_mod_decl(&strip_cfg_test_items(&blanked), mod_name);
                (saw_prod, saw_test) = (saw_prod | in_prod, saw_test | !in_prod);
            }
        }
        if saw_test && !saw_prod {
            return true;
        }
    }
    false
}

fn has_external_mod_decl(code: &str, mod_name: &str) -> bool {
    let (bytes, target) = (code.as_bytes(), mod_name.as_bytes());
    code.match_indices("mod").any(|(i, _)| {
        !prev_is_ident(bytes, i) && (i < 2 || &bytes[i - 2..i] != b"r#") && bytes.get(i + 3).is_some_and(u8::is_ascii_whitespace) && {
            let mut p = skip_ws(bytes, i + 3);
            p += 2 * usize::from(bytes.get(p..p + 2) == Some(b"r#"));
            bytes.get(p..p + target.len()) == Some(target)
                && bytes.get(p + target.len()).is_none_or(|b| !is_ident_byte(*b))
                && bytes.get(skip_ws(bytes, p + target.len())) == Some(&b';')
        }
    })
}

pub(crate) fn mask_cfg_test_items(code_bytes: &[u8], targets: &mut [&mut Vec<u8>]) {
    let mut i = 0usize;
    while i < code_bytes.len() {
        if code_bytes.get(i..i + 2) == Some(b"#[")
            && let Some(close) = find_matching_delim(code_bytes, i + 1, b'[', b']')
            && std::str::from_utf8(&code_bytes[i + 2..close])
                .ok()
                .is_some_and(|s| is_cfg_test_only(&s.chars().filter(|c| !c.is_whitespace()).collect::<String>()))
        {
            let mut body_start = skip_ws(code_bytes, close + 1);
            while code_bytes.get(body_start..body_start + 2) == Some(b"#[")
                && let Some(c2) = find_matching_delim(code_bytes, body_start + 1, b'[', b']')
            {
                body_start = skip_ws(code_bytes, c2 + 1);
            }
            let item_end = find_item_end(code_bytes, body_start);
            for target in targets.iter_mut() {
                target.iter_mut().take(item_end).skip(i).filter(|b| **b != b'\n').for_each(|b| *b = b' ');
            }
            i = item_end;
            continue;
        }
        i += 1;
    }
}

#[must_use]
pub(crate) fn is_cfg_test_only(compact_attr: &str) -> bool {
    strip_cfg_call(compact_attr, "cfg").is_some_and(|pred| !eval_cfg_pred_in_prod(pred).0)
}

fn eval_cfg_pred_in_prod(pred: &str) -> (bool, bool) {
    if let Some(inner) = strip_cfg_call(pred, "not") {
        let (t, f) = eval_cfg_pred_in_prod(inner);
        (f, t)
    } else if let Some(inner) = strip_cfg_call(pred, "all") {
        split_cfg_args(inner).into_iter().map(eval_cfg_pred_in_prod).fold((true, false), |(at, af), (t, f)| (at && t, af || f))
    } else if let Some(inner) = strip_cfg_call(pred, "any") {
        split_cfg_args(inner).into_iter().map(eval_cfg_pred_in_prod).fold((false, true), |(at, af), (t, f)| (at || t, af && f))
    } else {
        (pred != "test", true)
    }
}

fn strip_cfg_call<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let rest = s.strip_prefix(name)?;
    let close = find_matching_delim(rest.as_bytes(), 0, b'(', b')')?;
    (rest.starts_with('(') && close + 1 == rest.len()).then(|| rest.get(1..close)).flatten()
}

fn split_cfg_args(s: &str) -> Vec<&str> {
    let (mut args, mut depth, mut in_q, mut start) = (Vec::new(), 0usize, false, 0usize);
    for (i, ch) in s.char_indices() {
        match ch {
            '"' => in_q = !in_q,
            '(' if !in_q => depth += 1,
            ')' if !in_q => depth = depth.saturating_sub(1),
            ',' if !in_q && depth == 0 => {
                args.extend(s.get(start..i).filter(|a| !a.is_empty()));
                start = i + 1;
            }
            _ => {}
        }
    }
    args.extend(s.get(start..).filter(|a| !a.is_empty()));
    args
}

fn skip_ws(bytes: &[u8], mut pos: usize) -> usize {
    while bytes.get(pos).is_some_and(u8::is_ascii_whitespace) {
        pos += 1;
    }
    pos
}

fn starts_with_item_keyword(bytes: &[u8], mut pos: usize) -> bool {
    loop {
        pos = skip_ws(bytes, pos);
        let start = pos;
        if !bytes.get(pos).is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_') {
            return false;
        }
        while bytes.get(pos).is_some_and(|b| is_ident_byte(*b)) {
            pos += 1;
        }
        match &bytes[start..pos] {
            b"pub" => {
                pos = skip_ws(bytes, pos);
                if bytes.get(pos) == Some(&b'(')
                    && let Some(c) = find_matching_delim(bytes, pos, b'(', b')')
                {
                    pos = c + 1;
                }
            }
            b"default" | b"async" | b"unsafe" => {}
            b"extern" | b"fn" | b"struct" | b"enum" | b"trait" | b"impl" | b"mod" | b"type" | b"static" | b"const" | b"use" => return true,
            b"macro_rules" => return bytes.get(skip_ws(bytes, pos)) == Some(&b'!'),
            _ => return false,
        }
    }
}

fn find_item_end(bytes: &[u8], mut pos: usize) -> usize {
    let (is_item_decl, mut brace, mut paren, mut bracket, mut saw_brace) = (starts_with_item_keyword(bytes, pos), 0usize, 0usize, 0usize, false);
    while pos < bytes.len() {
        let flat = brace == 0 && paren == 0 && bracket == 0;
        match bytes[pos] {
            b'{' => (saw_brace, brace) = (saw_brace | (paren == 0 && bracket == 0), brace + 1),
            b'}' if brace > 0 => {
                brace -= 1;
                if brace == 0 && paren == 0 && bracket == 0 && saw_brace {
                    return pos + 1;
                }
            }
            b'(' => paren += 1,
            b')' if paren > 0 => paren -= 1,
            b'[' => bracket += 1,
            b']' if bracket > 0 => bracket -= 1,
            b'}' | b')' | b']' if flat && !saw_brace => return pos,
            b';' if flat && !saw_brace => return pos + 1,
            b',' if !is_item_decl && flat && !saw_brace => return pos + 1,
            _ => {}
        }
        pos += 1;
    }
    bytes.len()
}

pub(crate) struct ExtractedLiteral {
    pub(crate) text: String,
    pub(crate) byte_lines: Vec<usize>,
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
    pub(crate) quote_start: usize,
    pub(crate) quote_end: usize,
}

pub(crate) struct RustLexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pub(crate) pos: usize,
    line: usize,
}

impl<'a> RustLexer<'a> {
    #[must_use]
    pub(crate) fn new(src: &'a str) -> Self {
        Self { src, bytes: src.as_bytes(), pos: 0, line: 1 }
    }
    #[must_use]
    pub(crate) const fn is_eof(&self) -> bool {
        self.pos >= self.bytes.len()
    }
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }
    pub(crate) fn advance_one(&mut self) -> Option<u8> {
        let b = *self.bytes.get(self.pos)?;
        (self.pos, self.line) = (self.pos + 1, self.line + usize::from(b == b'\n'));
        Some(b)
    }

    pub(crate) fn try_read_comment(&mut self) -> Option<(usize, &'a str)> {
        let (sl, bs) = (self.line, self.pos + 2);
        if self.bytes.get(self.pos..bs) == Some(b"//") {
            let be = skip_line_comment_end(self.bytes, self.pos);
            self.pos = be + usize::from(self.bytes.get(be) == Some(&b'\n'));
            self.line += usize::from(self.pos > be);
            return Some((sl, self.src.get(bs..be).unwrap_or("")));
        }
        if self.bytes.get(self.pos..bs) == Some(b"/*") {
            let end = skip_block_comment_end(self.bytes, self.pos);
            let be = if end >= bs + 2 && self.bytes.get(end - 2..end) == Some(b"*/") { end - 2 } else { end };
            (self.line, self.pos) = (self.line + count_newlines(&self.bytes[self.pos..end]), end);
            return Some((sl, self.src.get(bs..be).unwrap_or("")));
        }
        None
    }

    pub(crate) fn try_read_identifier(&mut self) -> Option<(usize, &'a str)> {
        if self.bytes.get(self.pos..self.pos + 2) == Some(b"r#")
            && self.bytes.get(self.pos + 2).is_some_and(|&b| b.is_ascii_alphabetic() || b == b'_')
            && !prev_is_ident(self.bytes, self.pos)
        {
            self.pos += 2;
        }
        if !self.peek().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') || prev_is_ident(self.bytes, self.pos) {
            return None;
        }
        let (start, line) = (self.pos, self.line);
        while self.peek().is_some_and(is_ident_byte) {
            self.pos += 1;
        }
        Some((line, self.src.get(start..self.pos).unwrap_or("")))
    }

    pub(crate) fn try_skip_char_literal(&mut self) -> bool {
        find_char_literal_span(self.src, self.pos).is_some_and(|(_, eq)| {
            self.pos = eq + 1;
            true
        })
    }

    pub(crate) fn try_read_raw_string(&mut self) -> Option<ExtractedLiteral> {
        let (pe, ce, hashes) = find_raw_string_span(self.bytes, self.pos)?;
        let (quote_start, start_line) = (self.pos, self.line);
        let body = self.src.get(pe..raw_string_body_end(self.bytes, pe, ce, hashes)).unwrap_or("");
        let (mut text, mut byte_lines) = (String::with_capacity(body.len()), Vec::with_capacity(body.len()));
        for ch in body.chars() {
            push_char_with_line(&mut text, &mut byte_lines, ch, self.line);
            self.line += usize::from(ch == '\n');
        }
        self.pos = ce;
        Some(ExtractedLiteral { text, byte_lines, start_line, end_line: self.line, quote_start, quote_end: ce })
    }

    pub(crate) fn try_read_quoted_string(&mut self) -> Option<ExtractedLiteral> {
        let (quote_start, start_line) = (self.pos, self.line);
        let prefix = if self.peek() == Some(b'"') {
            1
        } else if matches!(self.peek(), Some(b'b' | b'c')) && self.bytes.get(self.pos + 1) == Some(&b'"') && !prev_is_ident(self.bytes, self.pos) {
            2
        } else {
            return None;
        };
        self.pos += prefix;
        let (mut text, mut byte_lines) = (String::new(), Vec::new());
        while let Some(b) = self.peek() {
            if b == b'"' {
                self.pos += 1;
                break;
            }
            if b == b'\\' {
                self.read_string_escape(&mut text, &mut byte_lines);
            } else if let Some(ch) = self.src.get(self.pos..).and_then(|r| r.chars().next()) {
                push_char_with_line(&mut text, &mut byte_lines, ch, self.line);
                (self.pos, self.line) = (self.pos + ch.len_utf8(), self.line + usize::from(ch == '\n'));
            } else {
                self.advance_one();
            }
        }
        Some(ExtractedLiteral { text, byte_lines, start_line, end_line: self.line, quote_start, quote_end: self.pos })
    }

    fn read_string_escape(&mut self, text: &mut String, byte_lines: &mut Vec<usize>) {
        let esc_line = self.line;
        self.pos += 1;
        let Some(next) = self.peek() else { return };
        if next == b'\n' || (next == b'\r' && self.bytes.get(self.pos + 1) == Some(&b'\n')) {
            self.pos += usize::from(next == b'\r');
            self.advance_one();
            while self.peek().is_some_and(|w| matches!(w, b' ' | b'\t' | b'\r' | b'\n')) {
                self.advance_one();
            }
            return;
        }
        self.advance_one();
        let ch = match next {
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'0' => '\0',
            b'\\' | b'"' | b'\'' => char::from(next),
            b'x' => self.src.get(self.pos..self.pos + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).map_or('x', |v| {
                self.pos += 2;
                char::from(v)
            }),
            b'u' if self.peek() == Some(b'{') => {
                if let Some(close) = find_matching_delim(self.bytes, self.pos, b'{', b'}') {
                    let hex: String = self.src.get(self.pos + 1..close).unwrap_or("").chars().filter(|&c| c != '_').collect();
                    self.pos = close + 1;
                    if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        push_char_with_line(text, byte_lines, c, esc_line);
                    }
                }
                return;
            }
            other => char::from(other),
        };
        push_char_with_line(text, byte_lines, ch, esc_line);
    }
}

pub(crate) fn push_char_with_line(text: &mut String, byte_lines: &mut Vec<usize>, ch: char, line: usize) {
    text.push(ch);
    byte_lines.resize(text.len(), line);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan_util::test_support::TestDir;

    #[test]
    fn cfg_test_lexing_and_submodules() {
        for (a, exp) in [
            ("cfg(test)", true),
            ("cfg(all(unix,test))", true),
            ("cfg(not(not(test)))", true),
            ("cfg(any(unix,test))", false),
            ("cfg(not(test))", false),
        ] {
            assert_eq!(is_cfg_test_only(a), exp, "{a}");
        }
        let src = "/* a /* b */\n */\nconst S: &str = r#\"line1\nline2\"#;\n#[cfg(test)]\nfn t() -> [u8; 4] { secret(); [0; 4] }\n#[cfg(test)]\nmod m { fn x() {} }\npub fn live() {}\n";
        let ((all, keep), both) = (blank_comments_and_literals_both(src, true), strip_non_prod_rust_both(src));
        assert!(
            all.lines().count() == src.lines().count()
                && !all.contains("line1")
                && keep.contains("line1")
                && both.prod_code == strip_non_prod_rust(src)
                && !both.prod_code.contains("secret")
                && both.prod_code.contains("pub fn live()")
                && both.no_comments_code.contains("line1")
        );
        let mut lex = RustLexer::new("// c1\nlet c = ('\\'','\"');\nlet s = \"a\\n\\x41\\u{42}\\\n\n  C\";\nlet r = r#\"hi\"#;\n");
        let (mut cs, mut ls) = (Vec::new(), Vec::new());
        while !lex.is_eof() {
            if let Some(c) = lex.try_read_comment() {
                cs.push(c);
            } else if lex.try_skip_char_literal() {
            } else if let Some(l) = lex.try_read_raw_string().or_else(|| lex.try_read_quoted_string()) {
                ls.push(l.text);
            } else {
                lex.advance_one();
            }
        }
        assert_eq!((cs, ls), (vec![(1, " c1")], vec![String::from("a\nABC"), String::from("hi")]));

        let dir = TestDir::new("rust-lex-sub");
        for (p, b) in [
            ("crates/clean/src/lib.rs", "#[cfg(test)]\nmod tests;\n"),
            ("crates/clean/src/tests.rs", "fn t() {}\n"),
            ("crates/dual/src/lib.rs", "#[cfg(test)]\nmod tests;\n"),
            ("crates/dual/src/main.rs", "mod tests;\nfn main() {}\n"),
            ("crates/dual/src/tests.rs", "pub fn leak() {}\n"),
        ] {
            dir.write(p, b);
        }
        assert!(
            is_cfg_test_only_submodule(dir.path(), "crates/clean/src/tests.rs")
                && !is_cfg_test_only_submodule(dir.path(), "crates/dual/src/tests.rs")
        );
    }
}
