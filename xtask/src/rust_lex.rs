//! Shared Rust source lexical utilities, `#[cfg(test)]` masking, and string/comment
//! tokenization used across `xtask` policy gates (`brands`, `layers`, `telemetry`,
//! `citations`, `status`).

/// Returns `true` when `b` is an ASCII Rust identifier continuation byte (`[A-Za-z0-9_]`).
#[must_use]
pub(crate) const fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Returns `true` when byte `i - 1` exists and is an ASCII identifier byte.
#[must_use]
pub(crate) fn prev_is_ident(bytes: &[u8], i: usize) -> bool {
    i > 0 && is_ident_byte(bytes[i - 1])
}

/// Count ASCII newline (`\n`) bytes in `bytes`.
#[must_use]
pub(crate) fn count_newlines(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&b| b == b'\n').count()
}

/// Starting at `//` (`bytes[start..start+2] == b"//"`), return the index of the terminating
/// `\n` (not consumed) or `bytes.len()`.
#[must_use]
pub(crate) fn skip_line_comment_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start.saturating_add(2);
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    i
}

/// Starting at `/*` (`bytes[start..start+2] == b"/*"`), return the byte index immediately
/// after the matching closing `*/` (handling nested `/* ... */`), or `bytes.len()`.
#[must_use]
pub(crate) fn skip_block_comment_end(bytes: &[u8], start: usize) -> usize {
    let mut depth = 1usize;
    let mut i = start.saturating_add(2);
    while i < bytes.len() && depth > 0 {
        if bytes.get(i..i + 2) == Some(b"/*") {
            depth += 1;
            i += 2;
        } else if bytes.get(i..i + 2) == Some(b"*/") {
            depth -= 1;
            i += 2;
        } else {
            i += 1;
        }
    }
    i
}

/// Starting at `start` (pointing at opening `"`), return the byte index immediately after the
/// closing `"` (respecting `\` escapes), or `bytes.len()`.
#[must_use]
pub(crate) fn skip_quoted_string_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start.saturating_add(1);
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' {
            i = i.saturating_add(2).min(bytes.len());
            continue;
        }
        if b == b'"' {
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

/// Attempt to parse a raw string literal (`r"..."`, `r#"..."#`, `br"..."`, `cr#"..."#`)
/// starting at `start`.
///
/// Returns `Some((open_quote_end, close_end, hashes))` where `open_quote_end` is the byte index
/// right after the opening `"`, `close_end` is the byte index right after the closing `"#*`
/// (or `bytes.len()` if unterminated), and `hashes` is the delimiter `#` count.
#[must_use]
pub(crate) fn find_raw_string_span(bytes: &[u8], start: usize) -> Option<(usize, usize, usize)> {
    if prev_is_ident(bytes, start) {
        return None;
    }
    let first = *bytes.get(start)?;
    let r_pos = if first == b'r' {
        start
    } else if (first == b'b' || first == b'c') && bytes.get(start + 1) == Some(&b'r') {
        start + 1
    } else {
        return None;
    };
    let mut hashes = 0usize;
    while bytes.get(r_pos + 1 + hashes) == Some(&b'#') {
        hashes += 1;
    }
    let open_quote = r_pos + 1 + hashes;
    if bytes.get(open_quote) != Some(&b'"') {
        return None;
    }
    let prefix_end = open_quote + 1;
    let mut i = prefix_end;
    while i < bytes.len() {
        if bytes[i] == b'"' && bytes.get(i + 1..i + 1 + hashes).is_some_and(|s| s.iter().all(|&b| b == b'#')) {
            return Some((prefix_end, i + 1 + hashes, hashes));
        }
        i += 1;
    }
    Some((prefix_end, bytes.len(), hashes))
}

fn find_escaped_char_end(bytes: &[u8], mut p: usize) -> Option<usize> {
    if p >= bytes.len() || bytes[p] == b'\n' {
        return None;
    }
    p += 1; // Escaped char (`'`, `\`, `n`, `x`, `u`, etc.).
    let limit = (p + 14).min(bytes.len());
    while p < limit {
        if bytes[p] == b'\n' {
            return None;
        }
        if bytes[p] == b'\'' {
            return Some(p);
        }
        p += 1;
    }
    None
}

/// Attempt to parse a Rust character or byte-character literal (`'x'`, `b'x'`, `'\n'`,
/// `'\u{1234}'`) starting at `start`, distinguishing it from lifetime tokens like `'a`.
///
/// Returns `Some((open_quote_pos, close_quote_pos))` where `close_quote_pos` is the byte index
/// of the closing `'`.
#[must_use]
pub(crate) fn find_char_literal_span(src: &str, start: usize) -> Option<(usize, usize)> {
    let bytes = src.as_bytes();
    let first = *bytes.get(start)?;
    let q_pos = if first == b'b' && bytes.get(start + 1) == Some(&b'\'') && !prev_is_ident(bytes, start) {
        start + 1
    } else if first == b'\'' {
        start
    } else {
        return None;
    };
    let end_q = if bytes.get(q_pos + 1) == Some(&b'\\') {
        find_escaped_char_end(bytes, q_pos + 2)
    } else if let Some(rest) = src.get(q_pos + 1..)
        && let Some(ch) = rest.chars().next().filter(|&c| c != '\'' && c != '\n' && c != '\r')
        && bytes.get(q_pos + 1 + ch.len_utf8()) == Some(&b'\'')
    {
        Some(q_pos + 1 + ch.len_utf8())
    } else {
        None
    }?;
    Some((q_pos, end_q))
}

fn push_blanked(bytes: &[u8], dst: &mut Vec<u8>) {
    for &b in bytes {
        dst.push(if b == b'\n' { b'\n' } else { b' ' });
    }
}

/// Blank comments and string/character literals while preserving exact byte offsets and line
/// numbers, returning `(blanked_all, comments_only_blanked)`.
///
/// When `keep_delimiters` is `true`, `blanked_all` retains the outer string/char quote
/// delimiters (`"   "`, `r#"   "#`, `' '`) as expected by `layers` and `telemetry`; when
/// `false`, `blanked_all` blanks the entire literal span including delimiters (as used by
/// `status` function-body scanning).
#[must_use]
pub(crate) fn blank_comments_and_literals_both(src: &str, keep_delimiters: bool) -> (String, String) {
    let bytes = src.as_bytes();
    let mut prod = Vec::with_capacity(bytes.len());
    let mut keep_lits = Vec::with_capacity(bytes.len());
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes.get(i..i + 2) == Some(b"//") {
            let end = skip_line_comment_end(bytes, i);
            let span_end = if bytes.get(end) == Some(&b'\n') { end + 1 } else { end };
            push_blanked(&bytes[i..span_end], &mut prod);
            push_blanked(&bytes[i..span_end], &mut keep_lits);
            i = span_end;
        } else if bytes.get(i..i + 2) == Some(b"/*") {
            let end = skip_block_comment_end(bytes, i);
            push_blanked(&bytes[i..end], &mut prod);
            push_blanked(&bytes[i..end], &mut keep_lits);
            i = end;
        } else if let Some((prefix_end, close_end, hashes)) = find_raw_string_span(bytes, i) {
            keep_lits.extend_from_slice(&bytes[i..close_end]);
            if keep_delimiters {
                let closing_len = 1 + hashes;
                let has_close = close_end >= prefix_end + closing_len
                    && bytes.get(close_end - closing_len) == Some(&b'"')
                    && bytes.get(close_end - hashes..close_end).is_some_and(|s| s.iter().all(|&b| b == b'#'));
                prod.extend_from_slice(&bytes[i..prefix_end]);
                if has_close {
                    let body_end = close_end - closing_len;
                    push_blanked(&bytes[prefix_end..body_end], &mut prod);
                    prod.extend_from_slice(&bytes[body_end..close_end]);
                } else {
                    push_blanked(&bytes[prefix_end..close_end], &mut prod);
                }
            } else {
                push_blanked(&bytes[i..close_end], &mut prod);
            }
            i = close_end;
        } else if let Some((q_pos, end_q)) = find_char_literal_span(src, i) {
            let close_end = end_q + 1;
            keep_lits.extend_from_slice(&bytes[i..close_end]);
            if keep_delimiters {
                prod.extend(std::iter::repeat_n(b'\'', q_pos - i + 1));
                push_blanked(&bytes[q_pos + 1..end_q], &mut prod);
                prod.push(b'\'');
            } else {
                push_blanked(&bytes[i..close_end], &mut prod);
            }
            i = close_end;
        } else if (bytes[i] == b'b' || bytes[i] == b'c') && bytes.get(i + 1) == Some(&b'"') && !prev_is_ident(bytes, i) {
            let close_end = skip_quoted_string_end(bytes, i + 1);
            keep_lits.extend_from_slice(&bytes[i..close_end]);
            if keep_delimiters {
                prod.push(bytes[i]);
                blank_quoted_inner(&bytes[i + 1..close_end], &mut prod);
            } else {
                push_blanked(&bytes[i..close_end], &mut prod);
            }
            i = close_end;
        } else if bytes[i] == b'"' {
            let close_end = skip_quoted_string_end(bytes, i);
            keep_lits.extend_from_slice(&bytes[i..close_end]);
            if keep_delimiters {
                blank_quoted_inner(&bytes[i..close_end], &mut prod);
            } else {
                push_blanked(&bytes[i..close_end], &mut prod);
            }
            i = close_end;
        } else {
            prod.push(bytes[i]);
            keep_lits.push(bytes[i]);
            i += 1;
        }
    }

    let prod_str = String::from_utf8(prod).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
    let lits_str = String::from_utf8(keep_lits).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
    (prod_str, lits_str)
}

fn blank_quoted_inner(span: &[u8], prod: &mut Vec<u8>) {
    if span.is_empty() {
        return;
    }
    prod.push(b'"');
    let mut j = 1usize;
    while j < span.len() {
        if span[j] == b'\\' {
            prod.push(b' ');
            j += 1;
            if j < span.len() {
                prod.push(if span[j] == b'\n' { b'\n' } else { b' ' });
                j += 1;
            }
            continue;
        }
        if span[j] == b'"' && j + 1 == span.len() {
            prod.push(b'"');
            break;
        }
        prod.push(if span[j] == b'\n' { b'\n' } else { b' ' });
        j += 1;
    }
}

/// Blank comments and string/character literals (retaining quote delimiters) while preserving
/// exact byte offsets and line numbers.
#[must_use]
pub(crate) fn blank_comments_and_literals(src: &str) -> String {
    blank_comments_and_literals_both(src, true).0
}

/// Production-only stripped views of a Rust source file (comments, literals, and `#[cfg(test)]`
/// items blanked with spaces while preserving line numbers).
pub(crate) struct StrippedRust {
    /// Comments, string/char literals, and test-only `#[cfg(...)]` items blanked.
    pub(crate) prod_code: String,
    /// Comments and test-only `#[cfg(...)]` items blanked, string/char literals retained.
    pub(crate) no_comments_code: String,
}

/// Produce both `prod_code` and `no_comments_code` with test-only `#[cfg(...)]` items masked.
#[must_use]
pub(crate) fn strip_non_prod_rust_both(src: &str) -> StrippedRust {
    let (blanked_all, blanked_comments_only) = blank_comments_and_literals_both(src, true);
    let mut prod_bytes = blanked_all.into_bytes();
    let mut comment_only_bytes = blanked_comments_only.into_bytes();
    let snapshot = prod_bytes.clone();
    mask_cfg_test_items(&snapshot, &mut [&mut prod_bytes, &mut comment_only_bytes]);
    StrippedRust {
        prod_code: String::from_utf8(prod_bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
        no_comments_code: String::from_utf8(comment_only_bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
    }
}

/// Produce `prod_code` only (comments, literals, and test-only `#[cfg(...)]` items blanked).
#[must_use]
pub(crate) fn strip_non_prod_rust(src: &str) -> String {
    let no_comments_or_lits = blank_comments_and_literals(src);
    strip_cfg_test_items(&no_comments_or_lits)
}

/// Mask all test-only `#[cfg(...)]` items out of an already comment/literal-blanked Rust source.
#[must_use]
pub(crate) fn strip_cfg_test_items(code: &str) -> String {
    let bytes = code.as_bytes();
    let mut out = bytes.to_vec();
    mask_cfg_test_items(bytes, &mut [&mut out]);
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// Mask every test-only `#[cfg(...)]` item in `code_bytes` across all `targets` in place,
/// preserving newlines so line numbers remain invariant.
pub(crate) fn mask_cfg_test_items(code_bytes: &[u8], targets: &mut [&mut Vec<u8>]) {
    let mut i = 0usize;
    while i < code_bytes.len() {
        if code_bytes[i] == b'#'
            && let Some(after_attr) = match_cfg_test_attr(code_bytes, i)
        {
            let body_start = skip_following_attrs(code_bytes, after_attr);
            let item_end = find_item_end(code_bytes, body_start);
            for target in targets.iter_mut() {
                for b in target.iter_mut().take(item_end).skip(i) {
                    if *b != b'\n' {
                        *b = b' ';
                    }
                }
            }
            i = item_end;
            continue;
        }
        i += 1;
    }
}

/// If `bytes[start..]` begins with an outer attribute `#[...]` whose `cfg(...)` predicate can
/// never hold in a non-`test` build, return the byte index right after the closing `]`.
#[must_use]
pub(crate) fn match_cfg_test_attr(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start) != Some(&b'#') || bytes.get(start + 1) != Some(&b'[') {
        return None;
    }
    let mut p = start + 2;
    let mut bracket_depth = 1usize;
    while p < bytes.len() && bracket_depth > 0 {
        match bytes[p] {
            b'[' => bracket_depth += 1,
            b']' => bracket_depth -= 1,
            _ => {}
        }
        p += 1;
    }
    if bracket_depth != 0 {
        return None;
    }
    let inner = std::str::from_utf8(&bytes[start + 2..p - 1]).ok()?;
    let compact: String = inner.chars().filter(|c| !c.is_whitespace()).collect();
    is_cfg_test_only(&compact).then_some(p)
}

/// Returns `true` iff a whitespace-compacted attribute expression like `cfg(all(unix,test))`
/// can NEVER be active in a non-`test` build (`cfg!(test) == false`).
#[must_use]
pub(crate) fn is_cfg_test_only(compact_attr: &str) -> bool {
    let Some(pred) = strip_cfg_call(compact_attr, "cfg") else {
        return false;
    };
    !eval_cfg_pred_in_prod(pred).0
}

/// Returns `(can_be_true_when_not_test, can_be_false_when_not_test)`.
fn eval_cfg_pred_in_prod(pred: &str) -> (bool, bool) {
    if let Some(inner) = strip_cfg_call(pred, "not") {
        let (t, f) = eval_cfg_pred_in_prod(inner);
        return (f, t);
    }
    if let Some(inner) = strip_cfg_call(pred, "all") {
        let args = split_cfg_args(inner);
        if args.is_empty() {
            return (true, false);
        }
        let mut all_true = true;
        let mut any_false = false;
        for arg in args {
            let (t, f) = eval_cfg_pred_in_prod(arg);
            all_true &= t;
            any_false |= f;
        }
        return (all_true, any_false);
    }
    if let Some(inner) = strip_cfg_call(pred, "any") {
        let args = split_cfg_args(inner);
        if args.is_empty() {
            return (false, true);
        }
        let mut any_true = false;
        let mut all_false = true;
        for arg in args {
            let (t, f) = eval_cfg_pred_in_prod(arg);
            any_true |= t;
            all_false &= f;
        }
        return (any_true, all_false);
    }
    if pred == "test" { (false, true) } else { (true, true) }
}

fn strip_cfg_call<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let rest = s.strip_prefix(name)?.strip_prefix('(')?;
    let bytes = rest.as_bytes();
    let mut depth = 1usize;
    let mut in_quotes = false;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => in_quotes = !in_quotes,
            b'(' if !in_quotes => depth += 1,
            b')' if !in_quotes => {
                depth -= 1;
                if depth == 0 {
                    return if i + 1 == bytes.len() { rest.strip_suffix(')') } else { None };
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn split_cfg_args(s: &str) -> Vec<&str> {
    let mut args = Vec::new();
    let mut depth = 0usize;
    let mut in_quotes = false;
    let mut start = 0usize;
    for (i, ch) in s.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            '(' if !in_quotes => depth += 1,
            ')' if !in_quotes => depth = depth.saturating_sub(1),
            ',' if !in_quotes && depth == 0 => {
                if let Some(arg) = s.get(start..i)
                    && !arg.is_empty()
                {
                    args.push(arg);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    if let Some(arg) = s.get(start..)
        && !arg.is_empty()
    {
        args.push(arg);
    }
    args
}

/// Skip whitespace and subsequent outer attributes `#[...]` following an attribute.
#[must_use]
pub(crate) fn skip_following_attrs(bytes: &[u8], mut pos: usize) -> usize {
    loop {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if bytes.get(pos) == Some(&b'#') && bytes.get(pos + 1) == Some(&b'[') {
            pos += 2;
            let mut depth = 1usize;
            while pos < bytes.len() && depth > 0 {
                match bytes[pos] {
                    b'[' => depth += 1,
                    b']' => depth -= 1,
                    _ => {}
                }
                pos += 1;
            }
        } else {
            return pos;
        }
    }
}

fn read_ident_at(bytes: &[u8], start: usize) -> Option<(&[u8], usize)> {
    let first = *bytes.get(start)?;
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return None;
    }
    let mut end = start + 1;
    while end < bytes.len() && is_ident_byte(bytes[end]) {
        end += 1;
    }
    Some((&bytes[start..end], end))
}

fn skip_ws(bytes: &[u8], mut pos: usize) -> usize {
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    pos
}

fn starts_with_rust_item_head(bytes: &[u8], mut pos: usize) -> bool {
    let mut saw_pub = false;
    loop {
        pos = skip_ws(bytes, pos);
        let Some((word, after)) = read_ident_at(bytes, pos) else {
            return false;
        };
        match word {
            b"pub" if !saw_pub => {
                saw_pub = true;
                pos = skip_ws(bytes, after);
                if bytes.get(pos) == Some(&b'(') {
                    let mut depth = 1usize;
                    pos += 1;
                    while pos < bytes.len() && depth > 0 {
                        match bytes[pos] {
                            b'(' => depth += 1,
                            b')' => depth -= 1,
                            _ => {}
                        }
                        pos += 1;
                    }
                }
            }
            b"default" | b"async" | b"unsafe" => {
                pos = after;
            }
            b"extern" | b"fn" | b"struct" | b"enum" | b"trait" | b"impl" | b"mod" | b"type" | b"static" | b"const" | b"use" => {
                return true;
            }
            b"macro_rules" => {
                pos = skip_ws(bytes, after);
                return bytes.get(pos) == Some(&b'!');
            }
            b"union" => {
                pos = skip_ws(bytes, after);
                return read_ident_at(bytes, pos).is_some();
            }
            _ => return false,
        }
    }
}

/// Find the end byte offset of a Rust item, field, variant, match arm, parameter, or trailing
/// expression in comment/literal-blanked source bytes.
#[must_use]
pub(crate) fn find_item_end(bytes: &[u8], mut pos: usize) -> usize {
    let is_item_decl = starts_with_rust_item_head(bytes, pos);
    let mut brace_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut angle_depth = 0usize;
    let mut saw_brace = false;
    while pos < bytes.len() {
        match bytes[pos] {
            b'{' => {
                if paren_depth == 0 && bracket_depth == 0 {
                    saw_brace = true;
                }
                brace_depth += 1;
            }
            b'}' => {
                if brace_depth > 0 {
                    brace_depth -= 1;
                    if brace_depth == 0 && paren_depth == 0 && bracket_depth == 0 && saw_brace {
                        return pos + 1;
                    }
                } else if paren_depth == 0 && bracket_depth == 0 && !saw_brace {
                    return pos;
                }
            }
            b'(' => paren_depth += 1,
            b')' => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                } else if brace_depth == 0 && bracket_depth == 0 && !saw_brace {
                    return pos;
                }
            }
            b'[' => bracket_depth += 1,
            b']' => {
                if bracket_depth > 0 {
                    bracket_depth -= 1;
                } else if brace_depth == 0 && paren_depth == 0 && !saw_brace {
                    return pos;
                }
            }
            b'<' if !is_item_decl
                && brace_depth == 0
                && paren_depth == 0
                && bracket_depth == 0
                && !saw_brace
                && pos > 0
                && (is_ident_byte(bytes[pos - 1]) || bytes[pos - 1] == b':')
                && !matches!(bytes.get(pos + 1), Some(b'=' | b'<')) =>
            {
                angle_depth += 1;
            }
            b'>' if angle_depth > 0 && brace_depth == 0 && paren_depth == 0 && bracket_depth == 0 => {
                if pos > 0 && bytes[pos - 1] == b'=' {
                    angle_depth = 0;
                } else if pos == 0 || bytes[pos - 1] != b'-' {
                    angle_depth -= 1;
                }
            }
            b';' if brace_depth == 0 && paren_depth == 0 && bracket_depth == 0 && !saw_brace => {
                return pos + 1;
            }
            b',' if !is_item_decl && brace_depth == 0 && paren_depth == 0 && bracket_depth == 0 && angle_depth == 0 && !saw_brace => {
                return pos + 1;
            }
            _ => {}
        }
        pos += 1;
    }
    bytes.len()
}

/// Decoded Rust string literal with per-byte source line mapping for accurate diagnostics.
pub(crate) struct ExtractedLiteral {
    pub(crate) text: String,
    pub(crate) byte_lines: Vec<usize>,
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
    pub(crate) quote_start: usize,
    pub(crate) quote_end: usize,
}

/// Line-tracking Rust lexer that extracts comments, identifiers, and decoded string literals.
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
        self.pos += 1;
        if b == b'\n' {
            self.line += 1;
        }
        Some(b)
    }

    pub(crate) fn try_read_comment(&mut self) -> Option<(usize, &'a str)> {
        if self.bytes.get(self.pos..self.pos + 2) == Some(b"//") {
            let start_line = self.line;
            let body_start = self.pos + 2;
            let body_end = skip_line_comment_end(self.bytes, self.pos);
            self.pos = if self.bytes.get(body_end) == Some(&b'\n') {
                self.line += 1;
                body_end + 1
            } else {
                body_end
            };
            return Some((start_line, self.src.get(body_start..body_end).unwrap_or("")));
        }
        if self.bytes.get(self.pos..self.pos + 2) == Some(b"/*") {
            let start_line = self.line;
            let body_start = self.pos + 2;
            let end = skip_block_comment_end(self.bytes, self.pos);
            let body_end = if end >= body_start + 2 && self.bytes.get(end - 2..end) == Some(b"*/") { end - 2 } else { end };
            self.line += count_newlines(&self.bytes[self.pos..end]);
            self.pos = end;
            return Some((start_line, self.src.get(body_start..body_end).unwrap_or("")));
        }
        None
    }

    pub(crate) fn try_read_identifier(&mut self) -> Option<(usize, &'a str)> {
        let mut start = self.pos;
        if self.peek() == Some(b'r')
            && self.bytes.get(self.pos + 1) == Some(&b'#')
            && self.bytes.get(self.pos + 2).is_some_and(|&b| b.is_ascii_alphabetic() || b == b'_')
            && !prev_is_ident(self.bytes, self.pos)
        {
            self.pos += 2;
            start = self.pos;
        }
        let first = self.peek()?;
        if !(first.is_ascii_alphabetic() || first == b'_') || prev_is_ident(self.bytes, self.pos) {
            return None;
        }
        let line = self.line;
        while let Some(b) = self.peek() {
            if is_ident_byte(b) {
                self.pos += 1;
            } else {
                break;
            }
        }
        Some((line, self.src.get(start..self.pos).unwrap_or("")))
    }

    pub(crate) fn try_skip_char_literal(&mut self) -> bool {
        if let Some((_, end_q)) = find_char_literal_span(self.src, self.pos) {
            self.pos = end_q + 1;
            return true;
        }
        false
    }

    pub(crate) fn try_read_raw_string(&mut self) -> Option<ExtractedLiteral> {
        let (prefix_end, close_end, hashes) = find_raw_string_span(self.bytes, self.pos)?;
        let quote_start = self.pos;
        let start_line = self.line;
        let closing_len = 1 + hashes;
        let body_end = if close_end >= prefix_end + closing_len
            && self.bytes.get(close_end - closing_len) == Some(&b'"')
            && self.bytes.get(close_end - hashes..close_end).is_some_and(|s| s.iter().all(|&b| b == b'#'))
        {
            close_end - closing_len
        } else {
            close_end
        };
        let body = self.src.get(prefix_end..body_end).unwrap_or("");
        let mut text = String::with_capacity(body.len());
        let mut byte_lines = Vec::with_capacity(body.len());
        for ch in body.chars() {
            let line = self.line;
            push_char_with_line(&mut text, &mut byte_lines, ch, line);
            if ch == '\n' {
                self.line += 1;
            }
        }
        self.pos = close_end;
        Some(ExtractedLiteral { text, byte_lines, start_line, end_line: self.line, quote_start, quote_end: self.pos })
    }

    pub(crate) fn try_read_quoted_string(&mut self) -> Option<ExtractedLiteral> {
        let quote_start = self.pos;
        let start_line = self.line;
        if self.peek() == Some(b'"') {
            self.pos += 1;
        } else if matches!(self.peek(), Some(b'b' | b'c')) && self.bytes.get(self.pos + 1) == Some(&b'"') && !prev_is_ident(self.bytes, self.pos) {
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
        Some(ExtractedLiteral { text, byte_lines, start_line, end_line: self.line, quote_start, quote_end: self.pos })
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
                if matches!(ws, b' ' | b'\t' | b'\r' | b'\n') {
                    self.advance_one();
                } else {
                    break;
                }
            }
            return;
        }
        self.advance_one();
        match next {
            b'n' => push_char_with_line(text, byte_lines, '\n', esc_line),
            b'r' => push_char_with_line(text, byte_lines, '\r', esc_line),
            b't' => push_char_with_line(text, byte_lines, '\t', esc_line),
            b'0' => push_char_with_line(text, byte_lines, '\0', esc_line),
            b'\\' => push_char_with_line(text, byte_lines, '\\', esc_line),
            b'"' => push_char_with_line(text, byte_lines, '"', esc_line),
            b'\'' => push_char_with_line(text, byte_lines, '\'', esc_line),
            b'x' => {
                if let (Some(h), Some(l)) = (self.peek(), self.bytes.get(self.pos + 1).copied())
                    && let (Some(hd), Some(ld)) = ((char::from(h)).to_digit(16), (char::from(l)).to_digit(16))
                {
                    self.pos += 2;
                    let byte_val = ((hd << 4) | ld) as u8;
                    push_char_with_line(text, byte_lines, char::from(byte_val), esc_line);
                } else {
                    push_char_with_line(text, byte_lines, 'x', esc_line);
                }
            }
            b'u' if self.peek() == Some(b'{') => {
                self.pos += 1;
                let mut hex = String::new();
                while let Some(b) = self.peek() {
                    if b == b'}' {
                        self.pos += 1;
                        break;
                    }
                    if b == b'_' {
                        self.pos += 1;
                        continue;
                    }
                    if (b as char).is_ascii_hexdigit() && hex.len() < 8 {
                        hex.push(char::from(b));
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                if let Ok(code) = u32::from_str_radix(&hex, 16)
                    && let Some(ch) = char::from_u32(code)
                {
                    push_char_with_line(text, byte_lines, ch, esc_line);
                }
            }
            other => push_char_with_line(text, byte_lines, char::from(other), esc_line),
        }
    }
}

pub(crate) fn push_char_with_line(text: &mut String, byte_lines: &mut Vec<usize>, ch: char, line: usize) {
    text.push(ch);
    byte_lines.resize(text.len(), line);
}

#[cfg(test)]
mod tests {
    use super::{RustLexer, blank_comments_and_literals_both, is_cfg_test_only, strip_non_prod_rust, strip_non_prod_rust_both};

    #[test]
    fn cfg_test_predicate_handles_boolean_combinators() {
        assert!(is_cfg_test_only("cfg(test)"));
        assert!(is_cfg_test_only("cfg(all(unix,test))"));
        assert!(!is_cfg_test_only("cfg(any(unix,test))"));
        assert!(!is_cfg_test_only("cfg(not(test))"));
        assert!(!is_cfg_test_only("cfg(all(unix,not(test)))"));
        assert!(is_cfg_test_only("cfg(not(not(test)))"));
    }

    #[test]
    fn stripping_preserves_line_numbers_and_handles_nested_comments_and_raw_strings() {
        let src = "/* a /* b */\n */\nconst S: &str = r#\"line1\nline2\"#;\n#[cfg(test)]\nmod t { fn x() {} }\npub fn live() {}\n";
        let (all_blank, comments_only) = blank_comments_and_literals_both(src, true);
        assert_eq!(all_blank.lines().count(), src.lines().count());
        assert_eq!(comments_only.lines().count(), src.lines().count());
        assert!(!all_blank.contains("line1"));
        assert!(comments_only.contains("line1"));

        let stripped = strip_non_prod_rust(src);
        assert!(!stripped.contains("fn x"));
        assert!(stripped.contains("pub fn live()"));

        let both = strip_non_prod_rust_both(src);
        assert_eq!(both.prod_code, stripped);
        assert!(both.no_comments_code.contains("line1"));
        assert!(!both.no_comments_code.contains("fn x"));
    }

    #[test]
    fn lexer_reads_comments_raw_strings_escapes_and_skips_chars() {
        let src = "// c1\nlet c = '\\'';\nlet s = \"a\\n\\u{41}\";\nlet r = r#\"hi\"#;\n";
        let mut lex = RustLexer::new(src);
        let mut comments = Vec::new();
        let mut lits = Vec::new();
        while !lex.is_eof() {
            if let Some((ln, body)) = lex.try_read_comment() {
                comments.push((ln, body));
            } else if lex.try_skip_char_literal() {
            } else if let Some(lit) = lex.try_read_raw_string().or_else(|| lex.try_read_quoted_string()) {
                lits.push(lit.text);
            } else {
                lex.advance_one();
            }
        }
        assert_eq!(comments, vec![(1, " c1")]);
        assert_eq!(lits, vec![String::from("a\nA"), String::from("hi")]);
    }

    #[test]
    fn da_03_01_find_item_end_respects_array_semicolon_and_comma_and_closing_delims() {
        let src = "\
#[cfg(test)]
fn test_arr() -> [u8; 4] { let _ = secret_arr(); [0; 4] }
pub fn prod_after_arr() {}

#[cfg(test)]
const TEST_CONST: [u8; 4] = [1, 2, 3, 4];
pub fn prod_after_const() {}

#[cfg(test)]
pub(crate) fn test_generic<A, B>(a: A) -> Result<A, B> where A: Copy, B: Clone { secret_gen(); }
pub fn prod_after_generic() {}

struct S {
    #[cfg(test)]
    test_field: Result<u8, u16>,
    pub prod_field: u32,
}

struct S2 {
    pub a: u32,
    #[cfg(test)]
    last_test_field: [u8; 2]
}
pub fn prod_after_struct() {}

enum E {
    #[cfg(test)]
    TestVar(u8, u16),
    ProdVar,
}

fn with_param(#[cfg(test)] test_param: i32) { prod_body(); }

fn with_match(x: u8) {
    match x {
        #[cfg(test)]
        0 => test_arm(),
        _ => prod_arm(),
    }
}

fn with_tail() -> i32 {
    #[cfg(test)]
    test_tail_expr
}
pub fn prod_after_tail() {}
";
        let stripped = strip_non_prod_rust(src);
        for masked in
            ["secret_arr", "TEST_CONST", "secret_gen", "test_field", "last_test_field", "TestVar", "test_param", "test_arm", "test_tail_expr"]
        {
            assert!(!stripped.contains(masked), "expected `{masked}` to be masked:\n{stripped}");
        }
        for kept in [
            "prod_after_arr",
            "prod_after_const",
            "prod_after_generic",
            "prod_field",
            "prod_after_struct",
            "ProdVar",
            "prod_body",
            "prod_arm",
            "prod_after_tail",
        ] {
            assert!(stripped.contains(kept), "expected production symbol `{kept}` to remain:\n{stripped}");
        }
    }

    #[test]
    fn da_03_06_string_line_continuation_skips_blank_lines_and_tracks_line_numbers() {
        let src = "let a = \"Foo\\\n\n   \r\n\tBar\";\nlet b = \"Next\";\n";
        let mut lex = RustLexer::new(src);
        let mut lits = Vec::new();
        while !lex.is_eof() {
            if let Some(lit) = lex.try_read_quoted_string() {
                lits.push((lit.start_line, lit.end_line, lit.text));
            } else {
                lex.advance_one();
            }
        }
        assert_eq!(lits, vec![(1, 4, String::from("FooBar")), (5, 5, String::from("Next"))]);
    }
}
