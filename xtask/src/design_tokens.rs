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

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

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
        let prod_slice = strip_cfg_test_tail(content);
        let is_tokens_mod = rel == TOKENS_MOD_REL;
        let mut file_colors = 0usize;
        let mut file_sizes = 0usize;

        for (line_idx, line) in prod_slice.lines().enumerate() {
            let line_no = line_idx + 1;
            let (code, comment) = split_code_and_line_comment(line);
            if let Some(comment_text) = comment {
                if let Some((_, tail)) = comment_text.split_once(EXEMPT_MARKER) {
                    let reason = tail.trim();
                    if reason.is_empty() {
                        report.invalid_exemptions.push(format!("{rel}:{line_no}: `{EXEMPT_MARKER}` requires a non-empty reason"));
                    } else {
                        continue;
                    }
                } else if comment_text.contains(BARE_EXEMPT_MARKER) {
                    report.invalid_exemptions.push(format!("{rel}:{line_no}: use `{EXEMPT_MARKER} <reason>` with an explicit justification"));
                }
            }

            let sanitized = blank_string_literals(&code);
            let c_hits = count_color_calls(&sanitized);
            if is_tokens_mod {
                report.tokens_mod_raw_colors += c_hits;
                continue;
            }
            let s_hits = count_raw_size_calls(&sanitized);
            file_colors += c_hits;
            file_sizes += s_hits;
        }

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

fn strip_cfg_test_tail(src: &str) -> &str {
    let mut offset = 0usize;
    for line in src.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed == "#[cfg(test)]" {
            return src.get(..offset).unwrap_or(src);
        }
        offset += line.len();
    }
    src
}

fn split_code_and_line_comment(line: &str) -> (String, Option<&str>) {
    let mut in_str = false;
    let mut escaped = false;
    let mut chars = line.char_indices().peekable();
    while let Some((i, ch)) = chars.next() {
        if in_str {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_str = false;
            }
            continue;
        }
        if ch == '"' {
            in_str = true;
            continue;
        }
        if ch == '/' && chars.peek().is_some_and(|&(_, next)| next == '/') {
            let code_part = line.get(..i).unwrap_or(line).to_owned();
            let comment_part = line.get(i + 2..);
            return (code_part, comment_part);
        }
    }
    (line.to_owned(), None)
}

fn blank_string_literals(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut in_str = false;
    let mut escaped = false;
    for ch in code.chars() {
        if in_str {
            if escaped {
                escaped = false;
                out.push(' ');
            } else if ch == '\\' {
                escaped = true;
                out.push(' ');
            } else if ch == '"' {
                in_str = false;
                out.push('"');
            } else {
                out.push(' ');
            }
        } else {
            if ch == '"' {
                in_str = true;
            }
            out.push(ch);
        }
    }
    out
}

fn count_color_calls(code: &str) -> usize {
    COLOR_PATTERNS.iter().map(|pat| code.matches(pat).count()).sum()
}

fn count_raw_size_calls(code: &str) -> usize {
    let mut count = 0usize;
    for pat in SIZE_CALL_PATTERNS {
        let mut search_from = 0usize;
        while let Some(slice) = code.get(search_from..)
            && let Some(rel_pos) = slice.find(pat)
        {
            let abs_pos = search_from + rel_pos;
            let valid_boundary =
                abs_pos == 0 || code.as_bytes().get(abs_pos.saturating_sub(1)).is_none_or(|&prev| !(prev.is_ascii_alphanumeric() || prev == b'_'));
            let args_start = abs_pos + pat.len();
            if valid_boundary
                && let Some(args_tail) = code.get(args_start..)
                && let Some(args) = extract_balanced_paren_args(args_tail)
                && args_contain_nonzero_numeric_literal(args)
            {
                count += 1;
            }
            search_from = args_start;
        }
    }
    count
}

fn extract_balanced_paren_args(tail: &str) -> Option<&str> {
    let mut depth = 1usize;
    for (idx, ch) in tail.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return tail.get(..idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn args_contain_nonzero_numeric_literal(args: &str) -> bool {
    for part in split_top_level_commas(args) {
        let trimmed = part.trim().trim_start_matches('-').trim();
        let trimmed = trimmed.strip_suffix("_f32").or_else(|| trimmed.strip_suffix("f32")).unwrap_or(trimmed);
        if let Ok(val) = trimmed.parse::<f64>()
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
}
