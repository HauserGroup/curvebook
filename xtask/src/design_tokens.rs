//! `cargo xtask design-tokens`: ratcheting gate on raw colour constructors and raw size pairs
//! inside `crates/ui-egui/src/` (`docs/design-language.md` §5).
//!
//! Ensures UI widgets read colours and measurements from `Tokens` and `LayoutMetrics` rather than
//! scattering ad-hoc RGB triples and pixel pairs across views and modals:
//! - `crates/ui-egui/src/egui_shell/mod.rs` (`Tokens`) must contain **zero** raw colour literals
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
pub const MAX_RAW_COLORS: usize = 39;

/// Maximum non-exempt raw numeric size pairs (`vec2(...)` / `Vec2::new(...)`) permitted across
/// `crates/ui-egui/src/`. Lower this ceiling whenever raw widget sizes are migrated onto `LayoutMetrics`.
pub const MAX_RAW_SIZE_PAIRS: usize = 236;

const UI_SRC_PREFIX: &str = "crates/ui-egui/src/";
const TOKENS_MOD_REL: &str = "crates/ui-egui/src/egui_shell/mod.rs";
const EXEMPT_MARKER: &str = concat!("design-token-", "exempt:");
const BARE_EXEMPT_MARKER: &str = concat!("design-token-", "exempt");

const EXEMPT_FILES: &[&str] = &[
    "crates/ui-egui/src/theme.rs",
    "crates/ui-egui/src/egui_shell/brand.rs",
    "crates/ui-egui/src/egui_shell/rasterizer.rs",
    "crates/ui-egui/src/egui_shell/driver.rs",
];

const COLOR_PATTERNS: &[&str] = &[
    concat!("Color32::", "from_rgb("),
    concat!("Color32::", "from_rgba_unmultiplied("),
    concat!("Color32::", "from_rgba_premultiplied("),
    concat!("Color32::", "from_gray("),
    concat!("Rgba8::", "rgb("),
    concat!("Rgba8::", "rgba("),
];

const SIZE_CALL_PATTERNS: &[&str] = &[concat!("Vec2::", "new("), concat!("vec", "2(")];

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

        let (file_colors, tokens_mod_colors) = count_color_calls(&prod_code, &line_starts, &exempt_lines, is_tokens_mod);
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

fn count_color_calls(code: &str, line_starts: &[usize], exempt_lines: &BTreeSet<usize>, is_tokens_mod: bool) -> (usize, usize) {
    let mut regular_count = 0usize;
    let mut tokens_mod_count = 0usize;
    for pat in COLOR_PATTERNS {
        let mut search_from = 0usize;
        while let Some(slice) = code.get(search_from..)
            && let Some(rel_pos) = slice.find(pat)
        {
            let abs_pos = search_from + rel_pos;
            let valid_boundary = abs_pos == 0 || code.as_bytes().get(abs_pos.saturating_sub(1)).is_none_or(|&prev| !is_ident_byte(prev));
            let args_start = abs_pos + pat.len();
            if valid_boundary {
                let args_opt = code.get(args_start..).and_then(extract_balanced_paren_args_with_end);
                let start_line = line_of_offset(line_starts, abs_pos);
                let end_line = match args_opt {
                    Some((_, close_rel)) => line_of_offset(line_starts, args_start + close_rel),
                    None => start_line,
                };
                let exempt = is_call_exempt(exempt_lines, start_line, end_line);
                if is_tokens_mod {
                    // `TOKENS_MOD_REL` forbids all raw colour literals even with an exemption
                    // comment; only non-literal type-conversion bridges (e.g. `Rgba8 <-> Color32`)
                    // with an explicit `design-token-exempt: <reason>` comment are permitted.
                    let has_literal_arg = args_opt.is_none_or(|(args, _)| contains_numeric_literal_token(args));
                    if !exempt || has_literal_arg {
                        tokens_mod_count += 1;
                    }
                } else if !exempt {
                    regular_count += 1;
                }
            }
            search_from = args_start;
        }
    }
    (regular_count, tokens_mod_count)
}

fn count_raw_size_calls(code: &str, line_starts: &[usize], exempt_lines: &BTreeSet<usize>) -> usize {
    let mut count = 0usize;
    for pat in SIZE_CALL_PATTERNS {
        let mut search_from = 0usize;
        while let Some(slice) = code.get(search_from..)
            && let Some(rel_pos) = slice.find(pat)
        {
            let abs_pos = search_from + rel_pos;
            let valid_boundary = abs_pos == 0 || code.as_bytes().get(abs_pos.saturating_sub(1)).is_none_or(|&prev| !is_ident_byte(prev));
            let args_start = abs_pos + pat.len();
            if valid_boundary
                && let Some(args_tail) = code.get(args_start..)
                && let Some((args, close_rel)) = extract_balanced_paren_args_with_end(args_tail)
            {
                let start_line = line_of_offset(line_starts, abs_pos);
                let end_line = line_of_offset(line_starts, args_start + close_rel);
                if !is_call_exempt(exempt_lines, start_line, end_line) && args_contain_nonzero_numeric_literal(args) {
                    count += 1;
                }
            }
            search_from = args_start;
        }
    }
    count
}

fn extract_balanced_paren_args_with_end(tail: &str) -> Option<(&str, usize)> {
    let mut depth = 1usize;
    for (idx, ch) in tail.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return tail.get(..idx).map(|args| (args, idx));
                }
            }
            _ => {}
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

fn parse_numeric_literal(part: &str) -> Option<f64> {
    let trimmed = part.trim();
    let trimmed = trimmed.strip_prefix('-').or_else(|| trimmed.strip_prefix('+')).unwrap_or(trimmed).trim();
    let trimmed = trimmed
        .strip_suffix("_f32")
        .or_else(|| trimmed.strip_suffix("f32"))
        .or_else(|| trimmed.strip_suffix("_f64"))
        .or_else(|| trimmed.strip_suffix("f64"))
        .or_else(|| trimmed.strip_suffix("_u8"))
        .or_else(|| trimmed.strip_suffix("u8"))
        .or_else(|| trimmed.strip_suffix("_u16"))
        .or_else(|| trimmed.strip_suffix("u16"))
        .or_else(|| trimmed.strip_suffix("_u32"))
        .or_else(|| trimmed.strip_suffix("u32"))
        .or_else(|| trimmed.strip_suffix("_i32"))
        .or_else(|| trimmed.strip_suffix("i32"))
        .or_else(|| trimmed.strip_suffix("_usize"))
        .or_else(|| trimmed.strip_suffix("usize"))
        .unwrap_or(trimmed);
    if !trimmed.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        return None;
    }
    if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        let clean: String = hex.chars().filter(|&c| c != '_').collect();
        return u128::from_str_radix(&clean, 16).ok().map(|v| v as f64);
    }
    if let Some(bin) = trimmed.strip_prefix("0b").or_else(|| trimmed.strip_prefix("0B")) {
        let clean: String = bin.chars().filter(|&c| c != '_').collect();
        return u128::from_str_radix(&clean, 2).ok().map(|v| v as f64);
    }
    if let Some(oct) = trimmed.strip_prefix("0o").or_else(|| trimmed.strip_prefix("0O")) {
        let clean: String = oct.chars().filter(|&c| c != '_').collect();
        return u128::from_str_radix(&clean, 8).ok().map(|v| v as f64);
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
}
