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

fn find_escaped_char_end(bytes: &[u8], p: usize, is_byte: bool) -> Option<usize> {
    let esc = *bytes.get(p)?;
    match esc {
        b'\'' | b'"' | b'\\' | b'n' | b'r' | b't' | b'0' => (bytes.get(p + 1) == Some(&b'\'')).then_some(p + 1),
        b'x' => (bytes.get(p + 1).is_some_and(u8::is_ascii_hexdigit)
            && bytes.get(p + 2).is_some_and(u8::is_ascii_hexdigit)
            && bytes.get(p + 3) == Some(&b'\''))
        .then_some(p + 3),
        b'u' if !is_byte && bytes.get(p + 1) == Some(&b'{') => {
            let mut i = p + 2;
            let mut hex_digits = 0usize;
            while let Some(&b) = bytes.get(i) {
                if b == b'}' {
                    return (hex_digits > 0 && bytes.get(i + 1) == Some(&b'\'')).then_some(i + 1);
                }
                if b.is_ascii_hexdigit() {
                    hex_digits += 1;
                    i += 1;
                } else if b == b'_' {
                    i += 1;
                } else {
                    return None;
                }
            }
            None
        }
        _ => None,
    }
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
    let (q_pos, is_byte) = if first == b'b' && bytes.get(start + 1) == Some(&b'\'') && !prev_is_ident(bytes, start) {
        (start + 1, true)
    } else if first == b'\'' {
        (start, false)
    } else {
        return None;
    };
    let end_q = if bytes.get(q_pos + 1) == Some(&b'\\') {
        find_escaped_char_end(bytes, q_pos + 2, is_byte)
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

/// Returns `true` iff a Rust file under `src/` (e.g. `crates/foo/src/bar/tests.rs` or
/// `crates/foo/src/bar/tests/helper.rs`) is loaded exclusively through a `#[cfg(test)]`
/// `mod <name>;` declaration in its immediate parent module file(s), with ZERO production
/// `mod <name>;` loads in sibling parents (e.g. dual `lib.rs` + `main.rs`) and ZERO production
/// `#[path = "..."]` loads anywhere in the crate's `src/` tree.
///
/// Unlike filename-based `_test.rs` / `tests.rs` exemptions, a file compiled unconditionally
/// into production (e.g. `pub mod t_test;`) returns `false` and stays audited.
pub(crate) fn is_cfg_test_only_submodule(root: &std::path::Path, rel: &str) -> bool {
    let norm = rel.replace('\\', "/");
    let Some((crate_prefix, inside_src)) = norm.split_once("/src/") else {
        return false;
    };
    let parts: Vec<&str> = inside_src.split('/').collect();
    if parts.is_empty() {
        return false;
    }
    let src_root_rel = format!("{crate_prefix}/src");
    let src_prefix = format!("{src_root_rel}/");
    // Walk from the file up through ancestor submodule directories inside `src/`.
    // At each depth, check ALL parent candidates (e.g., BOTH `lib.rs` and `main.rs`, or BOTH
    // `mod.rs` and `<dir>.rs`): require at least one `#[cfg(test)]` declaration AND zero
    // production declarations across all sibling parents.
    let mut exempt_scope: Option<String> = None;
    for depth in (0..parts.len()).rev() {
        let seg = parts[depth];
        let is_leaf = depth + 1 == parts.len();
        let mod_name = if is_leaf {
            let Some(stem) = seg.strip_suffix(".rs") else {
                return false;
            };
            if matches!(stem, "lib" | "main" | "mod") {
                continue;
            }
            stem
        } else {
            seg
        };
        let parent_dir = if depth == 0 { src_root_rel.clone() } else { format!("{src_prefix}{}", parts[..depth].join("/")) };
        let parent_candidates = if depth == 0 {
            [format!("{parent_dir}/lib.rs"), format!("{parent_dir}/main.rs")]
        } else {
            [format!("{parent_dir}/mod.rs"), format!("{parent_dir}.rs")]
        };
        let mut saw_cfg_test_decl = false;
        let mut saw_prod_decl = false;
        for cand in parent_candidates {
            let full = root.join(&cand);
            if !crate::scan_util::is_regular_file(&full) {
                continue;
            }
            let Ok(parent_src) = std::fs::read_to_string(&full) else {
                // Fail closed if an existing parent candidate cannot be read.
                return false;
            };
            let blanked = blank_comments_and_literals(&parent_src);
            if has_external_mod_decl(&blanked, mod_name) {
                let prod = strip_cfg_test_items(&blanked);
                if has_external_mod_decl(&prod, mod_name) {
                    saw_prod_decl = true;
                } else {
                    saw_cfg_test_decl = true;
                }
            }
        }
        if saw_cfg_test_decl && !saw_prod_decl {
            exempt_scope = Some(if is_leaf { norm.clone() } else { format!("{src_prefix}{}/", parts[..=depth].join("/")) });
            break;
        }
    }
    let Some(scope) = exempt_scope else {
        return false;
    };
    !crate_src_has_prod_path_attr_to(root, &src_root_rel, &norm, &scope)
}

fn crate_src_has_prod_path_attr_to(root: &std::path::Path, src_root_rel: &str, target_rel: &str, exempt_scope: &str) -> bool {
    let src_dir = root.join(src_root_rel);
    let mut files = Vec::new();
    if collect_rs_strict_no_symlinks(&src_dir, &mut files).is_err() {
        return true; // Fail closed on walk errors or symlinks.
    }
    let scope_is_dir = exempt_scope.ends_with('/');
    for full in files {
        let Ok(decl_rel) = crate::scan_util::rel_slash(root, &full) else {
            return true;
        };
        if decl_rel == target_rel || (scope_is_dir && decl_rel.starts_with(exempt_scope)) {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&full) else {
            return true;
        };
        if !src.contains("path") {
            continue;
        }
        let stripped = strip_non_prod_rust_both(&src);
        let paths = extract_prod_path_attrs(&stripped.no_comments_code);
        if paths.is_empty() {
            continue;
        }
        let decl_dir = decl_rel.rsplit_once('/').map_or("", |(d, _)| d);
        let stem_dir = decl_rel
            .rsplit_once('/')
            .map_or(decl_rel.as_str(), |(_, f)| f)
            .strip_suffix(".rs")
            .filter(|stem| !matches!(*stem, "lib" | "main" | "mod"))
            .map(|stem| if decl_dir.is_empty() { stem.to_owned() } else { format!("{decl_dir}/{stem}") });
        for (inline_mods, attr_path) in paths {
            let mut candidate_dirs: Vec<String> = vec![decl_dir.to_owned()];
            if let Some(sd) = stem_dir.as_deref() {
                candidate_dirs.push(sd.to_owned());
            }
            if !inline_mods.is_empty() {
                let inline_rel = inline_mods.join("/");
                if let Some(d) = resolve_rel_path(decl_dir, &inline_rel) {
                    candidate_dirs.push(d);
                }
                if let Some(sd) = stem_dir.as_deref()
                    && let Some(d) = resolve_rel_path(sd, &inline_rel)
                {
                    candidate_dirs.push(d);
                }
            }
            if candidate_dirs.iter().any(|dir| resolved_path_covers_scope(dir, &attr_path, target_rel, exempt_scope)) {
                return true;
            }
        }
    }
    false
}

fn collect_rs_strict_no_symlinks(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) -> Result<(), String> {
    let dir_meta = std::fs::symlink_metadata(dir).map_err(|e| format!("stat {}: {e}", dir.display()))?;
    if dir_meta.file_type().is_symlink() || !dir_meta.file_type().is_dir() {
        return Err(format!("{}: expected non-symlink directory", dir.display()));
    }
    for path in crate::scan_util::read_dir_paths_sorted(dir)? {
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("stat {}: {e}", path.display()))?;
        if meta.file_type().is_symlink() {
            return Err(format!("{}: symlink forbidden in src tree", path.display()));
        }
        if meta.is_dir() {
            collect_rs_strict_no_symlinks(&path, out)?;
        } else if meta.is_file() && crate::scan_util::has_ext_ci(&path, &["rs"]) {
            out.push(path);
        }
    }
    Ok(())
}

fn resolve_rel_path(base_dir: &str, rel_path: &str) -> Option<String> {
    let norm = rel_path.replace('\\', "/");
    let mut parts: Vec<&str> = base_dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in norm.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

fn resolved_path_covers_scope(base_dir: &str, attr_path: &str, target_rel: &str, exempt_scope: &str) -> bool {
    let Some(resolved) = resolve_rel_path(base_dir, attr_path) else {
        return false;
    };
    if resolved == target_rel {
        return true;
    }
    if !resolved.is_empty() && target_rel.starts_with(&format!("{resolved}/")) {
        return true;
    }
    if let Some(mod_dir) = resolved.strip_suffix("/mod.rs")
        && !mod_dir.is_empty()
        && target_rel.starts_with(&format!("{mod_dir}/"))
    {
        return true;
    }
    // If `target_rel` is exempt solely via a `#[cfg(test)]` ancestor directory (`exempt_scope`),
    // any production `#[path = "..."]` pointing inside `exempt_scope` (e.g. `a/entry.rs`, which can
    // load sibling `a/sibling.rs`) invalidates exemption for the entire directory scope.
    if exempt_scope.ends_with('/') && (resolved.starts_with(exempt_scope) || format!("{resolved}/") == exempt_scope) {
        return true;
    }
    false
}

fn read_path_attrs_in_brackets(lex: &mut RustLexer<'_>, bytes: &[u8]) -> Vec<String> {
    let mut found = Vec::new();
    let mut depth = 1usize;
    while !lex.is_eof() && depth > 0 {
        if lex.try_skip_char_literal() || lex.try_read_raw_string().is_some() || lex.try_read_quoted_string().is_some() {
            continue;
        }
        match bytes.get(lex.pos).copied() {
            Some(b'[') => {
                depth += 1;
                lex.advance_one();
            }
            Some(b']') => {
                depth -= 1;
                lex.advance_one();
            }
            _ => {
                let ident_start = lex.pos;
                if let Some((_, word)) = lex.try_read_identifier() {
                    let is_raw = ident_start >= 2 && bytes.get(ident_start - 2..ident_start) == Some(b"r#");
                    let mut prev = ident_start;
                    while prev > 0 && bytes[prev - 1].is_ascii_whitespace() {
                        prev -= 1;
                    }
                    let after_colon_or_dot = prev > 0 && (bytes[prev - 1] == b'.' || (prev >= 2 && &bytes[prev - 2..prev] == b"::"));
                    if word == "path" && !is_raw && !after_colon_or_dot {
                        let eq_pos = skip_ws(bytes, lex.pos);
                        if bytes.get(eq_pos) == Some(&b'=') && bytes.get(eq_pos + 1) != Some(&b'=') {
                            lex.pos = skip_ws(bytes, eq_pos + 1);
                            if let Some(lit) = lex.try_read_raw_string().or_else(|| lex.try_read_quoted_string()) {
                                found.push(lit.text);
                            }
                        }
                    }
                } else {
                    lex.advance_one();
                }
            }
        }
    }
    found
}

/// Extract `(inline_mod_stack, path_value)` pairs from production outer/inner `#[path = "..."]`
/// attributes in a comment- and `#[cfg(test)]`-stripped Rust source (with production literals kept).
fn extract_prod_path_attrs(prod_with_lits: &str) -> Vec<(Vec<String>, String)> {
    let mut out = Vec::new();
    let bytes = prod_with_lits.as_bytes();
    let mut lex = RustLexer::new(prod_with_lits);
    let mut brace_depth = 0usize;
    let mut mod_stack: Vec<(usize, String)> = Vec::new();
    let mut pending_mod_name: Option<String> = None;
    let mut last_outer_path_attr: Option<String> = None;
    while !lex.is_eof() {
        if lex.try_skip_char_literal() || lex.try_read_raw_string().is_some() || lex.try_read_quoted_string().is_some() {
            continue;
        }
        match bytes.get(lex.pos).copied() {
            Some(b'#') => {
                let mut p = skip_ws(bytes, lex.pos + 1);
                let is_inner = bytes.get(p) == Some(&b'!');
                if is_inner {
                    p = skip_ws(bytes, p + 1);
                }
                if bytes.get(p) == Some(&b'[') {
                    lex.pos = p + 1;
                    let cur_mods: Vec<String> = mod_stack.iter().map(|(_, m)| m.clone()).collect();
                    for val in read_path_attrs_in_brackets(&mut lex, bytes) {
                        if !is_inner {
                            last_outer_path_attr = Some(val.clone());
                        }
                        out.push((cur_mods.clone(), val));
                    }
                    continue;
                }
                lex.advance_one();
            }
            Some(b'{') => {
                brace_depth += 1;
                if let Some(mname) = pending_mod_name.take() {
                    let seg = last_outer_path_attr.take().unwrap_or(mname);
                    mod_stack.push((brace_depth, seg));
                } else {
                    last_outer_path_attr = None;
                }
                lex.advance_one();
            }
            Some(b'}') => {
                if mod_stack.last().is_some_and(|(d, _)| *d == brace_depth) {
                    mod_stack.pop();
                }
                brace_depth = brace_depth.saturating_sub(1);
                pending_mod_name = None;
                last_outer_path_attr = None;
                lex.advance_one();
            }
            Some(b';') => {
                pending_mod_name = None;
                last_outer_path_attr = None;
                lex.advance_one();
            }
            _ => {
                let ident_start = lex.pos;
                if let Some((_, word)) = lex.try_read_identifier() {
                    let is_raw = ident_start >= 2 && bytes.get(ident_start - 2..ident_start) == Some(b"r#");
                    if word == "mod" && !is_raw {
                        let next = skip_ws(bytes, lex.pos);
                        let name_start = if bytes.get(next..next + 2) == Some(b"r#") { next + 2 } else { next };
                        if let Some((name_bytes, after_name)) = read_ident_at(bytes, name_start)
                            && let Ok(name_str) = std::str::from_utf8(name_bytes)
                        {
                            let after_ws = skip_ws(bytes, after_name);
                            if bytes.get(after_ws) == Some(&b'{') {
                                pending_mod_name = Some(name_str.to_owned());
                                lex.pos = after_ws;
                                continue;
                            }
                        }
                    }
                } else {
                    lex.advance_one();
                }
            }
        }
    }
    out
}

fn has_external_mod_decl(blanked_code: &str, mod_name: &str) -> bool {
    let bytes = blanked_code.as_bytes();
    let target = mod_name.as_bytes();
    let mut i = 0usize;
    while i + 3 < bytes.len() {
        let is_raw_mod = i >= 2 && &bytes[i - 2..i] == b"r#";
        if &bytes[i..i + 3] == b"mod"
            && !is_raw_mod
            && (i == 0 || !is_ident_byte(bytes[i - 1]))
            && bytes.get(i + 3).is_some_and(u8::is_ascii_whitespace)
        {
            let mut p = i + 3;
            while p < bytes.len() && bytes[p].is_ascii_whitespace() {
                p += 1;
            }
            if bytes.get(p..p + 2) == Some(b"r#") {
                p += 2;
            }
            if bytes.get(p..p + target.len()) == Some(target) {
                let after = p + target.len();
                if bytes.get(after).is_none_or(|b| !is_ident_byte(*b)) {
                    let mut q = after;
                    while q < bytes.len() && bytes[q].is_ascii_whitespace() {
                        q += 1;
                    }
                    if bytes.get(q) == Some(&b';') {
                        return true;
                    }
                }
            }
        }
        i += 1;
    }
    false
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

fn is_rust_keyword_bytes(word: &[u8]) -> bool {
    matches!(
        word,
        b"as"
            | b"async"
            | b"await"
            | b"break"
            | b"const"
            | b"continue"
            | b"crate"
            | b"dyn"
            | b"else"
            | b"enum"
            | b"extern"
            | b"false"
            | b"fn"
            | b"for"
            | b"if"
            | b"impl"
            | b"in"
            | b"let"
            | b"loop"
            | b"match"
            | b"mod"
            | b"move"
            | b"mut"
            | b"pub"
            | b"ref"
            | b"return"
            | b"self"
            | b"Self"
            | b"static"
            | b"struct"
            | b"super"
            | b"trait"
            | b"true"
            | b"type"
            | b"unsafe"
            | b"use"
            | b"where"
            | b"while"
            | b"abstract"
            | b"become"
            | b"box"
            | b"do"
            | b"final"
            | b"macro"
            | b"override"
            | b"priv"
            | b"try"
            | b"typeof"
            | b"unsized"
            | b"virtual"
            | b"yield"
    )
}

fn is_union_type_name_at(bytes: &[u8], pos: usize) -> bool {
    if bytes.get(pos..pos + 2) == Some(b"r#") {
        return read_ident_at(bytes, pos + 2).is_some();
    }
    read_ident_at(bytes, pos).is_some_and(|(ident, _)| !is_rust_keyword_bytes(ident))
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
                return is_union_type_name_at(bytes, pos);
            }
            _ => return false,
        }
    }
}

/// Find the enclosing opening delimiter (`{`, `(`, `[`) immediately surrounding `pos`, if any.
fn find_enclosing_open_delim(bytes: &[u8], pos: usize) -> Option<(usize, u8)> {
    let mut brace = 0usize;
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut i = pos;
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b'}' => brace += 1,
            b'{' => {
                if brace == 0 {
                    return Some((i, b'{'));
                }
                brace -= 1;
            }
            b')' => paren += 1,
            b'(' => {
                if paren == 0 {
                    return Some((i, b'('));
                }
                paren -= 1;
            }
            b']' => bracket += 1,
            b'[' => {
                if bracket == 0 {
                    return Some((i, b'['));
                }
                bracket -= 1;
            }
            _ => {}
        }
    }
    None
}

/// Check whether the header immediately before `delim_pos` (back to the previous `;`, `{`, or `}`)
/// contains one of `keywords` at delimiter depth 0 (and not after `=`).
fn header_before_delim_has_keyword(bytes: &[u8], delim_pos: usize, keywords: &[&[u8]]) -> bool {
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut angle = 0usize;
    let mut i = delim_pos;
    while i > 0 {
        i -= 1;
        let b = bytes[i];
        match b {
            b')' => paren += 1,
            b'(' => {
                if paren == 0 {
                    break;
                }
                paren -= 1;
            }
            b']' => bracket += 1,
            b'[' => {
                if bracket == 0 {
                    break;
                }
                bracket -= 1;
            }
            b'>' if paren == 0 && bracket == 0 && (i == 0 || !matches!(bytes[i - 1], b'=' | b'-')) => angle += 1,
            b'<' if paren == 0 && bracket == 0 && angle > 0 => angle -= 1,
            b';' | b',' | b'{' | b'}' if paren == 0 && bracket == 0 => break,
            b'=' if paren == 0 && bracket == 0 && angle == 0 => return false,
            _ if paren == 0 && bracket == 0 && angle == 0 && is_ident_byte(b) => {
                let end = i + 1;
                while i > 0 && is_ident_byte(bytes[i - 1]) {
                    i -= 1;
                }
                let word = &bytes[i..end];
                if keywords.contains(&word) {
                    // Reject raw identifiers (`r#struct`, `r#fn`, `r#union`, `r#enum`) and path
                    // segments adjacent to `::` or `.` (e.g. `union::S { ... }`, `foo::union { ... }`).
                    let is_raw = i >= 2 && &bytes[i - 2..i] == b"r#";
                    let mut prev = i;
                    while prev > 0 && bytes[prev - 1].is_ascii_whitespace() {
                        prev -= 1;
                    }
                    let preceded_by_path = prev > 0 && (bytes[prev - 1] == b'.' || (prev >= 2 && &bytes[prev - 2..prev] == b"::"));
                    let next = skip_ws(bytes, end);
                    let followed_by_path = bytes.get(next) == Some(&b'.') || bytes.get(next..next + 2) == Some(b"::");
                    let union_missing_name = word == b"union" && !is_union_type_name_at(bytes, next);
                    if !is_raw && !preceded_by_path && !followed_by_path && !union_missing_name {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

/// Returns `true` when `pos` sits directly inside a type-bearing field or parameter list
/// (`struct`/`union` body, `enum` struct-variant body, or `fn` parameter list), where generic
/// type arguments `Type<A, B>` may appear without a `::` turbofish.
fn is_type_field_or_param_context(bytes: &[u8], pos: usize) -> bool {
    let Some((open_pos, delim)) = find_enclosing_open_delim(bytes, pos) else {
        return false;
    };
    match delim {
        b'(' => header_before_delim_has_keyword(bytes, open_pos, &[b"fn", b"struct"]),
        b'{' => {
            if header_before_delim_has_keyword(bytes, open_pos, &[b"struct", b"union"]) {
                return true;
            }
            // Enum struct variant: `enum E { Variant { #[cfg(test)] f: Result<A, B>, } }`.
            if let Some((outer_pos, b'{')) = find_enclosing_open_delim(bytes, open_pos)
                && header_before_delim_has_keyword(bytes, outer_pos, &[b"enum"])
            {
                let mut prev = open_pos;
                while prev > 0 && bytes[prev - 1].is_ascii_whitespace() {
                    prev -= 1;
                }
                if prev > 0 && is_ident_byte(bytes[prev - 1]) {
                    // Reject struct literals inside an enum discriminant (`A = S { ... }`).
                    let mut scan = prev;
                    while scan > outer_pos && is_ident_byte(bytes[scan - 1]) {
                        scan -= 1;
                    }
                    while scan > outer_pos && bytes[scan - 1].is_ascii_whitespace() {
                        scan -= 1;
                    }
                    return scan > outer_pos && matches!(bytes[scan - 1], b'{' | b',' | b']');
                }
            }
            false
        }
        _ => false,
    }
}

fn preceded_by_turbofish_colons(bytes: &[u8], open_pos: usize) -> bool {
    let mut p = open_pos;
    while p > 0 && bytes[p - 1].is_ascii_whitespace() {
        p -= 1;
    }
    p >= 2 && bytes[p - 2] == b':' && bytes[p - 1] == b':'
}

/// Look ahead from `<` at `open_pos` to confirm it has a structurally valid matching `>` before
/// any enclosing delimiter or expression operator (`;`, `=>`, `&&`, `||`, `==`, `!=`, `<=`, `>=`).
fn has_matching_generic_close(bytes: &[u8], open_pos: usize) -> bool {
    if matches!(bytes.get(open_pos + 1), Some(b'=' | b'<')) {
        return false;
    }
    let mut p = open_pos + 1;
    let mut depth = 1usize;
    let mut brace = 0usize;
    let mut paren = 0usize;
    let mut bracket = 0usize;
    while p < bytes.len() {
        let b = bytes[p];
        match b {
            b'{' => brace += 1,
            b'}' => {
                if brace == 0 {
                    return false;
                }
                brace -= 1;
            }
            b'(' => paren += 1,
            b')' => {
                if paren == 0 {
                    return false;
                }
                paren -= 1;
            }
            b'[' => bracket += 1,
            b']' => {
                if bracket == 0 {
                    return false;
                }
                bracket -= 1;
            }
            _ if brace == 0 && paren == 0 && bracket == 0 => match b {
                b';' => return false,
                b'-' if bytes.get(p + 1) == Some(&b'>') => {
                    p += 2;
                    continue;
                }
                b'=' if bytes.get(p + 1) == Some(&b'>') => return false,
                b'&' if bytes.get(p + 1) == Some(&b'&') => return false,
                b'|' if bytes.get(p + 1) == Some(&b'|') => return false,
                b'=' if bytes.get(p + 1) == Some(&b'=') => return false,
                b'!' if bytes.get(p + 1) == Some(&b'=') => return false,
                b'<' => {
                    if matches!(bytes.get(p + 1), Some(b'=' | b'<')) {
                        return false;
                    }
                    depth += 1;
                }
                b'>' => {
                    if bytes.get(p + 1) == Some(&b'=') && depth == 1 {
                        return false;
                    }
                    depth -= 1;
                    if depth == 0 {
                        let next = skip_ws(bytes, p + 1);
                        return match bytes.get(next) {
                            None => true,
                            Some(b',' | b';' | b')' | b']' | b'}' | b'(' | b'{' | b':' | b'.' | b'?' | b'>' | b'=' | b'+' | b'|' | b'-') => true,
                            Some(_) => matches!(read_ident_at(bytes, next), Some((b"where" | b"as", _))),
                        };
                    }
                }
                _ => {}
            },
            _ => {}
        }
        p += 1;
    }
    false
}

/// Find the end byte offset of a Rust item, field, variant, match arm, parameter, or trailing
/// expression in comment/literal-blanked source bytes.
#[must_use]
pub(crate) fn find_item_end(bytes: &[u8], mut pos: usize) -> usize {
    let start_pos = pos;
    let is_item_decl = starts_with_rust_item_head(bytes, pos);
    let in_type_ctx = !is_item_decl && is_type_field_or_param_context(bytes, start_pos);
    let mut brace_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut angle_depth = 0usize;
    let mut saw_brace = false;
    let mut saw_expr_sep = false;
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
            b'=' if !is_item_decl && brace_depth == 0 && paren_depth == 0 && bracket_depth == 0 && angle_depth == 0 => {
                saw_expr_sep = true;
            }
            b'<' if !is_item_decl && brace_depth == 0 && paren_depth == 0 && bracket_depth == 0 && !saw_brace && pos > 0 => {
                let is_turbofish = preceded_by_turbofish_colons(bytes, pos);
                let is_type_generic = in_type_ctx && !saw_expr_sep && is_ident_byte(bytes[pos - 1]);
                if (is_turbofish || is_type_generic) && has_matching_generic_close(bytes, pos) {
                    angle_depth += 1;
                }
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

    #[test]
    fn aud_12_02_long_unicode_char_escape_preserves_quote_parity() {
        let src = "\
pub fn leak() {
    let _ = ('\\u{0______________41}','\"');
    let _ = std::net::TcpStream::connect(\"8.8.8.8:443\");
    let _ = \"\\x43urveBook\";
}
";
        let (prod, keep_lits) = blank_comments_and_literals_both(src, true);
        assert!(prod.contains("std::net::TcpStream::connect"), "prod code must not be blanked:\n{prod}");
        assert!(keep_lits.contains("std::net::TcpStream::connect"), "keep_lits must preserve code:\n{keep_lits}");

        let mut lex = RustLexer::new(src);
        let mut lits = Vec::new();
        while !lex.is_eof() {
            if lex.try_skip_char_literal() {
            } else if let Some(lit) = lex.try_read_raw_string().or_else(|| lex.try_read_quoted_string()) {
                lits.push(lit.text);
            } else {
                lex.advance_one();
            }
        }
        assert_eq!(lits, vec![String::from("8.8.8.8:443"), String::from("CurveBook")]);
    }

    #[test]
    fn aud_12_03_cfg_test_comparison_does_not_mask_following_production_arms_or_fields() {
        let src = "\
pub fn cfg_test_match_arm(x: u8, a: u8, b: u8) -> bool {
    match x {
        #[cfg(test)]
        0 => a<b,
        _ => { let _ = (a, b); let _ = std::net::UdpSocket::bind(\"0.0.0.0:0\"); false }
    }
}
pub fn cfg_test_struct_lit(a: u8, b: u8) -> S {
    S {
        #[cfg(test)]
        flag: a<b,
        val: { let _ = std::net::TcpStream::connect(\"1.1.1.1:443\"); true },
    }
}
pub fn cfg_test_struct_lit_with_gt(a: u8, b: u8) -> S {
    S {
        #[cfg(test)]
        flag: a<b,
        val: { let _ = std::process::Command::new(\"sh\"); 1 } > (0),
    }
}
pub fn cfg_test_turbofish(x: u8) {
    match x {
        #[cfg(test)]
        0 => helper::<u8, u16>(),
        _ => prod_after_turbofish(),
    }
}
enum EnumWithStructVariant {
    Variant {
        #[cfg(test)]
        test_only: Result<u8, u16>,
        pub_prod: u32,
    },
}
";
        let stripped = strip_non_prod_rust(src);
        assert!(stripped.contains("UdpSocket::bind"), "match arm production code masked:\n{stripped}");
        assert!(stripped.contains("TcpStream::connect"), "struct literal field production code masked:\n{stripped}");
        assert!(stripped.contains("Command::new"), "struct literal field with > masked:\n{stripped}");
        assert!(stripped.contains("prod_after_turbofish"), "arm after turbofish masked:\n{stripped}");
        assert!(!stripped.contains("helper"), "turbofish test arm must be masked:\n{stripped}");
        assert!(!stripped.contains("test_only"), "enum struct variant test field must be masked:\n{stripped}");
        assert!(stripped.contains("pub_prod"), "enum struct variant prod field must remain:\n{stripped}");

        // Weak keyword `union` in paths (`union::S`), bare `union { ... }` struct literal, and raw
        // identifiers (`r#struct`, `r#fn`) must NOT trigger type-declaration generic context.
        let path_and_raw = "\
pub fn bypass_union_path(a: u8, b: u8) -> union::S {
    union::S {
        #[cfg(test)]
        flag: a<b,
        leak: { let _ = std::net::TcpStream::connect(\"1.1.1.1:443\"); 1 } > (0),
    }
}
pub fn bypass_raw_struct(a: u8, b: u8) -> r#struct {
    r#struct {
        #[cfg(test)]
        flag: a<b,
        leak: { let _ = std::net::UdpSocket::bind(\"0.0.0.0:0\"); 1 } > (0),
    }
}
pub fn bypass_raw_fn(a: u8, b: u8) {
    r#fn(#[cfg(test)] a<b, { let _ = std::process::Command::new(\"sh\"); 1 } > (0));
}
pub union RealUnion {
    #[cfg(test)]
    test_f: Result<u8, u16>,
    pub prod_f: u32,
}
";
        let stripped2 = strip_non_prod_rust(path_and_raw);
        assert!(stripped2.contains("TcpStream::connect"), "union::S prod field masked:\n{stripped2}");
        assert!(stripped2.contains("UdpSocket::bind"), "r#struct prod field masked:\n{stripped2}");
        assert!(stripped2.contains("Command::new"), "r#fn prod arg masked:\n{stripped2}");
        assert!(!stripped2.contains("test_f"), "RealUnion test field must still be masked:\n{stripped2}");
        assert!(stripped2.contains("prod_f"), "RealUnion prod field must remain:\n{stripped2}");

        // Round-3 #216 (AUD-12-03): `union as u8` and comma-separated `union` before `{` must NOT treat `union` as an item.
        let union_kw_bypass = r#"
pub fn bypass_union_as(union: u16, a: u8, b: u8) {
    #[cfg(test)]
    let _ = union as u8;
    let _ = std::net::TcpStream::connect("1.1.1.1:443");
    foo(union, Bar {
        #[cfg(test)]
        x: a < b,
        y: { let _ = std::net::UdpSocket::bind("0.0.0.0:0"); 1 } > (0),
    });
}
"#;
        let stripped3 = strip_non_prod_rust(union_kw_bypass);
        assert!(stripped3.contains("TcpStream::connect"), "union as u8 masked prod stmt:\n{stripped3}");
        assert!(stripped3.contains("UdpSocket::bind"), "comma-separated union masked prod field:\n{stripped3}");
    }

    #[test]
    fn aud_12_18_33_38_cfg_test_submodule_checks_all_parents_and_prod_path_attrs() {
        let dir = crate::scan_util::test_support::TestDir::new("rust-lex-submodule");
        // 1. Clean #[cfg(test)] mod tests; in lib.rs only -> exempt.
        dir.write("crates/clean/src/lib.rs", "#[cfg(test)]\nmod tests;\n");
        dir.write("crates/clean/src/tests.rs", "fn t() {}\n");
        assert!(super::is_cfg_test_only_submodule(dir.path(), "crates/clean/src/tests.rs"));

        // 2. Dual lib+bin crate: lib.rs marks #[cfg(test)] mod tests;, main.rs loads mod tests; in prod -> NOT exempt!
        dir.write("crates/dual/src/lib.rs", "#[cfg(test)]\nmod tests;\n");
        dir.write("crates/dual/src/main.rs", "mod tests;\nfn main() {}\n");
        dir.write("crates/dual/src/tests.rs", "pub fn leak() {}\n");
        assert!(!super::is_cfg_test_only_submodule(dir.path(), "crates/dual/src/tests.rs"));

        // 3. Production #[path = "tests.rs"] alias alongside #[cfg(test)] mod tests; -> NOT exempt!
        dir.write("crates/path_alias/src/lib.rs", "#[cfg(test)]\nmod tests;\n#[path = \"tests.rs\"]\npub mod prod;\n");
        dir.write("crates/path_alias/src/tests.rs", "pub fn leak() {}\n");
        assert!(!super::is_cfg_test_only_submodule(dir.path(), "crates/path_alias/src/tests.rs"));

        // 4. Production #[path = "../tests/mod.rs"] from a sibling file loading a subdir helper -> NOT exempt!
        dir.write("crates/path_dir/src/lib.rs", "#[cfg(test)]\nmod tests;\npub mod other;\n");
        dir.write("crates/path_dir/src/other.rs", "#[path = \"tests/mod.rs\"]\npub mod sneaky;\n");
        dir.write("crates/path_dir/src/tests/mod.rs", "pub mod helper;\n");
        dir.write("crates/path_dir/src/tests/helper.rs", "pub fn leak() {}\n");
        assert!(!super::is_cfg_test_only_submodule(dir.path(), "crates/path_dir/src/tests/helper.rs"));

        // 5. A #[cfg(test)]-gated #[path = "tests.rs"] does NOT disqualify exemption.
        dir.write("crates/test_path/src/lib.rs", "#[cfg(test)]\nmod tests;\n#[cfg(test)]\n#[path = \"tests.rs\"]\nmod tests2;\n");
        dir.write("crates/test_path/src/tests.rs", "fn t() {}\n");
        assert!(super::is_cfg_test_only_submodule(dir.path(), "crates/test_path/src/tests.rs"));

        // 6. Round-3 #216: raw-ident `pub mod r#tests;` -> NOT exempt!
        dir.write("crates/raw_mod/src/lib.rs", "#[cfg(test)]\nmod tests;\n");
        dir.write("crates/raw_mod/src/main.rs", "pub mod r#tests;\nfn main() {}\n");
        dir.write("crates/raw_mod/src/tests.rs", "pub fn leak() {}\n");
        assert!(!super::is_cfg_test_only_submodule(dir.path(), "crates/raw_mod/src/tests.rs"));

        // 7. Round-3 #216: sibling submodule inside exempt dir when prod `#[path = "a/entry.rs"]` enters `a/` -> NOT exempt!
        dir.write("crates/sibling_dir/src/lib.rs", "#[cfg(test)]\nmod a;\n#[path = \"a/entry.rs\"]\npub mod prod;\n");
        dir.write("crates/sibling_dir/src/a/mod.rs", "pub mod entry;\npub mod sibling;\n");
        dir.write("crates/sibling_dir/src/a/entry.rs", "pub mod sibling;\n");
        dir.write("crates/sibling_dir/src/a/sibling.rs", "pub fn leak() {}\n");
        assert!(!super::is_cfg_test_only_submodule(dir.path(), "crates/sibling_dir/src/a/sibling.rs"));

        // 8. Round-3 #216: inline `mod sub { #[path = "../tests.rs"] pub mod prod; }` -> NOT exempt!
        dir.write("crates/inline_mod/src/lib.rs", "#[cfg(test)]\nmod tests;\npub mod sub { #[path = \"../tests.rs\"] pub mod prod; }\n");
        dir.write("crates/inline_mod/src/tests.rs", "pub fn leak() {}\n");
        assert!(!super::is_cfg_test_only_submodule(dir.path(), "crates/inline_mod/src/tests.rs"));
    }
}
