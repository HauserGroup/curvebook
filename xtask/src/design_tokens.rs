//! `cargo xtask design-tokens`: ratcheting gate on raw colour constructors and raw size pairs
//! inside `crates/ui-egui/src/` (`docs/design-language.md` §5).
//!
//! Ensures UI widgets read colours and measurements from `Tokens` and `LayoutMetrics` rather than
//! scattering ad-hoc RGB triples and pixel pairs across views and modals:
//! - `crates/ui-egui/src/egui_shell/tokens.rs` (`Tokens`) must contain **zero** raw colour literals
//!   (all UI colours derive from `ThemeSeeds` in `crates/ui-egui/src/theme.rs`).
//! - Canonical token/seed definitions (`theme.rs`), procedural vector brand/icon art (`brand.rs`),
//!   CPU screenshot encoding (`rasterizer.rs`), test driver (`driver.rs`), and test modules are exempt.
//! - Legitimate geometric/canvas calculations may opt out on a single line with a documented
//!   `// design-token-exempt: <reason>` comment (bare `// design-token-exempt` without a reason fails).
//! - Total non-exempt raw colours and raw numeric size pairs across `crates/ui-egui/src/` may
//!   only stay equal or fall over time.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use crate::rust_lex::{RustLexer, blank_comments_and_literals, is_ident_byte, mask_cfg_test_items};

/// Maximum non-exempt raw colour constructors permitted across `crates/ui-egui/src/`.
/// Lower this ceiling whenever raw colour call sites are migrated onto `Tokens`.
///
/// Updated from `39` to `55` under `AUD-12-27` after expanding detection to cover
/// `Color32::from_black_alpha`, `from_white_alpha`, `from_rgb_additive`,
/// `from_rgba_unmultiplied_const`, `from_hex`, named `Color32` color constants, `Rgba8 { .. }`
/// struct literals, and whitespace-separated calls (16 previously uncounted live sites in
/// `crates/ui-egui/src/`).
pub const MAX_RAW_COLORS: usize = 55;

/// Maximum non-exempt raw numeric size pairs (`vec2(...)`, `Vec2::new(...)`, `Vec2::splat(...)`,
/// `Vec2 { .. }`) permitted across `crates/ui-egui/src/`. Lower this ceiling whenever raw widget
/// sizes are migrated onto `LayoutMetrics`.
///
/// Tightened from `236` to `232` (exact live count after `AUD-12-27` expanded detection to cover
/// `Vec2::splat`, `Vec2 { .. }`, parenthesized expressions, and `as <ty>` casts).
pub const MAX_RAW_SIZE_PAIRS: usize = 232;

const UI_SRC_PREFIX: &str = "crates/ui-egui/src/";
const TOKENS_MOD_REL: &str = "crates/ui-egui/src/egui_shell/tokens.rs";
const EXEMPT_MARKER: &str = concat!("design-token-", "exempt:");
const BARE_EXEMPT_MARKER: &str = concat!("design-token-", "exempt");

const EXEMPT_FILES: &[&str] = &[
    "crates/ui-egui/src/theme.rs",
    "crates/ui-egui/src/egui_shell/brand.rs",
    "crates/ui-egui/src/egui_shell/rasterizer.rs",
    "crates/ui-egui/src/egui_shell/driver.rs",
];

const COLOR32_CONSTRUCTORS: &[&str] = &[
    "from_rgb",
    "from_rgba_unmultiplied",
    "from_rgba_premultiplied",
    "from_gray",
    "from_black_alpha",
    "from_white_alpha",
    "from_rgb_additive",
    "from_rgba_unmultiplied_const",
    "from_hex",
];

const COLOR32_NAMED_CONSTANTS: &[&str] = &[
    "WHITE",
    "BLACK",
    "RED",
    "GREEN",
    "BLUE",
    "YELLOW",
    "CYAN",
    "MAGENTA",
    "GRAY",
    "LIGHT_GRAY",
    "DARK_GRAY",
    "LIGHT_RED",
    "DARK_RED",
    "LIGHT_GREEN",
    "DARK_GREEN",
    "LIGHT_BLUE",
    "DARK_BLUE",
    "LIGHT_YELLOW",
    "GOLD",
    "BROWN",
    "KHAKI",
    "ORANGE",
    "PURPLE",
    "PLACEHOLDER",
];

const RGBA8_CONSTRUCTORS: &[&str] = &["rgb", "rgba"];
const VEC2_CONSTRUCTORS: &[&str] = &["new", "splat"];

/// Summary of raw colour and raw size-pair counts across `crates/ui-egui/src/`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesignTokenScanReport {
    pub raw_colors: usize,
    pub raw_size_pairs: usize,
    pub tokens_mod_raw_colors: usize,
    pub per_file_colors: BTreeMap<String, usize>,
    pub per_file_sizes: BTreeMap<String, usize>,
    pub invalid_exemptions: Vec<String>,
}

/// Entry point for `cargo xtask design-tokens [--verbose]`.
pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let verbose = match args {
        [] => false,
        [flag] if flag == "--verbose" => true,
        _ => return Err(String::from("usage: cargo xtask design-tokens [--verbose]")),
    };
    let files = crate::repo_files(root)?;
    let mut sources = Vec::new();
    for rel in files {
        if rel.starts_with(UI_SRC_PREFIX) && rel.ends_with(".rs") {
            let content = fs::read_to_string(root.join(&rel)).map_err(|e| format!("read {rel}: {e}"))?;
            sources.push((rel, content));
        }
    }
    let report = scan_sources(&sources);
    if verbose {
        print_breakdown(&report);
    }
    enforce_ceilings(&report, MAX_RAW_COLORS, MAX_RAW_SIZE_PAIRS)?;
    println!(
        "design-tokens: OK (raw_colors={}/{MAX_RAW_COLORS}, raw_size_pairs={}/{MAX_RAW_SIZE_PAIRS}, tokens_mod_raw_colors=0)",
        report.raw_colors, report.raw_size_pairs
    );
    Ok(())
}

fn print_breakdown(report: &DesignTokenScanReport) {
    for (file, count) in &report.per_file_colors {
        println!("  raw-color  {count:>4}  {file}");
    }
    for (file, count) in &report.per_file_sizes {
        println!("  raw-size   {count:>4}  {file}");
    }
}

/// Scan `(relative_path, source_text)` pairs and return [`DesignTokenScanReport`].
#[must_use]
pub fn scan_sources(sources: &[(String, String)]) -> DesignTokenScanReport {
    let mut report = DesignTokenScanReport::default();
    for (rel, content) in sources {
        if is_test_file(rel) || EXEMPT_FILES.contains(&rel.as_str()) {
            continue;
        }
        let blanked = blank_comments_and_literals(content);
        let mut prod_code_bytes = blanked.as_bytes().to_vec();
        let mut raw_prod_bytes = content.as_bytes().to_vec();
        mask_cfg_test_items(blanked.as_bytes(), &mut [&mut prod_code_bytes, &mut raw_prod_bytes]);
        let prod_code = String::from_utf8(prod_code_bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
        let raw_prod = String::from_utf8(raw_prod_bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());

        let exempt_lines = collect_exemption_lines(rel, &raw_prod, &mut report.invalid_exemptions);
        let line_starts = compute_line_starts(&prod_code);
        let is_tokens_mod = rel == TOKENS_MOD_REL;

        let (file_colors, tokens_mod_colors) = count_color_calls(&prod_code, &raw_prod, &line_starts, &exempt_lines, is_tokens_mod);
        let file_sizes = count_raw_size_calls(&prod_code, &line_starts, &exempt_lines);

        report.tokens_mod_raw_colors += tokens_mod_colors;
        if file_colors > 0 {
            report.raw_colors += file_colors;
            report.per_file_colors.insert(rel.clone(), file_colors);
        }
        if file_sizes > 0 {
            report.raw_size_pairs += file_sizes;
            report.per_file_sizes.insert(rel.clone(), file_sizes);
        }
    }
    report
}

/// Verify [`DesignTokenScanReport`] against tracked ceilings and token-file rules.
pub fn enforce_ceilings(report: &DesignTokenScanReport, max_colors: usize, max_sizes: usize) -> Result<(), String> {
    let mut errors = Vec::new();
    if !report.invalid_exemptions.is_empty() {
        errors.push(format!("malformed design-token exemption comment(s):\n  {}", report.invalid_exemptions.join("\n  ")));
    }
    if report.tokens_mod_raw_colors > 0 {
        errors.push(format!(
            "`{TOKENS_MOD_REL}` contains {} raw colour literal(s); derive `Tokens` colours from `ThemeSeeds` in `crates/ui-egui/src/theme.rs` instead",
            report.tokens_mod_raw_colors
        ));
    }
    if report.raw_colors > max_colors {
        let top = format_map(&report.per_file_colors);
        errors.push(format!(
            "raw UI colour count {} exceeds ceiling {max_colors}; use `Tokens` fields (or document canvas math with `// {EXEMPT_MARKER} <reason>`):\n{top}",
            report.raw_colors
        ));
    }
    if report.raw_size_pairs > max_sizes {
        let top = format_map(&report.per_file_sizes);
        errors.push(format!(
            "raw UI size-pair count {} exceeds ceiling {max_sizes}; use `LayoutMetrics` tokens (or document canvas geometry with `// {EXEMPT_MARKER} <reason>`):\n{top}",
            report.raw_size_pairs
        ));
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("\n\n")) }
}

fn format_map(map: &BTreeMap<String, usize>) -> String {
    map.iter().map(|(f, n)| format!("  {n:>4}  {f}")).collect::<Vec<_>>().join("\n")
}

fn is_test_file(rel: &str) -> bool {
    rel.ends_with("/tests.rs") || rel.contains("/tests/")
}

fn collect_exemption_lines(rel: &str, raw_prod: &str, invalid_exemptions: &mut Vec<String>) -> BTreeSet<usize> {
    let mut exempt_lines = BTreeSet::new();
    let mut lex = RustLexer::new(raw_prod);
    while !lex.is_eof() {
        if let Some((line_no, comment_text)) = lex.try_read_comment() {
            if let Some((_, tail)) = comment_text.split_once(EXEMPT_MARKER) {
                let reason = tail.trim();
                if reason.is_empty() {
                    invalid_exemptions.push(format!("{rel}:{line_no}: `{EXEMPT_MARKER}` requires a non-empty reason"));
                } else {
                    exempt_lines.insert(line_no);
                }
            } else if comment_text.contains(BARE_EXEMPT_MARKER) {
                invalid_exemptions.push(format!("{rel}:{line_no}: use `{EXEMPT_MARKER} <reason>` with an explicit justification"));
            }
            continue;
        }
        if lex.try_read_raw_string().is_some() || lex.try_skip_char_literal() || lex.try_read_quoted_string().is_some() {
            continue;
        }
        lex.advance_one();
    }
    exempt_lines
}

fn compute_line_starts(src: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (idx, &b) in src.as_bytes().iter().enumerate() {
        if b == b'\n' {
            starts.push(idx + 1);
        }
    }
    starts
}

fn line_of_offset(line_starts: &[usize], offset: usize) -> usize {
    line_starts.partition_point(|&s| s <= offset)
}

fn is_call_exempt(exempt_lines: &BTreeSet<usize>, start_line: usize, end_line: usize) -> bool {
    exempt_lines.range(start_line..=end_line).next().is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodeTokKind<'a> {
    Ident(&'a str),
    ColonColon,
    LParen,
    LBrace,
    Dot,
    Other,
}

#[derive(Debug, Clone, Copy)]
struct CodeTok<'a> {
    kind: CodeTokKind<'a>,
    start: usize,
    end: usize,
}

fn tokenize_blanked_code(code: &str) -> Vec<CodeTok<'_>> {
    let bytes = code.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
        } else if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ident_byte(bytes[i]) {
                i += 1;
            }
            if let Some(ident) = code.get(start..i) {
                out.push(CodeTok { kind: CodeTokKind::Ident(ident), start, end: i });
            }
        } else if b == b':' && bytes.get(i + 1) == Some(&b':') {
            out.push(CodeTok { kind: CodeTokKind::ColonColon, start: i, end: i + 2 });
            i += 2;
        } else {
            let kind = match b {
                b'(' => CodeTokKind::LParen,
                b'{' => CodeTokKind::LBrace,
                b'.' => CodeTokKind::Dot,
                _ => CodeTokKind::Other,
            };
            let step = code.get(i..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
            out.push(CodeTok { kind, start: i, end: i + step });
            i += step;
        }
    }
    out
}

fn record_color_hit(exempt: bool, is_tokens_mod: bool, has_literal_arg: bool, regular_count: &mut usize, tokens_mod_count: &mut usize) {
    if is_tokens_mod {
        // `TOKENS_MOD_REL` forbids all raw colour literals even with an exemption comment;
        // only non-literal type-conversion bridges (e.g. `Rgba8 <-> Color32`) with an
        // explicit `design-token-exempt: <reason>` comment are permitted.
        if !exempt || has_literal_arg {
            *tokens_mod_count += 1;
        }
    } else if !exempt {
        *regular_count += 1;
    }
}

fn count_color_calls(code: &str, raw_prod: &str, line_starts: &[usize], exempt_lines: &BTreeSet<usize>, is_tokens_mod: bool) -> (usize, usize) {
    let toks = tokenize_blanked_code(code);
    let mut regular_count = 0usize;
    let mut tokens_mod_count = 0usize;
    let mut i = 0usize;
    while i < toks.len() {
        let CodeTokKind::Ident(head) = toks[i].kind else {
            i += 1;
            continue;
        };
        if (head == "Color32" || head == "Rgba8")
            && matches!(toks.get(i + 1).map(|t| t.kind), Some(CodeTokKind::ColonColon))
            && let Some(CodeTok { kind: CodeTokKind::Ident(member), end: member_end, .. }) = toks.get(i + 2).copied()
        {
            let is_ctor = (head == "Color32" && COLOR32_CONSTRUCTORS.contains(&member)) || (head == "Rgba8" && RGBA8_CONSTRUCTORS.contains(&member));
            if is_ctor && let Some(CodeTok { kind: CodeTokKind::LParen, end: args_start, .. }) = toks.get(i + 3).copied() {
                let args_opt = code.get(args_start..).and_then(|t| extract_balanced_delim_with_end(t, '(', ')'));
                let start_line = line_of_offset(line_starts, toks[i].start);
                let end_line = args_opt.map_or(start_line, |(_, close_rel)| line_of_offset(line_starts, args_start + close_rel));
                let exempt = is_call_exempt(exempt_lines, start_line, end_line);
                let has_literal_arg = args_opt.is_none_or(|(args, close_rel)| {
                    contains_numeric_literal_token(args)
                        || raw_prod.get(args_start..args_start + close_rel).is_some_and(|raw_args| raw_args.contains('"'))
                });
                record_color_hit(exempt, is_tokens_mod, has_literal_arg, &mut regular_count, &mut tokens_mod_count);
                i += 4;
                continue;
            }
            if head == "Color32" && COLOR32_NAMED_CONSTANTS.contains(&member) {
                let start_line = line_of_offset(line_starts, toks[i].start);
                let end_line = line_of_offset(line_starts, member_end.saturating_sub(1));
                let exempt = is_call_exempt(exempt_lines, start_line, end_line);
                record_color_hit(exempt, is_tokens_mod, true, &mut regular_count, &mut tokens_mod_count);
                i += 3;
                continue;
            }
        } else if head == "Rgba8"
            && let Some(CodeTok { kind: CodeTokKind::LBrace, end: body_start, .. }) = toks.get(i + 1).copied()
        {
            let body_opt = code.get(body_start..).and_then(|t| extract_balanced_delim_with_end(t, '{', '}'));
            let start_line = line_of_offset(line_starts, toks[i].start);
            let end_line = body_opt.map_or(start_line, |(_, close_rel)| line_of_offset(line_starts, body_start + close_rel));
            let exempt = is_call_exempt(exempt_lines, start_line, end_line);
            let has_literal_arg = body_opt.is_none_or(|(body, _)| contains_numeric_literal_token(body));
            record_color_hit(exempt, is_tokens_mod, has_literal_arg, &mut regular_count, &mut tokens_mod_count);
            i += 2;
            continue;
        }
        i += 1;
    }
    (regular_count, tokens_mod_count)
}

fn count_raw_size_calls(code: &str, line_starts: &[usize], exempt_lines: &BTreeSet<usize>) -> usize {
    let toks = tokenize_blanked_code(code);
    let mut count = 0usize;
    let mut i = 0usize;
    while i < toks.len() {
        let CodeTokKind::Ident(head) = toks[i].kind else {
            i += 1;
            continue;
        };
        let prev_is_dot = i > 0 && toks[i - 1].kind == CodeTokKind::Dot;
        if head == "Vec2"
            && matches!(toks.get(i + 1).map(|t| t.kind), Some(CodeTokKind::ColonColon))
            && let Some(CodeTok { kind: CodeTokKind::Ident(member), .. }) = toks.get(i + 2).copied()
            && VEC2_CONSTRUCTORS.contains(&member)
            && let Some(CodeTok { kind: CodeTokKind::LParen, end: args_start, .. }) = toks.get(i + 3).copied()
        {
            if let Some((args, close_rel)) = code.get(args_start..).and_then(|t| extract_balanced_delim_with_end(t, '(', ')')) {
                let start_line = line_of_offset(line_starts, toks[i].start);
                let end_line = line_of_offset(line_starts, args_start + close_rel);
                if !is_call_exempt(exempt_lines, start_line, end_line) && args_contain_nonzero_numeric_literal(args) {
                    count += 1;
                }
            }
            i += 4;
            continue;
        }
        if head == "Vec2"
            && let Some(CodeTok { kind: CodeTokKind::LBrace, end: body_start, .. }) = toks.get(i + 1).copied()
        {
            if let Some((body, close_rel)) = code.get(body_start..).and_then(|t| extract_balanced_delim_with_end(t, '{', '}')) {
                let start_line = line_of_offset(line_starts, toks[i].start);
                let end_line = line_of_offset(line_starts, body_start + close_rel);
                if !is_call_exempt(exempt_lines, start_line, end_line) && struct_fields_contain_nonzero_numeric_literal(body) {
                    count += 1;
                }
            }
            i += 2;
            continue;
        }
        if head == "vec2"
            && !prev_is_dot
            && let Some(CodeTok { kind: CodeTokKind::LParen, end: args_start, .. }) = toks.get(i + 1).copied()
        {
            if let Some((args, close_rel)) = code.get(args_start..).and_then(|t| extract_balanced_delim_with_end(t, '(', ')')) {
                let start_line = line_of_offset(line_starts, toks[i].start);
                let end_line = line_of_offset(line_starts, args_start + close_rel);
                if !is_call_exempt(exempt_lines, start_line, end_line) && args_contain_nonzero_numeric_literal(args) {
                    count += 1;
                }
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    count
}

fn extract_balanced_delim_with_end(tail: &str, open: char, close: char) -> Option<(&str, usize)> {
    let mut depth = 1usize;
    for (idx, ch) in tail.char_indices() {
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
            if depth == 0 {
                return tail.get(..idx).map(|args| (args, idx));
            }
        }
    }
    None
}

fn contains_numeric_literal_token(s: &str) -> bool {
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b.is_ascii_digit() {
            let prev_ident = i > 0 && is_ident_byte(bytes[i - 1]);
            if !prev_ident {
                return true;
            }
        }
    }
    false
}

fn strip_wrapping_parens(s: &str) -> Option<&str> {
    let inner = s.strip_prefix('(')?.strip_suffix(')')?;
    let mut depth = 0usize;
    for ch in inner.chars() {
        match ch {
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    (depth == 0).then_some(inner)
}

fn strip_trailing_as_cast(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let mut depth = 0usize;
    let mut last_as = None;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b'a' if depth == 0
                && bytes.get(i + 1) == Some(&b's')
                && (i == 0 || !is_ident_byte(bytes[i - 1]))
                && bytes.get(i + 2).is_none_or(|&b| !is_ident_byte(b)) =>
            {
                last_as = Some(i);
            }
            _ => {}
        }
        i += 1;
    }
    let pos = last_as?;
    let rhs = s.get(pos + 2..)?.trim();
    if rhs.is_empty() || !rhs.bytes().all(is_ident_byte) {
        return None;
    }
    Some(s.get(..pos)?.trim())
}

fn parse_numeric_literal(part: &str) -> Option<f64> {
    const SUFFIXES: &[&str] = &["_f32", "f32", "_f64", "f64", "_u8", "u8", "_u16", "u16", "_u32", "u32", "_i32", "i32", "_usize", "usize"];
    let mut trimmed = part.trim();
    loop {
        let prev = trimmed;
        if let Some(inner) = strip_wrapping_parens(trimmed) {
            trimmed = inner.trim();
        } else if let Some(lhs) = strip_trailing_as_cast(trimmed) {
            trimmed = lhs.trim();
        } else if let Some(rest) = trimmed.strip_prefix('-').or_else(|| trimmed.strip_prefix('+')) {
            trimmed = rest.trim();
        }
        if trimmed == prev {
            break;
        }
    }
    for sfx in SUFFIXES {
        if let Some(stem) = trimmed.strip_suffix(sfx) {
            trimmed = stem;
            break;
        }
    }
    if !trimmed.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        return None;
    }
    for (p_lo, p_hi, radix) in [("0x", "0X", 16), ("0b", "0B", 2), ("0o", "0O", 8)] {
        if let Some(digits) = trimmed.strip_prefix(p_lo).or_else(|| trimmed.strip_prefix(p_hi)) {
            let clean: String = digits.chars().filter(|&c| c != '_').collect();
            return u128::from_str_radix(&clean, radix).ok().map(|v| v as f64);
        }
    }
    let clean: String = trimmed.chars().filter(|&c| c != '_').collect();
    clean.parse::<f64>().ok()
}

fn args_contain_nonzero_numeric_literal(args: &str) -> bool {
    for part in split_top_level_commas(args) {
        if let Some(val) = parse_numeric_literal(part)
            && val.abs() > 0.0
        {
            return true;
        }
    }
    false
}

fn struct_fields_contain_nonzero_numeric_literal(body: &str) -> bool {
    for part in split_top_level_commas(body) {
        let trimmed = part.trim();
        if trimmed.is_empty() || trimmed.starts_with("..") {
            continue;
        }
        let expr = trimmed.split_once(':').map_or(trimmed, |(_, rhs)| rhs);
        if let Some(val) = parse_numeric_literal(expr)
            && val.abs() > 0.0
        {
            return true;
        }
    }
    false
}

fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (idx, ch) in s.char_indices() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                if let Some(seg) = s.get(start..idx) {
                    parts.push(seg);
                }
                start = idx + 1;
            }
            _ => {}
        }
    }
    if let Some(tail) = s.get(start..) {
        parts.push(tail);
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::{TOKENS_MOD_REL, enforce_ceilings, scan_sources};

    #[test]
    fn counts_raw_colors_and_literal_sizes_while_preserving_token_calls_and_tests() {
        let widget_code = format!(
            "{}\n{}\n{}\n{}\n{}\n#[cfg(test)]\nmod tests {{\n    fn t() {{ let _ = {}(1, 2, 3); let _ = {}(9.0, 9.0); }}\n}}\n",
            concat!("let bad_c = Color32::", "from_rgb(10, 20, 30);"),
            concat!("let bad_sz = egui::vec", "2(14.0, 28.0);"),
            concat!("let ok_tok = egui::vec", "2(tokens.metrics.space_md, tokens.metrics.min_toolbar_target_pt);"),
            concat!("let ok_zero = Vec2::", "new(0.0, 0.0);"),
            concat!("let ok_exempt = Color32::", "from_rgb(1, 2, 3); // design-token-", "exempt: canvas crosshair tint"),
            concat!("Color32::", "from_rgb"),
            concat!("vec", "2"),
        );
        let sources = vec![
            (String::from("crates/ui-egui/src/egui_shell/sample_widget.rs"), widget_code),
            (String::from("crates/ui-egui/src/theme.rs"), format!("fn seed() {{ let _ = {}(255, 255, 255); }}", concat!("Rgba8::", "rgb"))),
            (
                String::from("crates/ui-egui/src/dialogs_canvas/tests.rs"),
                format!("fn fixture() {{ let _ = {}(9, 9, 9); }}", concat!("Rgba8::", "rgb")),
            ),
        ];

        let report = scan_sources(&sources);
        assert_eq!(report.raw_colors, 1, "expected exactly 1 non-exempt raw colour");
        assert_eq!(report.raw_size_pairs, 1, "expected token-backed and zero vec2 calls to pass");
        assert!(report.invalid_exemptions.is_empty());

        assert!(enforce_ceilings(&report, 1, 1).is_ok());
        let color_err = enforce_ceilings(&report, 0, 1).unwrap_err();
        assert!(color_err.contains("raw UI colour count 1 exceeds ceiling 0"), "{color_err}");
        let size_err = enforce_ceilings(&report, 1, 0).unwrap_err();
        assert!(size_err.contains("raw UI size-pair count 1 exceeds ceiling 0"), "{size_err}");
    }

    #[test]
    fn rejects_raw_colors_in_tokens_mod_and_reasonless_exemptions() {
        let sources = vec![
            (String::from(TOKENS_MOD_REL), format!("let c = {}(12, 34, 56);", concat!("Color32::", "from_rgb"))),
            (
                String::from("crates/ui-egui/src/egui_shell/view.rs"),
                format!("let v = {}(8.0, 8.0); // {}", concat!("vec", "2"), concat!("design-token-", "exempt:")),
            ),
        ];
        let report = scan_sources(&sources);
        assert_eq!(report.tokens_mod_raw_colors, 1);
        assert_eq!(report.invalid_exemptions.len(), 1);
        let err = enforce_ceilings(&report, 10, 10).unwrap_err();
        assert!(err.contains(TOKENS_MOD_REL), "{err}");
        assert!(err.contains("requires a non-empty reason"), "{err}");
    }

    #[test]
    fn da_03_03_and_da_03_08_item_scoped_cfg_test_tokens_mod_and_rust_lex_scanning() {
        let mod_with_early_cfg_test = format!(
            "#[cfg(test)]\nmod tests;\n\npub fn live() {{\n    /* {}(1, 2, 3); {}(9.0, 9.0); */\n    let q = '\"';\n    let _c = {}(10, 20, 30);\n    let _s = {}(\n        1_000.0,\n        2_500.0,\n    );\n}}\n",
            concat!("Color32::", "from_rgb"),
            concat!("vec", "2"),
            concat!("Color32::", "from_rgb"),
            concat!("Vec2::", "new"),
        );
        let tokens_mod_src = format!(
            "pub const fn bridge(r: u8, g: u8, b: u8) {{ let _ = {}(r, g, b); }} // {} canonical bridge\npub fn bad_lit() {{ let _ = {}(12, 34, 56); }} // {} fake excuse\npub fn bad_sz() {{ let _ = {}(14.0, 28.0); }}\n",
            concat!("Color32::", "from_rgb"),
            concat!("design-token-", "exempt:"),
            concat!("Color32::", "from_rgb"),
            concat!("design-token-", "exempt:"),
            concat!("vec", "2"),
        );
        let sources = vec![
            (String::from("crates/ui-egui/src/egui_shell/sample_mod.rs"), mod_with_early_cfg_test),
            (String::from(TOKENS_MOD_REL), tokens_mod_src),
        ];
        let report = scan_sources(&sources);
        assert_eq!(report.raw_colors, 1, "production colour after `#[cfg(test)] mod tests;` and char literal ` '\"'` must be counted");
        assert_eq!(report.raw_size_pairs, 2, "multi-line underscore Vec2::new + TOKENS_MOD_REL vec2 must both be counted");
        assert_eq!(
            report.tokens_mod_raw_colors, 1,
            "TOKENS_MOD_REL literal colour must not be bypassed by exemption, while non-literal bridge is allowed"
        );
        assert!(report.invalid_exemptions.is_empty());
    }

    #[test]
    fn aud_12_27_counts_whitespace_spaced_constructors_named_colors_struct_literals_splat_and_casts() {
        let code = "\
pub fn widget() {
    let _c1 = Color32 :: from_rgb (10, 20, 30);
    let _c2 = Color32::from_black_alpha(32);
    let _c3 = Color32::from_white_alpha(16);
    let _c4 = Color32::from_rgb_additive(1, 2, 3);
    let _c5 = Color32::from_rgba_unmultiplied_const(1, 2, 3, 255);
    let _c6 = Color32::from_hex(\"#123456\");
    let _c7 = Color32::WHITE;
    let _c8 = Rgba8 { r: 10, g: 20, b: 30, a: 255 };
    let _ok_trans = Color32::TRANSPARENT;

    let _s1 = vec2 (14.0, 28.0);
    let _s2 = vec2((14.0), (28.0));
    let _s3 = vec2(14 as f32, 28 as f32);
    let _s4 = Vec2::splat(16.0);
    let _s5 = Vec2 { x: 12.0, y: 24.0 };
    let _ok_zero_struct = Vec2 { x: 0.0, y: 0.0 };
    let _ok_splat_zero = Vec2::splat(0.0);
}
";
        let tokens_bad = "\
pub fn bad_tokens() {
    let _a = Color32 :: from_rgb (1, 2, 3);
    let _b = Color32::from_hex(\"#ffffff\");
    let _c = Rgba8 { r: 1, g: 2, b: 3, a: 255 };
    let _d = Color32::WHITE;
    let _ok_t = Color32::TRANSPARENT;
}
";
        let report = scan_sources(&[
            (String::from("crates/ui-egui/src/egui_shell/widget_aud12.rs"), code.to_owned()),
            (String::from(TOKENS_MOD_REL), tokens_bad.to_owned()),
        ]);
        assert_eq!(report.raw_colors, 8, "expected all 8 raw color forms counted");
        assert_eq!(report.raw_size_pairs, 5, "expected all 5 raw size forms counted");
        assert_eq!(report.tokens_mod_raw_colors, 4, "expected all 4 raw color bypasses caught in tokens.rs");
    }
}
