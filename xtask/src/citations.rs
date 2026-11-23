//! `cargo xtask citations`: enforce `D1` rule 1 (`AGENTS.md` › Statistical Rigor & Verified
//! Oracles) across `crates/math`, `crates/stats`, `crates/fit`, and `crates/transform` (`R0.30`).
//!
//! Every non-test `.rs` module in the four numerical crates that defines a public routine
//! (`pub fn`, including `pub const fn` / `pub async fn` and inherent `impl` methods) must cite a
//! published reference (peer-reviewed paper, textbook, DOI, ISBN, NIST DLMF, Abramowitz & Stegun,
//! or NIST StRD specification) in either its module-level `//!` header or each public routine's
//! `///` documentation. Modules that hold only error types, crate/module re-exports, formatting,
//! expression syntax/DAG plumbing, or structural table-reshaping helpers are explicitly
//! allow-listed with a stated reason, and stale/nonexistent allow-list paths fail closed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Crate source directories governed by the `D1` rule-1 literature-citation gate.
pub const NUMERICAL_SRC_DIRS: &[&str] = &["crates/math/src", "crates/stats/src", "crates/fit/src", "crates/transform/src"];

/// Explicit exemption entry for a module in [`NUMERICAL_SRC_DIRS`] that contains only error types,
/// re-exports, data structures, formatting, parsers, or structural table plumbing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllowlistedModule {
    /// Slash-normalized workspace-relative `.rs` path.
    pub path: &'static str,
    /// Human-readable reason explaining why this module requires no `D1` numerical citation.
    pub reason: &'static str,
}

/// Canonical allow-list for non-numerical modules in `crates/{math,stats,fit,transform}/src`.
pub const ALLOWLIST: &[AllowlistedModule] = &[
    // curvebook-math
    AllowlistedModule {
        path: "crates/math/src/lib.rs",
        reason: "Crate-root module declarations, re-exports, and elementary chart axis-scale domain bijections (delegating probit quantiles to cited dist::norm_ppf)",
    },
    AllowlistedModule { path: "crates/math/src/error.rs", reason: "Structured MathError enum and Display/Error trait implementations only" },
    AllowlistedModule {
        path: "crates/math/src/pvalue.rs",
        reason: "ConfidenceLevel bounds wrapper and p-value/significance-star string formatting presets",
    },
    AllowlistedModule { path: "crates/math/src/dist/mod.rs", reason: "Probability-distribution submodule declarations and re-exports only" },
    AllowlistedModule { path: "crates/math/src/linalg/mod.rs", reason: "Linear-algebra submodule declarations and re-exports only" },
    AllowlistedModule {
        path: "crates/math/src/linalg/matrix.rs",
        reason: "Bounds-checked DenseMatrix storage container and basic matrix operations delegating compensated accumulation to cited sum module",
    },
    AllowlistedModule { path: "crates/math/src/special/mod.rs", reason: "Special-function submodule declarations and re-exports only" },
    // curvebook-stats
    AllowlistedModule {
        path: "crates/stats/src/lib.rs",
        reason: "Crate-root module declarations, re-exports, and top-level AnalysisKind dispatch to cited statistical submodules",
    },
    AllowlistedModule { path: "crates/stats/src/error.rs", reason: "Structured StatsError enum and Display/Error trait implementations only" },
    // curvebook-fit
    AllowlistedModule { path: "crates/fit/src/lib.rs", reason: "Crate-root module declarations and public re-exports only" },
    AllowlistedModule {
        path: "crates/fit/src/error.rs",
        reason: "Structured FitError, FitStatus, and ParameterStatus types without numerical routines",
    },
    AllowlistedModule {
        path: "crates/fit/src/types.rs",
        reason: "Regression input/output data structures, HookSource lookup, and sheet-to-series extraction plumbing",
    },
    AllowlistedModule {
        path: "crates/fit/src/tex_format.rs",
        reason: "Canonical TeX-subset string formatting for equations and numeric coefficients",
    },
    // curvebook-transform
    AllowlistedModule { path: "crates/transform/src/lib.rs", reason: "Crate-root module declarations and public re-exports only" },
    AllowlistedModule {
        path: "crates/transform/src/error.rs",
        reason: "Structured TransformError enum and Display/Error trait implementations only",
    },
    AllowlistedModule {
        path: "crates/transform/src/archetype_convert.rs",
        reason: "Structural table-archetype conversion and cell/exclusion coordinate remapping",
    },
    AllowlistedModule {
        path: "crates/transform/src/import.rs",
        reason: "Four-stage delimited/linked cell import, range placement, and sheet-link cycle detection",
    },
    AllowlistedModule {
        path: "crates/transform/src/plate_and_tidy.rs",
        reason: "96/384-well microplate grid indexing, .cbtemplate JSON codec, and tidy long-to-wide table pivot (delegating moments to cited curvebook_math::sum)",
    },
    AllowlistedModule { path: "crates/transform/src/formula/mod.rs", reason: "Formula-engine submodule declarations and re-exports only" },
    AllowlistedModule {
        path: "crates/transform/src/formula/parser.rs",
        reason: "Formula lexer, AST types, and Pratt recursive-descent expression parser",
    },
    AllowlistedModule { path: "crates/transform/src/formula/dag.rs", reason: "Formula column-dependency extraction and topological DAG scheduling" },
    AllowlistedModule {
        path: "crates/transform/src/formula/eval.rs",
        reason: "Spreadsheet expression and window-clause interpreter delegating special and statistical functions to cited curvebook_math modules",
    },
    AllowlistedModule {
        path: "crates/transform/src/ops/mod.rs",
        reason: "TransformContext hook lookup, ResultSheet wrapper, and TransformAnalysisSpec dispatcher",
    },
    AllowlistedModule {
        path: "crates/transform/src/ops/reshape_select.rs",
        reason: "Table transposition, row decimation/binning, and predicate row-selection plumbing (delegating sums to cited curvebook_math::sum)",
    },
    AllowlistedModule {
        path: "crates/transform/src/ops/normalize_baseline.rs",
        reason: "Linear endpoint normalization and row/column baseline subtraction or ratio plumbing (delegating means to cited curvebook_math::sum)",
    },
    AllowlistedModule {
        path: "crates/transform/src/ops/math.rs",
        reason: "Cell-wise elementary algebraic preset dispatch and first-order/endpoint spread propagation (delegating quantiles to cited curvebook_math::dist)",
    },
    AllowlistedModule {
        path: "crates/transform/src/simulate.rs",
        reason: "Synthetic XY table scaffolding evaluating preset curves and sampling noise via cited curvebook_math::DeterministicRng",
    },
];

/// Non-author capitalized words commonly found near integers in Rust doc comments; excluded from
/// surname matching so operational bounds (e.g. `Table (2000) rows` or `Audit 2026-11-18`) never
/// masquerade as `Author (Year)` citations.
const NON_AUTHOR_CAPITALIZED_WORDS: &[&str] = &[
    "Algorithm",
    "Audit",
    "Chapter",
    "Column",
    "Columns",
    "Computes",
    "Default",
    "Equation",
    "Error",
    "Example",
    "Examples",
    "Figure",
    "Item",
    "Layer",
    "Maximum",
    "Minimum",
    "Mode",
    "Model",
    "Module",
    "Note",
    "Notes",
    "Option",
    "Output",
    "Part",
    "Phase",
    "Port",
    "Returns",
    "Row",
    "Rows",
    "Rule",
    "Rules",
    "Section",
    "See",
    "Sheet",
    "Stage",
    "Step",
    "Supports",
    "Table",
    "Tables",
    "Test",
    "Tests",
    "Value",
    "Values",
    "Version",
    "Wave",
];

/// Direct reference corpus / identifier markers that satisfy `D1` rule 1 even when no 4-digit year
/// appears in the same sentence.
const EXPLICIT_REFERENCE_MARKERS: &[&str] = &[
    "doi:10.",
    "doi.org/10.",
    "dlmf.nist.gov",
    "nist digital library of mathematical functions",
    "nist strd",
    "statistical reference datasets",
    "itl.nist.gov/div898/strd",
    "abramowitz & stegun",
    "abramowitz and stegun",
];

/// Summary counts returned by [`check_workspace_with_allowlist`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CitationStats {
    /// Total `.rs` modules discovered under `src_dirs`.
    pub total_modules: usize,
    /// Modules covered by explicit allow-list entries.
    pub allowlisted_modules: usize,
    /// Non-allow-listed modules with no public routines (`pub fn`).
    pub private_only_modules: usize,
    /// Non-allow-listed modules with public routines verified to carry citations.
    pub cited_modules: usize,
    /// Public routines verified across non-allow-listed modules.
    pub verified_public_routines: usize,
}

/// A single citation-policy failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CitationViolation {
    /// Workspace-relative file path.
    pub path: String,
    /// 1-based source line number (`1` for file- or allow-list-level errors).
    pub line: usize,
    /// Human-readable diagnostic message.
    pub detail: String,
}

impl std::fmt::Display for CitationViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.path, self.line, self.detail)
    }
}

/// Execute `cargo xtask citations`.
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
    let formatted: Vec<String> = violations.iter().map(ToString::to_string).collect();
    Err(format!("{} D1 citation violation(s) found:\n  {}", violations.len(), formatted.join("\n  ")))
}

/// Validate `allowlist` and scan every `.rs` module under `src_dirs` inside `root`.
pub fn check_workspace_with_allowlist(
    root: &Path,
    src_dirs: &[&str],
    allowlist: &[AllowlistedModule],
) -> Result<(CitationStats, Vec<CitationViolation>), String> {
    let mut stats = CitationStats::default();
    let mut violations = Vec::new();

    let allow_map = validate_allowlist(root, src_dirs, allowlist, &mut violations);

    for &src_rel in src_dirs {
        let dir = root.join(src_rel);
        if !std::fs::symlink_metadata(&dir).is_ok_and(|m| !m.file_type().is_symlink() && m.file_type().is_dir()) {
            violations.push(CitationViolation {
                path: src_rel.to_owned(),
                line: 1,
                detail: String::from("numerical crate source directory does not exist"),
            });
            continue;
        }
        let mut rs_files = Vec::new();
        collect_rs_files(root, &dir, &mut rs_files)?;
        rs_files.sort();
        if rs_files.is_empty() {
            violations.push(CitationViolation {
                path: src_rel.to_owned(),
                line: 1,
                detail: String::from("numerical crate source directory contains zero `.rs` files"),
            });
            continue;
        }
        for rel in rs_files {
            stats.total_modules += 1;
            if allow_map.contains_key(rel.as_str()) {
                stats.allowlisted_modules += 1;
                continue;
            }
            let text = std::fs::read_to_string(root.join(&rel)).map_err(|e| format!("{rel}: {e}"))?;
            let report = scan_module_source(&rel, &text);
            if report.public_routines == 0 {
                stats.private_only_modules += 1;
            } else if report.violations.is_empty() {
                stats.cited_modules += 1;
                stats.verified_public_routines += report.public_routines;
            } else {
                violations.extend(report.violations);
            }
        }
    }

    violations.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.line.cmp(&b.line)).then_with(|| a.detail.cmp(&b.detail)));
    Ok((stats, violations))
}

fn validate_allowlist<'a>(
    root: &Path,
    src_dirs: &[&str],
    allowlist: &'a [AllowlistedModule],
    violations: &mut Vec<CitationViolation>,
) -> BTreeMap<&'a str, &'a str> {
    let mut map = BTreeMap::new();
    for entry in allowlist {
        let path = entry.path.trim();
        if path.is_empty() {
            violations.push(CitationViolation {
                path: String::from("<allowlist>"),
                line: 1,
                detail: String::from("allow-list contains an empty module path"),
            });
            continue;
        }
        if entry.reason.trim().is_empty() {
            violations.push(CitationViolation {
                path: path.to_owned(),
                line: 1,
                detail: String::from("allow-listed module must provide a non-empty stated reason"),
            });
        }
        if map.insert(path, entry.reason).is_some() {
            violations.push(CitationViolation { path: path.to_owned(), line: 1, detail: String::from("duplicate allow-list entry") });
            continue;
        }
        let in_governed_dir = src_dirs.iter().any(|dir| path.starts_with(dir) && path.as_bytes().get(dir.len()) == Some(&b'/'));
        let has_rs_ext = Path::new(path).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("rs"));
        if !in_governed_dir || !has_rs_ext {
            violations.push(CitationViolation {
                path: path.to_owned(),
                line: 1,
                detail: format!("allow-listed path `{path}` is not a `.rs` module inside {:?}", src_dirs),
            });
            continue;
        }
        if !root.join(path).is_file() {
            violations.push(CitationViolation {
                path: path.to_owned(),
                line: 1,
                detail: format!("stale allow-list entry: `{path}` does not exist on disk"),
            });
        }
    }
    map
}

fn collect_rs_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        paths.push(entry.path());
    }
    paths.sort();
    for path in paths {
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if meta.is_dir() {
            if name == "tests" {
                continue;
            }
            collect_rs_files(root, &path, out)?;
        } else if meta.is_file() && path.extension().and_then(|e| e.to_str()) == Some("rs") {
            if name == "tests.rs" {
                continue;
            }
            let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            out.push(rel);
        }
    }
    Ok(())
}

/// Per-module scan outcome produced by [`scan_module_source`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ModuleScanReport {
    /// Whether the module's `//!` / `/*!` / `#![doc = ...]` header cites a published reference.
    pub module_cited: bool,
    /// Number of non-test public routines (`pub fn`) discovered in this module.
    pub public_routines: usize,
    /// Violations for public routines missing both module- and routine-level citations.
    pub violations: Vec<CitationViolation>,
}

#[derive(Debug, Clone)]
enum Token {
    InnerDoc(String),
    OuterDoc(String),
    OuterAttr { is_cfg_test: bool },
    Ident { text: String, line: usize },
    LParen,
    RParen,
    LBracket,
    RBracket,
    LAngle,
    RAngle,
    LBrace,
    RBrace,
    Semi,
    Other,
}

/// Scan a single Rust module's source text for non-test `pub fn` routines and `D1` citations.
#[must_use]
pub fn scan_module_source(rel_path: &str, source: &str) -> ModuleScanReport {
    let tokens = lex_module(source);
    let mut module_doc = String::new();
    for tok in &tokens {
        if let Token::InnerDoc(line) = tok {
            if !module_doc.is_empty() {
                module_doc.push('\n');
            }
            module_doc.push_str(line);
        }
    }
    let module_cited = cites_published_reference(&module_doc);
    let mut report = ModuleScanReport { module_cited, public_routines: 0, violations: Vec::new() };
    parse_container_items(&tokens, 0, tokens.len(), rel_path, &mut report);
    report
}

fn parse_container_items(tokens: &[Token], mut idx: usize, end: usize, rel_path: &str, report: &mut ModuleScanReport) {
    let mut pending_doc = String::new();
    let mut cfg_test_item = false;

    while idx < end {
        match &tokens[idx] {
            Token::InnerDoc(_) => {
                idx += 1;
            }
            Token::OuterDoc(doc) => {
                if !pending_doc.is_empty() {
                    pending_doc.push('\n');
                }
                pending_doc.push_str(doc);
                idx += 1;
            }
            Token::OuterAttr { is_cfg_test } => {
                if *is_cfg_test {
                    cfg_test_item = true;
                }
                idx += 1;
            }
            Token::Semi | Token::RBrace => {
                pending_doc.clear();
                cfg_test_item = false;
                idx += 1;
            }
            _ => {
                if cfg_test_item {
                    idx = skip_item(tokens, idx, end);
                    pending_doc.clear();
                    cfg_test_item = false;
                    continue;
                }
                idx = inspect_item(tokens, idx, end, &pending_doc, rel_path, report);
                pending_doc.clear();
            }
        }
    }
}

fn inspect_item(tokens: &[Token], idx: usize, end: usize, item_doc: &str, rel_path: &str, report: &mut ModuleScanReport) -> usize {
    let (is_bare_pub, kw_idx) = parse_visibility(tokens, idx, end);
    let Some(Token::Ident { text: kw, .. }) = tokens.get(kw_idx) else {
        return skip_item(tokens, idx, end);
    };

    if is_bare_pub && let Some((fn_name, fn_line, after_name)) = match_fn_signature(tokens, kw_idx, end) {
        report.public_routines += 1;
        if !report.module_cited && !cites_published_reference(item_doc) {
            report.violations.push(CitationViolation {
                path: rel_path.to_owned(),
                line: fn_line,
                detail: format!(
                    "public routine `{fn_name}` lacks a published D1 literature citation in its `///` doc comment or module `//!` header"
                ),
            });
        }
        return skip_item(tokens, after_name, end);
    }

    if kw == "mod" {
        return enter_or_skip_container(tokens, kw_idx + 1, end, false, rel_path, report);
    }
    if kw == "impl" {
        return enter_or_skip_container(tokens, kw_idx + 1, end, true, rel_path, report);
    }

    skip_item(tokens, idx, end)
}

fn parse_visibility(tokens: &[Token], idx: usize, end: usize) -> (bool, usize) {
    let Some(Token::Ident { text, .. }) = tokens.get(idx) else {
        return (false, idx);
    };
    if text != "pub" {
        return (false, idx);
    }
    if matches!(tokens.get(idx + 1), Some(Token::LParen)) {
        // Restricted visibility like `pub(crate)` or `pub(super)`: advance past `(...)`.
        let mut cursor = idx + 2;
        while cursor < end {
            if matches!(tokens.get(cursor), Some(Token::Ident { .. } | Token::OuterAttr { .. })) {
                // `LParen` isn't paired with `RParen` in `Token`; scan until the item keyword.
            }
            if let Some(Token::Ident { text: w, .. }) = tokens.get(cursor)
                && matches!(
                    w.as_str(),
                    "fn" | "const" | "async" | "extern" | "struct" | "enum" | "union" | "type" | "trait" | "mod" | "use" | "static"
                )
            {
                return (false, cursor);
            }
            cursor += 1;
        }
        return (false, end);
    }
    (true, idx + 1)
}

fn match_fn_signature(tokens: &[Token], mut idx: usize, end: usize) -> Option<(String, usize, usize)> {
    while idx < end {
        let Token::Ident { text, .. } = &tokens[idx] else {
            break;
        };
        match text.as_str() {
            "const" | "async" => idx += 1,
            "extern" => {
                idx += 1;
                if matches!(tokens.get(idx), Some(Token::Other)) {
                    idx += 1;
                }
            }
            "fn" => {
                if let Some(Token::Ident { text: name, line }) = tokens.get(idx + 1) {
                    return Some((name.clone(), *line, idx + 2));
                }
                return None;
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
    let mut saw_for = false;
    let mut in_where = false;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut angle_depth = 0usize;
    while idx < end {
        match &tokens[idx] {
            Token::Semi if paren_depth == 0 && bracket_depth == 0 => return idx + 1,
            Token::LParen => {
                paren_depth += 1;
                idx += 1;
            }
            Token::RParen => {
                paren_depth = paren_depth.saturating_sub(1);
                idx += 1;
            }
            Token::LBracket => {
                bracket_depth += 1;
                idx += 1;
            }
            Token::RBracket => {
                bracket_depth = bracket_depth.saturating_sub(1);
                idx += 1;
            }
            Token::LAngle => {
                angle_depth += 1;
                idx += 1;
            }
            Token::RAngle => {
                angle_depth = angle_depth.saturating_sub(1);
                idx += 1;
            }
            Token::Ident { text, .. } if reject_trait_for && text == "where" && paren_depth == 0 && bracket_depth == 0 && angle_depth == 0 => {
                in_where = true;
                idx += 1;
            }
            Token::Ident { text, .. }
                if reject_trait_for && text == "for" && !in_where && paren_depth == 0 && bracket_depth == 0 && angle_depth == 0 =>
            {
                saw_for = true;
                idx += 1;
            }
            Token::LBrace => {
                let close = find_matching_rbrace(tokens, idx, end);
                if !saw_for {
                    parse_container_items(tokens, idx + 1, close, rel_path, report);
                }
                return (close + 1).min(end);
            }
            _ => idx += 1,
        }
    }
    end
}

fn skip_item(tokens: &[Token], mut idx: usize, end: usize) -> usize {
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    while idx < end {
        match tokens[idx] {
            Token::LParen => {
                paren_depth += 1;
                idx += 1;
            }
            Token::RParen => {
                paren_depth = paren_depth.saturating_sub(1);
                idx += 1;
            }
            Token::LBracket => {
                bracket_depth += 1;
                idx += 1;
            }
            Token::RBracket => {
                bracket_depth = bracket_depth.saturating_sub(1);
                idx += 1;
            }
            Token::Semi if paren_depth == 0 && bracket_depth == 0 => return idx + 1,
            Token::LBrace => {
                let close = find_matching_rbrace(tokens, idx, end);
                return (close + 1).min(end);
            }
            _ => idx += 1,
        }
    }
    end
}

fn find_matching_rbrace(tokens: &[Token], open_idx: usize, end: usize) -> usize {
    let mut depth = 0usize;
    let mut cursor = open_idx;
    while cursor < end {
        match tokens[cursor] {
            Token::LBrace => depth += 1,
            Token::RBrace => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return cursor;
                }
            }
            _ => {}
        }
        cursor += 1;
    }
    end
}

/// Check whether `doc` (module `//!` text or routine `///` text) cites a published `D1` reference.
#[must_use]
pub fn cites_published_reference(doc: &str) -> bool {
    if doc.trim().is_empty() {
        return false;
    }
    let lower = doc.to_ascii_lowercase();
    if EXPLICIT_REFERENCE_MARKERS.iter().any(|marker| lower.contains(marker)) {
        return true;
    }
    if contains_isbn(&lower) {
        return true;
    }
    contains_author_year_citation(doc)
}

fn contains_isbn(lower: &str) -> bool {
    for (pos, _) in lower.match_indices("isbn") {
        if pos > 0 && lower.as_bytes()[pos - 1].is_ascii_alphanumeric() {
            continue;
        }
        let mut rest = &lower[pos + 4..];
        if let Some(after_tag) = rest.strip_prefix("-13").or_else(|| rest.strip_prefix("-10")) {
            rest = after_tag;
        }
        let trimmed = rest.trim_start_matches([' ', ':', '-']);
        if trimmed.as_bytes().first().is_some_and(u8::is_ascii_digit) {
            return true;
        }
    }
    false
}

fn contains_author_year_citation(doc: &str) -> bool {
    let bytes = doc.as_bytes();
    if bytes.len() < 4 {
        return false;
    }
    let disallowed_set: BTreeSet<&str> = NON_AUTHOR_CAPITALIZED_WORDS.iter().copied().collect();
    for idx in 0..=bytes.len() - 4 {
        if !is_publication_year_at(bytes, idx) {
            continue;
        }
        let window_start = floor_char_boundary(doc, idx.saturating_sub(56));
        let mut prefix = &doc[window_start..idx];
        if let Some(cut) = prefix.rfind(['<', '>', '=', '[', ']', '`']) {
            prefix = &prefix[cut + 1..];
        }
        if (prefix.contains('(') || prefix.contains(',') || prefix.contains(' ')) && window_contains_author_surname(prefix, &disallowed_set) {
            return true;
        }
    }
    false
}

fn floor_char_boundary(s: &str, mut index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    while index > 0 && !s.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn is_publication_year_at(bytes: &[u8], idx: usize) -> bool {
    let Some(slice) = bytes.get(idx..idx + 4) else {
        return false;
    };
    if !slice.iter().all(u8::is_ascii_digit) {
        return false;
    }
    let year =
        usize::from(slice[0] - b'0') * 1000 + usize::from(slice[1] - b'0') * 100 + usize::from(slice[2] - b'0') * 10 + usize::from(slice[3] - b'0');
    if !(1700..=2035).contains(&year) {
        return false;
    }
    if idx > 0 {
        let prev = bytes[idx - 1];
        if prev.is_ascii_alphanumeric() || matches!(prev, b'<' | b'>' | b'=' | b'+' | b'-' | b'*' | b'/' | b'.' | b'_') {
            return false;
        }
    }
    if let Some(&next) = bytes.get(idx + 4) {
        if next.is_ascii_digit() || matches!(next, b'.' | b'_') {
            return false;
        }
        if next.is_ascii_alphabetic() {
            let disambiguator = matches!(next, b'a'..=b'd') && !bytes.get(idx + 5).is_some_and(u8::is_ascii_alphanumeric);
            if !disambiguator {
                return false;
            }
        }
        // Reject ISO-8601 calendar dates `YYYY-MM` (such as `2026-11-18`).
        if next == b'-'
            && bytes.get(idx + 5).is_some_and(u8::is_ascii_digit)
            && bytes.get(idx + 6).is_some_and(u8::is_ascii_digit)
            && !bytes.get(idx + 7).is_some_and(u8::is_ascii_digit)
        {
            return false;
        }
    }
    true
}

fn window_contains_author_surname(window: &str, disallowed: &BTreeSet<&str>) -> bool {
    let mut current = String::new();
    for ch in window.chars() {
        if ch.is_alphabetic() || matches!(ch, '\'' | '\u{2019}' | '-') {
            current.push(ch);
        } else {
            if is_candidate_surname(&current, disallowed) {
                return true;
            }
            current.clear();
        }
    }
    is_candidate_surname(&current, disallowed)
}

fn is_candidate_surname(word: &str, disallowed: &BTreeSet<&str>) -> bool {
    let trimmed = word.trim_matches(['\'', '\u{2019}', '-']);
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_uppercase() {
        return false;
    }
    if !chars.any(char::is_lowercase) {
        return false;
    }
    !disallowed.contains(trimmed)
}

fn lex_module(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;

    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\n' {
            line += 1;
            i += 1;
        } else if b.is_ascii_whitespace() {
            i += 1;
        } else if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i = lex_line_comment(source, i, &mut tokens);
        } else if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i = lex_block_comment(source, i, &mut line, &mut tokens);
        } else if b == b'#' && (bytes.get(i + 1) == Some(&b'[') || (bytes.get(i + 1) == Some(&b'!') && bytes.get(i + 2) == Some(&b'['))) {
            i = lex_attribute(source, i, &mut line, &mut tokens);
        } else if b == b'r' && matches!(bytes.get(i + 1), Some(b'"' | b'#')) {
            if let Some(next_i) = skip_raw_string(bytes, i, &mut line) {
                tokens.push(Token::Other);
                i = next_i;
            } else {
                i = lex_ident(source, i, line, &mut tokens);
            }
        } else if b == b'"' {
            i = skip_quoted_string(bytes, i, &mut line);
            tokens.push(Token::Other);
        } else if b == b'\'' {
            i = skip_char_or_lifetime(source, i);
        } else if b.is_ascii_alphabetic() || b == b'_' {
            i = lex_ident(source, i, line, &mut tokens);
        } else if (b == b'-' || b == b'=' || b == b'<' || b == b'>') && matches!(bytes.get(i + 1), Some(b'>' | b'=')) {
            tokens.push(Token::Other);
            i += 2;
        } else {
            match b {
                b'(' => tokens.push(Token::LParen),
                b')' => tokens.push(Token::RParen),
                b'[' => tokens.push(Token::LBracket),
                b']' => tokens.push(Token::RBracket),
                b'<' => tokens.push(Token::LAngle),
                b'>' => tokens.push(Token::RAngle),
                b'{' => tokens.push(Token::LBrace),
                b'}' => tokens.push(Token::RBrace),
                b';' => tokens.push(Token::Semi),
                _ => {}
            }
            i += source[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
    tokens
}

fn lex_line_comment(source: &str, start: usize, tokens: &mut Vec<Token>) -> usize {
    let bytes = source.as_bytes();
    let mut end = start + 2;
    while end < bytes.len() && bytes[end] != b'\n' {
        end += 1;
    }
    let slice = &source[start..end];
    if let Some(inner) = slice.strip_prefix("//!") {
        tokens.push(Token::InnerDoc(inner.trim().to_owned()));
    } else if let Some(outer) = slice.strip_prefix("///")
        && !outer.starts_with('/')
    {
        tokens.push(Token::OuterDoc(outer.trim().to_owned()));
    }
    end
}

fn lex_block_comment(source: &str, start: usize, line: &mut usize, tokens: &mut Vec<Token>) -> usize {
    let bytes = source.as_bytes();
    let is_inner = bytes.get(start + 2) == Some(&b'!');
    let is_outer = bytes.get(start + 2) == Some(&b'*') && bytes.get(start + 3) != Some(&b'*') && bytes.get(start + 3) != Some(&b'/');
    let mut depth = 1usize;
    let mut i = start + 2;
    while i < bytes.len() && depth > 0 {
        if bytes[i] == b'\n' {
            *line += 1;
            i += 1;
        } else if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
            depth -= 1;
            i += 2;
        } else {
            i += 1;
        }
    }
    let body_end = i.saturating_sub(2);
    if is_inner && start + 3 <= body_end {
        tokens.push(Token::InnerDoc(source[start + 3..body_end].trim().to_owned()));
    } else if is_outer && start + 3 <= body_end {
        tokens.push(Token::OuterDoc(source[start + 3..body_end].trim().to_owned()));
    }
    i
}

fn lex_attribute(source: &str, start: usize, line: &mut usize, tokens: &mut Vec<Token>) -> usize {
    let bytes = source.as_bytes();
    let is_inner = bytes.get(start + 1) == Some(&b'!');
    let open_bracket = if is_inner { start + 2 } else { start + 1 };
    let mut depth = 0usize;
    let mut i = open_bracket;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                *line += 1;
                i += 1;
            }
            b'"' => {
                i = skip_quoted_string(bytes, i, line);
            }
            b'[' => {
                depth += 1;
                i += 1;
            }
            b']' => {
                depth = depth.saturating_sub(1);
                i += 1;
                if depth == 0 {
                    break;
                }
            }
            _ => i += 1,
        }
    }
    let inside = source.get(open_bracket + 1..i.saturating_sub(1)).unwrap_or_default().trim();
    if let Some(doc_val) = extract_doc_attr_value(inside) {
        if is_inner {
            tokens.push(Token::InnerDoc(doc_val));
        } else {
            tokens.push(Token::OuterDoc(doc_val));
        }
    } else if !is_inner {
        let compact: String = inside.chars().filter(|c| !c.is_whitespace()).collect();
        tokens.push(Token::OuterAttr { is_cfg_test: compact == "cfg(test)" });
    }
    i
}

fn extract_doc_attr_value(inside: &str) -> Option<String> {
    let rest = inside.strip_prefix("doc")?.trim_start();
    let rhs = rest.strip_prefix('=')?.trim();
    let unquoted = rhs.strip_prefix('"')?.strip_suffix('"')?;
    Some(unquoted.to_owned())
}

fn skip_quoted_string(bytes: &[u8], start: usize, line: &mut usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                *line += 1;
                i += 1;
            }
            b'\\' => i = (i + 2).min(bytes.len()),
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

fn skip_raw_string(bytes: &[u8], start: usize, line: &mut usize) -> Option<usize> {
    let mut i = start + 1;
    let mut hashes = 0usize;
    while bytes.get(i) == Some(&b'#') {
        hashes += 1;
        i += 1;
    }
    if bytes.get(i) != Some(&b'"') {
        return None;
    }
    i += 1;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            *line += 1;
            i += 1;
            continue;
        }
        if bytes[i] == b'"' {
            let mut matched = 0usize;
            while matched < hashes && bytes.get(i + 1 + matched) == Some(&b'#') {
                matched += 1;
            }
            if matched == hashes {
                return Some(i + 1 + hashes);
            }
        }
        i += 1;
    }
    Some(bytes.len())
}

fn skip_char_or_lifetime(source: &str, start: usize) -> usize {
    let Some(tail) = source.get(start + 1..) else {
        return start + 1;
    };
    let mut chars = tail.char_indices();
    let Some((_, first)) = chars.next() else {
        return start + 1;
    };
    if first == '\\' {
        let Some(_) = chars.next() else {
            return start + 1;
        };
        for (rel, ch) in chars {
            if ch == '\'' {
                return start + 1 + rel + ch.len_utf8();
            }
            if ch == '\n' || rel > 12 {
                return start + 1;
            }
        }
        return start + 1;
    }
    // Single-char literal `'x'` (including multi-byte UTF-8); lifetimes like `'a` have no closing `'`.
    if first != '\''
        && let Some((rel, '\'')) = chars.next()
    {
        return start + 1 + rel + 1;
    }
    start + 1
}

fn lex_ident(source: &str, start: usize, line: usize, tokens: &mut Vec<Token>) -> usize {
    let bytes = source.as_bytes();
    let mut end = start + 1;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }
    let text = source[start..end].to_owned();
    tokens.push(Token::Ident { text, line });
    end
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{ALLOWLIST, AllowlistedModule, NUMERICAL_SRC_DIRS, check_workspace_with_allowlist, cites_published_reference, scan_module_source};

    fn make_temp_workspace(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("cb-xtask-citations-{tag}-{}-{nanos}", std::process::id()));
        for rel in NUMERICAL_SRC_DIRS {
            std::fs::create_dir_all(dir.join(rel)).unwrap();
        }
        dir
    }

    #[test]
    fn cited_module_header_passes_all_public_routines() {
        let src = "\
//! Safeguarded 1D root finding.
//!
//! Reference: Brent, R. P. (1973). *Algorithms for Minimization without Derivatives*. Prentice-Hall.

pub struct RootBracket {
    pub lo: f64,
    pub hi: f64,
}

impl RootBracket {
    #[must_use]
    pub const fn span(&self) -> f64 {
        self.hi - self.lo
    }
}

/// Find a bracketed root without repeating the module citation.
pub fn find_root(lo: f64, hi: f64) -> f64 {
    0.5 * (lo + hi)
}
";
        let report = scan_module_source("crates/math/src/roots.rs", src);
        assert!(report.module_cited);
        assert_eq!(report.public_routines, 2);
        assert!(report.violations.is_empty(), "unexpected violations: {:?}", report.violations);
    }

    #[test]
    fn cited_routine_doc_passes_when_module_header_has_no_citation() {
        let src = "\
//! Assorted statistical routines citing primary sources per function.

/// Shapiro-Wilk W statistic using Royston (1992; 1995) polynomial coefficients.
#[must_use]
pub fn shapiro_wilk_w(sample: &[f64]) -> f64 {
    sample.len() as f64
}

pub struct Estimator;

impl Estimator {
    /// Nonparametric scale estimator; see Rousseeuw & Croux (1993), `doi:10.1080/01621459.1993.10476408`.
    pub fn qn_scale(sample: &[f64]) -> f64 {
        sample.len() as f64
    }
}
";
        let report = scan_module_source("crates/stats/src/custom.rs", src);
        assert!(!report.module_cited);
        assert_eq!(report.public_routines, 2);
        assert!(report.violations.is_empty(), "unexpected violations: {:?}", report.violations);
    }

    #[test]
    fn uncited_pub_fn_fails_closed_even_when_sibling_routine_is_cited() {
        let src = "\
//! Module summary with audit date 2026-11-18 and cap n <= 2000.

/// Cited routine per Zar (2010), Ch. 8.
pub fn cited_test(x: &[f64]) -> f64 {
    x.len() as f64
}

/// Computes an ad-hoc score for up to 2000 rows without citing any reference.
pub fn uncited_heuristic(x: &[f64]) -> f64 {
    x.len() as f64
}

#[cfg(test)]
mod tests {
    pub fn helper_in_tests_is_ignored() {}
}

/// Uncited routine placed after `#[cfg(test)]` must still be caught.
pub fn trailing_uncited(x: f64) -> f64 {
    x
}
";
        let report = scan_module_source("crates/stats/src/uncited.rs", src);
        assert!(!report.module_cited);
        assert_eq!(report.public_routines, 3);
        assert_eq!(report.violations.len(), 2, "expected 2 uncited routines, got {:?}", report.violations);
        assert!(report.violations[0].detail.contains("uncited_heuristic"));
        assert!(report.violations[1].detail.contains("trailing_uncited"));
    }

    #[test]
    fn private_and_crate_internal_only_module_passes_without_allowlist() {
        let src = "\
//! Internal helper utilities.

pub enum SampleError {
    Empty,
}

fn private_kernel(x: f64) -> f64 {
    x * x
}

pub(crate) fn crate_visible_kernel(x: f64) -> f64 {
    private_kernel(x)
}

#[cfg(test)]
mod tests {
    pub fn test_only_pub_fn() {}
}
";
        let report = scan_module_source("crates/math/src/internal.rs", src);
        assert_eq!(report.public_routines, 0);
        assert!(report.violations.is_empty());
    }

    #[test]
    fn allowlisted_plumbing_module_passes_and_nonexistent_or_empty_reason_allowlist_fails() {
        let ws = make_temp_workspace("allowlist");
        std::fs::write(ws.join("crates/math/src/lib.rs"), "//! Plumbing root.\npub fn axis_helper(x: f64) -> f64 { x }\n").unwrap();
        std::fs::write(ws.join("crates/stats/src/anova.rs"), "//! Reference: Satterthwaite (1946).\npub fn run_anova() -> f64 { 1.0 }\n").unwrap();
        std::fs::write(ws.join("crates/fit/src/solver.rs"), "//! Reference: Marquardt (1963).\npub fn solve() -> f64 { 1.0 }\n").unwrap();
        std::fs::write(ws.join("crates/transform/src/curve.rs"), "//! Reference: Savitzky & Golay (1964).\npub fn smooth() -> f64 { 1.0 }\n")
            .unwrap();

        let valid_allow = [AllowlistedModule { path: "crates/math/src/lib.rs", reason: "Crate-root plumbing without statistical routines" }];
        let (stats, violations) = check_workspace_with_allowlist(&ws, NUMERICAL_SRC_DIRS, &valid_allow).unwrap();
        assert!(violations.is_empty(), "expected clean pass, got {violations:?}");
        assert_eq!(stats.allowlisted_modules, 1);
        assert_eq!(stats.cited_modules, 3);

        // Stale / nonexistent allow-list path must fail closed.
        let stale_allow = [
            AllowlistedModule { path: "crates/math/src/lib.rs", reason: "Plumbing" },
            AllowlistedModule { path: "crates/math/src/ghost.rs", reason: "Removed module" },
        ];
        let (_, stale_violations) = check_workspace_with_allowlist(&ws, NUMERICAL_SRC_DIRS, &stale_allow).unwrap();
        assert_eq!(stale_violations.len(), 1);
        assert!(stale_violations[0].detail.contains("ghost.rs"));

        // Blank allow-list reason must also fail closed.
        let blank_reason = [AllowlistedModule { path: "crates/math/src/lib.rs", reason: "   " }];
        let (_, reason_violations) = check_workspace_with_allowlist(&ws, NUMERICAL_SRC_DIRS, &blank_reason).unwrap();
        assert_eq!(reason_violations.len(), 1);
        assert!(reason_violations[0].detail.contains("reason"));

        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn citation_detector_calibrates_positive_references_and_rejects_dates_and_bounds() {
        for valid in [
            "Student (1908). The probable error of a mean.",
            "Nonlinear ROUT outlier detection (Motulsky & Brown 2006).",
            "Textbook least squares (Draper & Smith 1998; Zar 2010).",
            "See `doi:10.1080/00031305.1996.10473566`.",
            "NIST Digital Library of Mathematical Functions, Ch. 7.",
            "Abramowitz & Stegun, Ch. 26.",
            "Press et al., ISBN 978-0-521-88068-8.",
        ] {
            assert!(cites_published_reference(valid), "expected citation match for `{valid}`");
        }

        for non_citation in [
            "",
            "Audit 2026-11-18: supports n <= 2000 observations.",
            "Table (2000) rows maximum.",
            "Port 1999 local listener.",
            "Computes arithmetic mean of finite values.",
        ] {
            assert!(!cites_published_reference(non_citation), "expected non-match for `{non_citation}`");
        }
    }

    #[test]
    fn live_workspace_numerical_crates_satisfy_d1_citation_gate() {
        let root = crate::workspace_root().unwrap();
        let (stats, violations) = check_workspace_with_allowlist(&root, NUMERICAL_SRC_DIRS, ALLOWLIST).unwrap();
        assert!(violations.is_empty(), "live workspace has D1 citation violations: {violations:?}");
        assert!(stats.cited_modules >= 40, "expected at least 40 cited numerical modules, got {stats:?}");
    }

    #[test]
    fn fa_sec23_05_hrtb_for_lifetimes_and_multibyte_chars_scan_accurately() {
        let src = "\
const ALPHA: char = 'α';
const ESC: char = '\\'';
impl<T> Estimator<T> where for<'a> &'a T: Clone {
    pub fn undocumented_in_where_hrtb(x: T) -> T { x }
}
impl<F> Wrapper<for<'a> fn(&'a f64)> {
    pub fn undocumented_in_angle_hrtb() {}
}
impl Default for Estimator<f64> {
    fn default() -> Self { unreachable!() }
}
";
        let report = scan_module_source("crates/stats/src/hrtb.rs", src);
        assert_eq!(report.public_routines, 2, "both inherent impl pub fns must be counted");
        assert_eq!(report.violations.len(), 2, "both undocumented HRTB pub fns must trigger violations: {:?}", report.violations);
    }
}
