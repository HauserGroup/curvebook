//! `cargo xtask citations`: enforce `D1` rule 1 (`AGENTS.md` › Statistical Rigor & Verified
//! Oracles) across `crates/math`, `crates/stats`, `crates/fit`, and `crates/transform` (`R0.30`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::rust_lex::{
    count_newlines, find_char_literal_span, find_raw_string_span, is_cfg_test_only, is_cfg_test_only_submodule, is_ident_byte,
    skip_block_comment_end, skip_line_comment_end, skip_quoted_string_end,
};
use crate::scan_util::{format_error_list, is_real_dir, is_regular_file, read_dir_paths_sorted, rel_slash};

pub const NUMERICAL_SRC_DIRS: &[&str] = &["crates/math/src", "crates/stats/src", "crates/fit/src", "crates/transform/src"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllowlistedModule {
    pub path: &'static str,
    pub reason: &'static str,
}

const fn allow(path: &'static str, reason: &'static str) -> AllowlistedModule {
    AllowlistedModule { path, reason }
}

pub const ALLOWLIST: &[AllowlistedModule] = &[
    allow("crates/math/src/lib.rs", "Crate-root module declarations, re-exports, and elementary axis-scale bijections"),
    allow("crates/math/src/error.rs", "Structured MathError enum and Display/Error trait implementations only"),
    allow("crates/math/src/pvalue.rs", "ConfidenceLevel bounds wrapper and p-value/significance-star string formatting presets"),
    allow("crates/math/src/dist/mod.rs", "Probability-distribution submodule declarations and re-exports only"),
    allow("crates/math/src/linalg/mod.rs", "Linear-algebra submodule declarations and re-exports only"),
    allow("crates/math/src/linalg/matrix.rs", "Bounds-checked DenseMatrix storage delegating accumulation to cited sum module"),
    allow("crates/math/src/special/mod.rs", "Special-function submodule declarations and re-exports only"),
    allow("crates/stats/src/lib.rs", "Crate-root module declarations, re-exports, and AnalysisKind dispatch to cited submodules"),
    allow("crates/stats/src/error.rs", "Structured StatsError enum and Display/Error trait implementations only"),
    allow("crates/fit/src/lib.rs", "Crate-root module declarations and public re-exports only"),
    allow("crates/fit/src/error.rs", "Structured FitError, FitStatus, and ParameterStatus types without numerical routines"),
    allow("crates/fit/src/types.rs", "Regression input/output data structures, HookSource lookup, and sheet-to-series plumbing"),
    allow("crates/fit/src/tex_format.rs", "Canonical TeX-subset string formatting for equations and numeric coefficients"),
    allow("crates/transform/src/lib.rs", "Crate-root module declarations and public re-exports only"),
    allow("crates/transform/src/error.rs", "Structured TransformError enum and Display/Error trait implementations only"),
    allow("crates/transform/src/archetype_convert.rs", "Structural table-archetype conversion and cell/exclusion coordinate remapping"),
    allow("crates/transform/src/import.rs", "Four-stage delimited/linked cell import, range placement, and sheet-link cycle detection"),
    allow("crates/transform/src/plate_and_tidy.rs", "96/384-well microplate grid indexing, .cbtemplate JSON codec, and tidy pivot"),
    allow("crates/transform/src/formula/mod.rs", "Formula-engine submodule declarations and re-exports only"),
    allow("crates/transform/src/formula/parser.rs", "Formula lexer, AST types, and Pratt recursive-descent expression parser"),
    allow("crates/transform/src/formula/dag.rs", "Formula column-dependency extraction and topological DAG scheduling"),
    allow("crates/transform/src/formula/eval.rs", "Spreadsheet expression/window interpreter delegating math to cited curvebook_math"),
    allow("crates/transform/src/ops/mod.rs", "TransformContext hook lookup, ResultSheet wrapper, and TransformAnalysisSpec dispatcher"),
    allow("crates/transform/src/ops/reshape_select.rs", "Table transposition, row decimation/binning, and predicate row-selection plumbing"),
    allow("crates/transform/src/ops/normalize_baseline.rs", "Linear endpoint normalization and row/column baseline subtraction or ratio plumbing"),
    allow("crates/transform/src/ops/math.rs", "Cell-wise algebraic preset dispatch and first-order/endpoint spread propagation"),
    allow("crates/transform/src/simulate.rs", "Synthetic XY table scaffolding evaluating preset curves and DeterministicRng noise"),
];

const NON_AUTHOR_CAPITALIZED_WORDS: &str = "Algorithm Audit Chapter Column Columns Computes Default Equation Error Example Examples Figure Item Layer Maximum Minimum Mode Model Module Note Notes Option Output Part Phase Port Returns Row Rows Rule Rules Section See Sheet Stage Step Supports Table Tables Test Tests Value Values Version Wave";
const EXPLICIT_REFERENCE_MARKERS: &str = "doi:10.|doi.org/10.|dlmf.nist.gov|nist digital library of mathematical functions|nist strd|statistical reference datasets|itl.nist.gov/div898/strd|abramowitz & stegun|abramowitz and stegun";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CitationStats {
    pub total_modules: usize,
    pub allowlisted_modules: usize,
    pub private_only_modules: usize,
    pub cited_modules: usize,
    pub verified_public_routines: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CitationViolation {
    pub path: String,
    pub line: usize,
    pub detail: String,
}

impl std::fmt::Display for CitationViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.path, self.line, self.detail)
    }
}

fn push_cit(out: &mut Vec<CitationViolation>, path: impl Into<String>, line: usize, detail: impl Into<String>) {
    out.push(CitationViolation { path: path.into(), line, detail: detail.into() });
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("usage: cargo xtask citations (unexpected args: `{}`)", args.join(" ")));
    }
    let (stats, violations) = check_workspace_with_allowlist(root, NUMERICAL_SRC_DIRS, ALLOWLIST)?;
    if violations.is_empty() {
        println!(
            "citations: 0 violations ({} cited numerical module(s), {} public routine(s), {} allow-listed plumbing/error/re-export module(s), {} private-only module(s))",
            stats.cited_modules, stats.verified_public_routines, stats.allowlisted_modules, stats.private_only_modules
        );
        return Ok(());
    }
    Err(format_error_list("D1 citation violation(s) found", &violations))
}

pub fn check_workspace_with_allowlist(
    root: &Path,
    src_dirs: &[&str],
    allowlist: &[AllowlistedModule],
) -> Result<(CitationStats, Vec<CitationViolation>), String> {
    let (mut stats, mut violations, mut allow_map) = (CitationStats::default(), Vec::new(), BTreeMap::new());
    for entry in allowlist {
        let path = entry.path.trim();
        if path.is_empty() {
            push_cit(&mut violations, "<allowlist>", 1, "allow-list contains an empty module path");
            continue;
        }
        if entry.reason.trim().is_empty() {
            push_cit(&mut violations, path, 1, "allow-listed module must provide a non-empty stated reason");
        }
        if allow_map.insert(path, entry.reason).is_some() {
            push_cit(&mut violations, path, 1, "duplicate allow-list entry");
            continue;
        }
        let ok_dir = src_dirs.iter().any(|d| path.starts_with(d) && path.as_bytes().get(d.len()) == Some(&b'/'));
        if !ok_dir || !Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case("rs")) {
            push_cit(&mut violations, path, 1, format!("allow-listed path `{path}` is not a `.rs` module inside {src_dirs:?}"));
        } else if !is_regular_file(&root.join(path)) {
            push_cit(&mut violations, path, 1, format!("stale or symlinked allow-list entry: `{path}` is not a regular file on disk"));
        }
    }
    for &src_rel in src_dirs {
        let dir = root.join(src_rel);
        if !is_real_dir(&dir) {
            push_cit(&mut violations, src_rel, 1, "numerical crate source directory does not exist");
            continue;
        }
        let mut rs_files = Vec::new();
        collect_rs_files_fail_closed(root, &dir, &mut violations, &mut rs_files)?;
        rs_files.sort();
        if rs_files.is_empty() {
            push_cit(&mut violations, src_rel, 1, "numerical crate source directory contains zero `.rs` files");
            continue;
        }
        for rel in rs_files.into_iter().filter(|r| !is_cfg_test_only_submodule(root, r)) {
            stats.total_modules += 1;
            if allow_map.contains_key(rel.as_str()) {
                stats.allowlisted_modules += 1;
                continue;
            }
            let report = scan_module_source(&rel, &std::fs::read_to_string(root.join(&rel)).map_err(|e| format!("{rel}: {e}"))?);
            if report.public_routines == 0 {
                stats.private_only_modules += 1;
            } else if report.violations.is_empty() {
                (stats.cited_modules, stats.verified_public_routines) =
                    (stats.cited_modules + 1, stats.verified_public_routines + report.public_routines);
            } else {
                violations.extend(report.violations);
            }
        }
    }
    violations.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.line.cmp(&b.line)).then_with(|| a.detail.cmp(&b.detail)));
    Ok((stats, violations))
}

fn collect_rs_files_fail_closed(root: &Path, dir: &Path, violations: &mut Vec<CitationViolation>, out: &mut Vec<String>) -> Result<(), String> {
    for path in read_dir_paths_sorted(dir)? {
        let (meta, rel) = (std::fs::symlink_metadata(&path).map_err(|e| format!("stat {}: {e}", path.display()))?, rel_slash(root, &path)?);
        if meta.file_type().is_symlink() {
            push_cit(violations, rel, 1, "symlink is forbidden inside governed numerical crate source directory");
        } else if meta.is_dir() {
            collect_rs_files_fail_closed(root, &path, violations, out)?;
        } else if meta.is_file() && path.extension().is_some_and(|e| e == "rs") {
            out.push(rel);
        }
    }
    Ok(())
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ModuleScanReport {
    pub module_cited: bool,
    pub public_routines: usize,
    pub violations: Vec<CitationViolation>,
}

#[derive(Debug, Clone)]
enum Token {
    InnerDoc(String),
    OuterDoc(String),
    OuterAttr { is_cfg_test: bool },
    Ident { text: String, line: usize },
    Punct(u8),
    Other,
}

#[must_use]
pub fn scan_module_source(rel_path: &str, source: &str) -> ModuleScanReport {
    let tokens = lex_module(source);
    let module_doc: Vec<&str> = tokens.iter().map_while(|t| if let Token::InnerDoc(s) = t { Some(s.as_str()) } else { None }).collect();
    let mut report = ModuleScanReport { module_cited: cites_published_reference(&module_doc.join("\n")), public_routines: 0, violations: Vec::new() };
    parse_container_items(&tokens, 0, tokens.len(), rel_path, &mut report);
    report
}

fn parse_container_items(tokens: &[Token], mut idx: usize, end: usize, rel_path: &str, report: &mut ModuleScanReport) {
    let (mut pending_doc, mut cfg_test_item) = (String::new(), false);
    while idx < end {
        match &tokens[idx] {
            Token::InnerDoc(_) => idx += 1,
            Token::OuterDoc(doc) => {
                if !pending_doc.is_empty() {
                    pending_doc.push('\n');
                }
                pending_doc.push_str(doc);
                idx += 1;
            }
            Token::OuterAttr { is_cfg_test } => (cfg_test_item, idx) = (cfg_test_item | *is_cfg_test, idx + 1),
            Token::Punct(b';' | b'}') => (pending_doc.clear(), cfg_test_item = false, idx += 1).0,
            _ if cfg_test_item => (idx = skip_item(tokens, idx, end), pending_doc.clear(), cfg_test_item = false).1,
            _ => (idx = inspect_item(tokens, idx, end, &pending_doc, rel_path, report), pending_doc.clear()).1,
        }
    }
}

fn inspect_item(tokens: &[Token], idx: usize, end: usize, item_doc: &str, rel_path: &str, report: &mut ModuleScanReport) -> usize {
    let (is_bare_pub, kw_idx) = parse_visibility(tokens, idx, end);
    let Some(Token::Ident { text: kw, .. }) = tokens.get(kw_idx) else { return skip_item(tokens, idx, end) };
    if is_bare_pub && let Some((fn_name, fn_line, after)) = match_fn_signature(tokens, kw_idx, end) {
        report.public_routines += 1;
        if !report.module_cited && !cites_published_reference(item_doc) {
            push_cit(
                &mut report.violations,
                rel_path,
                fn_line,
                format!("public routine `{fn_name}` lacks a published D1 literature citation in its `///` doc comment or module `//!` header"),
            );
        }
        return skip_item(tokens, after, end);
    }
    if matches!(kw.as_str(), "mod" | "impl") {
        enter_or_skip_container(tokens, kw_idx + 1, end, kw == "impl", rel_path, report)
    } else {
        skip_item(tokens, idx, end)
    }
}

fn parse_visibility(tokens: &[Token], idx: usize, end: usize) -> (bool, usize) {
    if !matches!(tokens.get(idx), Some(Token::Ident { text, .. }) if text == "pub") {
        return (false, idx);
    }
    if matches!(tokens.get(idx + 1), Some(Token::Punct(b'('))) {
        let next = ((idx + 2)..end).find(|&cur| matches!(tokens.get(cur), Some(Token::Ident { text: w, .. }) if matches!(w.as_str(), "fn" | "const" | "async" | "unsafe" | "extern" | "struct" | "enum" | "union" | "type" | "trait" | "mod" | "use" | "static")));
        return (false, next.unwrap_or(end));
    }
    (true, idx + 1)
}

fn match_fn_signature(tokens: &[Token], mut idx: usize, end: usize) -> Option<(String, usize, usize)> {
    while let Some(Token::Ident { text, .. }) = tokens.get(idx).filter(|_| idx < end) {
        match text.as_str() {
            "const" | "async" | "unsafe" => idx += 1,
            "extern" => idx += 1 + usize::from(matches!(tokens.get(idx + 1), Some(Token::Other))),
            "fn" => {
                return match tokens.get(idx + 1) {
                    Some(Token::Ident { text: name, line }) => Some((name.clone(), *line, idx + 2)),
                    _ => None,
                };
            }
            _ => return None,
        }
    }
    None
}

fn enter_or_skip_container(
    tokens: &[Token],
    mut idx: usize,
    end: usize,
    reject_trait_for: bool,
    rel_path: &str,
    report: &mut ModuleScanReport,
) -> usize {
    let (mut saw_for, mut in_where, mut p, mut b, mut a) = (false, false, 0usize, 0usize, 0usize);
    while idx < end {
        let flat = p == 0 && b == 0 && a == 0;
        match &tokens[idx] {
            Token::Punct(b';') if p == 0 && b == 0 => return idx + 1,
            Token::Punct(b'(') => p += 1,
            Token::Punct(b')') => p = p.saturating_sub(1),
            Token::Punct(b'[') => b += 1,
            Token::Punct(b']') => b = b.saturating_sub(1),
            Token::Punct(b'<') => a += 1,
            Token::Punct(b'>') => a = a.saturating_sub(1),
            Token::Ident { text, .. } if reject_trait_for && flat && text == "where" => in_where = true,
            Token::Ident { text, .. } if reject_trait_for && flat && !in_where && text == "for" => saw_for = true,
            Token::Punct(b'{') => {
                let close = find_matching_rbrace(tokens, idx, end);
                if flat {
                    if !saw_for {
                        parse_container_items(tokens, idx + 1, close, rel_path, report);
                    }
                    return (close + 1).min(end);
                }
                idx = (close + 1).min(end);
                continue;
            }
            _ => {}
        }
        idx += 1;
    }
    end
}

fn skip_item(tokens: &[Token], mut idx: usize, end: usize) -> usize {
    let (mut p, mut b) = (0usize, 0usize);
    while idx < end {
        match tokens[idx] {
            Token::Punct(b'(') => p += 1,
            Token::Punct(b')') => p = p.saturating_sub(1),
            Token::Punct(b'[') => b += 1,
            Token::Punct(b']') => b = b.saturating_sub(1),
            Token::Punct(b';') if p == 0 && b == 0 => return idx + 1,
            Token::Punct(b'{') => {
                let close = find_matching_rbrace(tokens, idx, end);
                if p == 0 && b == 0 && !matches!(tokens.get(close + 1), Some(Token::Punct(b'>'))) {
                    return (close + 1).min(end);
                }
                idx = (close + 1).min(end);
                continue;
            }
            _ => {}
        }
        idx += 1;
    }
    end
}

fn find_matching_rbrace(tokens: &[Token], open_idx: usize, end: usize) -> usize {
    let (mut depth, mut cur) = (0usize, open_idx);
    while cur < end {
        match tokens[cur] {
            Token::Punct(b'{') => depth += 1,
            Token::Punct(b'}') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return cur;
                }
            }
            _ => {}
        }
        cur += 1;
    }
    end
}

#[must_use]
pub fn cites_published_reference(doc: &str) -> bool {
    if doc.trim().is_empty() {
        return false;
    }
    let lower = doc.to_ascii_lowercase();
    EXPLICIT_REFERENCE_MARKERS.split('|').any(|m| lower.contains(m))
        || lower.match_indices("isbn").any(|(pos, _)| {
            (pos == 0 || !lower.as_bytes()[pos - 1].is_ascii_alphanumeric())
                && lower.get(pos + 4..).is_some_and(|r| {
                    r.strip_prefix("-13")
                        .or_else(|| r.strip_prefix("-10"))
                        .unwrap_or(r)
                        .trim_start_matches([' ', ':', '-'])
                        .as_bytes()
                        .first()
                        .is_some_and(u8::is_ascii_digit)
                })
        })
        || contains_author_year_citation(doc)
}

fn contains_author_year_citation(doc: &str) -> bool {
    let bytes = doc.as_bytes();
    if bytes.len() < 4 {
        return false;
    }
    let disallowed: BTreeSet<&str> = NON_AUTHOR_CAPITALIZED_WORDS.split_ascii_whitespace().collect();
    for idx in 0..=bytes.len() - 4 {
        if !is_publication_year_at(bytes, idx) {
            continue;
        }
        let mut ws = idx.saturating_sub(56);
        while ws > 0 && !doc.is_char_boundary(ws) {
            ws -= 1;
        }
        let Some(mut prefix) = doc.get(ws..idx) else { continue };
        if let Some((_, after)) = prefix.rsplit_once(['<', '>', '=', '[', ']', '`', '\n']) {
            prefix = after;
        }
        let trimmed = prefix.trim_end();
        let next_after = bytes.get(idx + 4 + usize::from(bytes.get(idx + 4).is_some_and(|b| matches!(b, b'a'..=b'd')))).copied();
        if trimmed.strip_suffix('(').or_else(|| trimmed.strip_suffix(',')).is_some_and(|p| author_clause_ends_with_surname(p, &disallowed))
            || (matches!(next_after, Some(b')' | b';' | b',' | b']'))
                && trimmed.rsplit_once(['(', ';']).is_some_and(|(_, inside)| author_clause_ends_with_surname(inside, &disallowed)))
            || (matches!(next_after, None | Some(b')' | b';' | b',' | b'.' | b':' | b']' | b'\n' | b'\r'))
                && (trimmed.ends_with("et al.") || trimmed.ends_with("et al") || trimmed.contains('&'))
                && author_clause_ends_with_surname(trimmed, &disallowed))
        {
            return true;
        }
    }
    false
}

fn author_clause_ends_with_surname(clause: &str, disallowed: &BTreeSet<&str>) -> bool {
    const CONN: &str = "and et al de van von di la le del der den ed eds";
    let tail = clause.rsplit_once(['.', '!', '?', ':']).map_or(clause, |(h, a)| {
        if a.trim().is_empty() || h.trim_end().ends_with("al") || h.trim_end().chars().last().is_some_and(char::is_uppercase) { clause } else { a }
    });
    for w in tail.split(|c: char| !(c.is_alphabetic() || matches!(c, '\'' | '\u{2019}' | '-' | '&'))).filter(|w| !w.is_empty()).rev() {
        let wl = w.to_ascii_lowercase();
        if w == "&" || CONN.split_ascii_whitespace().any(|c| c == wl) {
            continue;
        }
        let t = w.trim_matches(['\'', '\u{2019}', '-']);
        if t.len() == 1 && t.chars().all(char::is_uppercase) {
            continue;
        }
        let mut chars = t.chars();
        return chars.next().is_some_and(char::is_uppercase) && chars.any(char::is_lowercase) && !disallowed.contains(t);
    }
    false
}

fn is_publication_year_at(bytes: &[u8], idx: usize) -> bool {
    let Some(s) = bytes.get(idx..idx + 4).filter(|s| s.iter().all(u8::is_ascii_digit)) else { return false };
    let year = usize::from(s[0] - b'0') * 1000 + usize::from(s[1] - b'0') * 100 + usize::from(s[2] - b'0') * 10 + usize::from(s[3] - b'0');
    if !(1700..=2035).contains(&year)
        || (idx > 0
            && (bytes[idx - 1].is_ascii_alphanumeric() || matches!(bytes[idx - 1], b'<' | b'>' | b'=' | b'+' | b'-' | b'*' | b'/' | b'.' | b'_')))
    {
        return false;
    }
    bytes.get(idx + 4).is_none_or(|&next| {
        !(next.is_ascii_digit()
            || matches!(next, b'.' | b'_')
            || (next.is_ascii_alphabetic() && !(matches!(next, b'a'..=b'd') && !bytes.get(idx + 5).is_some_and(u8::is_ascii_alphanumeric)))
            || (next == b'-'
                && bytes.get(idx + 5).is_some_and(u8::is_ascii_digit)
                && bytes.get(idx + 6).is_some_and(u8::is_ascii_digit)
                && !bytes.get(idx + 7).is_some_and(u8::is_ascii_digit)))
    })
}

fn skip_string_or_char(source: &str, bytes: &[u8], i: usize, line: &mut usize) -> Option<usize> {
    if let Some((_, n, _)) = find_raw_string_span(bytes, i) {
        *line += count_newlines(&bytes[i..n]);
        Some(n)
    } else if bytes[i] == b'"' {
        let n = skip_quoted_string_end(bytes, i);
        *line += count_newlines(&bytes[i..n]);
        Some(n)
    } else if bytes[i] == b'\'' {
        Some(find_char_literal_span(source, i).map_or(i + 1, |(_, eq)| eq + 1))
    } else {
        None
    }
}

fn lex_module(source: &str) -> Vec<Token> {
    let (bytes, mut tokens, mut i, mut line) = (source.as_bytes(), Vec::new(), 0usize, 1usize);
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            (line, i) = (line + usize::from(b == b'\n'), i + 1);
        } else if bytes.get(i..i + 2) == Some(b"//") {
            let end = skip_line_comment_end(bytes, i);
            let slice = source.get(i..end).unwrap_or("");
            if let Some(inner) = slice.strip_prefix("//!") {
                tokens.push(Token::InnerDoc(inner.trim().to_owned()));
            } else if let Some(outer) = slice.strip_prefix("///").filter(|s| !s.starts_with('/')) {
                tokens.push(Token::OuterDoc(outer.trim().to_owned()));
            }
            i = end;
        } else if bytes.get(i..i + 2) == Some(b"/*") {
            let (is_inner, is_outer) =
                (bytes.get(i + 2) == Some(&b'!'), bytes.get(i + 2) == Some(&b'*') && !matches!(bytes.get(i + 3), Some(b'*' | b'/')));
            let end = skip_block_comment_end(bytes, i);
            line += count_newlines(&bytes[i..end]);
            if (is_inner || is_outer)
                && let Some(body) = source.get(i + 3..end.saturating_sub(2))
            {
                tokens.push(if is_inner { Token::InnerDoc(body.trim().to_owned()) } else { Token::OuterDoc(body.trim().to_owned()) });
            }
            i = end;
        } else if b == b'#' && (bytes.get(i + 1) == Some(&b'[') || bytes.get(i + 1..i + 3) == Some(b"![")) {
            i = lex_attribute(source, i, &mut line, &mut tokens);
        } else if let Some(n) = skip_string_or_char(source, bytes, i, &mut line) {
            if b != b'\'' {
                tokens.push(Token::Other);
            }
            i = n;
        } else if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            while i < bytes.len() && is_ident_byte(bytes[i]) {
                i += 1;
            }
            tokens.push(Token::Ident { text: source.get(start..i).unwrap_or("").to_owned(), line });
        } else if matches!((b, bytes.get(i + 1).copied()), (b'-' | b'=', Some(b'>')) | (b'<' | b'>', Some(b'='))) {
            tokens.push(Token::Other);
            i += 2;
        } else {
            if matches!(b, b'(' | b')' | b'[' | b']' | b'<' | b'>' | b'{' | b'}' | b';') {
                tokens.push(Token::Punct(b));
            }
            i += source.get(i..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
        }
    }
    tokens
}

fn lex_attribute(source: &str, start: usize, line: &mut usize, tokens: &mut Vec<Token>) -> usize {
    let bytes = source.as_bytes();
    let is_inner = bytes.get(start + 1) == Some(&b'!');
    let open = start + 1 + usize::from(is_inner);
    let (mut depth, mut i) = (0usize, open);
    while i < bytes.len() {
        let b = bytes[i];
        if bytes.get(i..i + 2) == Some(b"//") {
            i = skip_line_comment_end(bytes, i);
        } else if bytes.get(i..i + 2) == Some(b"/*") {
            let n = skip_block_comment_end(bytes, i);
            (*line, i) = (*line + count_newlines(&bytes[i..n]), n);
        } else if let Some(n) = skip_string_or_char(source, bytes, i, line) {
            i = n;
        } else {
            (*line, i) = (*line + usize::from(b == b'\n'), i + 1);
            if b == b'[' {
                depth += 1;
            } else if b == b']' {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
        }
    }
    let inside = source.get(open + 1..i.saturating_sub(1)).unwrap_or_default().trim();
    if let Some(doc_val) = inside.strip_prefix("doc").and_then(|r| r.trim_start().strip_prefix('=')).and_then(|rhs| {
        let rhs = rhs.trim();
        rhs.strip_prefix('"').and_then(|s| s.strip_suffix('"')).map(str::to_owned).or_else(|| {
            let (cs, end_idx, h) = find_raw_string_span(rhs.as_bytes(), 0)?;
            (end_idx == rhs.len()).then(|| rhs.get(cs..end_idx.saturating_sub(1 + h)).map(str::to_owned)).flatten()
        })
    }) {
        tokens.push(if is_inner { Token::InnerDoc(doc_val) } else { Token::OuterDoc(doc_val) });
    } else if !is_inner {
        let compact: String = inside.chars().filter(|c| !c.is_whitespace()).collect();
        tokens.push(Token::OuterAttr { is_cfg_test: is_cfg_test_only(&compact) });
    }
    i
}

#[cfg(test)]
mod tests {
    use super::{ALLOWLIST, AllowlistedModule, NUMERICAL_SRC_DIRS, check_workspace_with_allowlist, cites_published_reference, scan_module_source};
    use crate::scan_util::test_support::TestDir;

    #[test]
    fn citation_detector_and_module_scanner_rules() {
        for ok in [
            "Student (1908).",
            "Motulsky & Brown 2006).",
            "doi:10.1080/00031305.1996.10473566",
            "NIST Digital Library of Mathematical Functions",
            "Abramowitz & Stegun",
            "ISBN 978-0-521-88068-8",
        ] {
            assert!(cites_published_reference(ok), "{ok}");
        }
        for bad in ["", "Audit 2026-11-18: n <= 2000.", "Table (2000) rows", "Gaussian window of 2000 samples.", "CurveBook 2026 notes."] {
            assert!(!cites_published_reference(bad), "{bad}");
        }
        let cited_mod = scan_module_source("crates/math/src/r.rs", "//! Brent, R. P. (1973).\npub fn root() -> f64 { 0.0 }\n");
        assert!(cited_mod.module_cited && cited_mod.public_routines == 1 && cited_mod.violations.is_empty());

        let mixed = scan_module_source(
            "crates/stats/src/x.rs",
            "/// Zar (2010).\npub fn ok() {}\npub unsafe fn bad() {}\n#[cfg(test)] mod tests { //! Brent (1973).\n pub fn t() {} }\nimpl<T> E<T> where for<'a> &'a T: Clone { pub fn hrtb() {} }\nimpl Solver<[f64; { 4 }]> { pub fn cg() {} }\n#[doc = r#\"[0,1)\"#] pub fn raw_bad() {}\n#[doc = r#\"Brent (1973)\"#] pub fn raw_ok() {}\n",
        );
        assert!(!mixed.module_cited && mixed.public_routines == 6 && mixed.violations.len() == 4, "{mixed:?}");

        let ws = TestDir::new("citations-allow");
        for d in NUMERICAL_SRC_DIRS {
            std::fs::create_dir_all(ws.join(d)).unwrap();
        }
        for (p, b) in [
            ("crates/math/src/lib.rs", "pub fn a() {}\n"),
            ("crates/stats/src/a.rs", "//! Zar (2010).\npub fn f() {}\n"),
            ("crates/fit/src/a.rs", "//! Marquardt (1963).\npub fn f() {}\n"),
            ("crates/transform/src/a.rs", "//! Savitzky & Golay (1964).\npub fn f() {}\n"),
        ] {
            ws.write(p, b);
        }
        assert!(
            check_workspace_with_allowlist(
                ws.path(),
                NUMERICAL_SRC_DIRS,
                &[AllowlistedModule { path: "crates/math/src/lib.rs", reason: "plumbing" }]
            )
            .unwrap()
            .1
            .is_empty()
        );
        assert!(
            !check_workspace_with_allowlist(
                ws.path(),
                NUMERICAL_SRC_DIRS,
                &[AllowlistedModule { path: "crates/math/src/ghost.rs", reason: "stale" }]
            )
            .unwrap()
            .1
            .is_empty()
        );

        let (stats, violations) = check_workspace_with_allowlist(&crate::workspace_root().unwrap(), NUMERICAL_SRC_DIRS, ALLOWLIST).unwrap();
        assert!(violations.is_empty() && stats.cited_modules >= 40, "{stats:?} {violations:?}");
    }
}
