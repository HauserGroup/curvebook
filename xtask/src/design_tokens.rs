//! `cargo xtask design-tokens`: ratcheting gate on raw colour constructors and raw size pairs
//! inside `crates/ui-egui/src/` (`docs/design-language.md` §5).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use crate::rust_lex::{RustLexer, blank_comments_and_literals, find_matching_delim, is_ident_byte, mask_cfg_test_items};

pub const MAX_RAW_COLORS: usize = 55;
pub const MAX_RAW_SIZE_PAIRS: usize = 232;

const UI_SRC_PREFIX: &str = "crates/ui-egui/src/";
const TOKENS_MOD_REL: &str = "crates/ui-egui/src/egui_shell/tokens.rs";
const EXEMPT_MARKER: &str = concat!("design-token-", "exempt:");
const BARE_EXEMPT_MARKER: &str = concat!("design-token-", "exempt");
const EXEMPT_FILES: &str = "crates/ui-egui/src/theme.rs crates/ui-egui/src/egui_shell/brand.rs crates/ui-egui/src/egui_shell/rasterizer.rs crates/ui-egui/src/egui_shell/driver.rs";
const COLOR32_CTORS: &str = "from_rgb from_rgba_unmultiplied from_rgba_premultiplied from_gray from_black_alpha from_white_alpha from_rgb_additive from_rgba_unmultiplied_const from_hex";
const COLOR32_CONSTS: &str = "WHITE BLACK RED GREEN BLUE YELLOW CYAN MAGENTA GRAY LIGHT_GRAY DARK_GRAY LIGHT_RED DARK_RED LIGHT_GREEN DARK_GREEN LIGHT_BLUE DARK_BLUE LIGHT_YELLOW GOLD BROWN KHAKI ORANGE PURPLE PLACEHOLDER";
const RGBA8_CTORS: &str = "rgb rgba";
const VEC2_CTORS: &str = "new splat";

fn has_word(list: &str, needle: &str) -> bool {
    list.split_ascii_whitespace().any(|w| w == needle)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesignTokenScanReport {
    pub raw_colors: usize,
    pub raw_size_pairs: usize,
    pub tokens_mod_raw_colors: usize,
    pub per_file_colors: BTreeMap<String, usize>,
    pub per_file_sizes: BTreeMap<String, usize>,
    pub invalid_exemptions: Vec<String>,
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let verbose = match args {
        [] => false,
        [flag] if flag == "--verbose" => true,
        _ => return Err(String::from("usage: cargo xtask design-tokens [--verbose]")),
    };
    let mut sources = Vec::new();
    for rel in crate::repo_files(root)? {
        if rel.starts_with(UI_SRC_PREFIX) && rel.ends_with(".rs") {
            sources.push((rel.clone(), fs::read_to_string(root.join(&rel)).map_err(|e| format!("read {rel}: {e}"))?));
        }
    }
    let report = scan_sources(&sources);
    if verbose {
        report.per_file_colors.iter().for_each(|(f, n)| println!("  raw-color  {n:>4}  {f}"));
        report.per_file_sizes.iter().for_each(|(f, n)| println!("  raw-size   {n:>4}  {f}"));
    }
    enforce_ceilings(&report, MAX_RAW_COLORS, MAX_RAW_SIZE_PAIRS)?;
    println!(
        "design-tokens: OK (raw_colors={}/{MAX_RAW_COLORS}, raw_size_pairs={}/{MAX_RAW_SIZE_PAIRS}, tokens_mod_raw_colors=0)",
        report.raw_colors, report.raw_size_pairs
    );
    Ok(())
}

#[must_use]
pub fn scan_sources(sources: &[(String, String)]) -> DesignTokenScanReport {
    let mut report = DesignTokenScanReport::default();
    for (rel, content) in sources {
        if rel.ends_with("/tests.rs") || rel.contains("/tests/") || has_word(EXEMPT_FILES, rel) {
            continue;
        }
        let blanked = blank_comments_and_literals(content);
        let (mut prod_bytes, mut raw_bytes) = (blanked.as_bytes().to_vec(), content.as_bytes().to_vec());
        mask_cfg_test_items(blanked.as_bytes(), &mut [&mut prod_bytes, &mut raw_bytes]);
        let prod_code = String::from_utf8(prod_bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
        let raw_prod = String::from_utf8(raw_bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
        let exempt_lines = collect_exemption_lines(rel, &raw_prod, &mut report.invalid_exemptions);
        let line_starts: Vec<usize> =
            std::iter::once(0).chain(prod_code.as_bytes().iter().enumerate().filter_map(|(i, &b)| (b == b'\n').then_some(i + 1))).collect();
        let toks = tokenize_blanked_code(&prod_code);
        let (file_colors, tokens_mod_colors) = count_color_calls(&prod_code, &raw_prod, &toks, &line_starts, &exempt_lines, rel == TOKENS_MOD_REL);
        let file_sizes = count_raw_size_calls(&prod_code, &toks, &line_starts, &exempt_lines);
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

pub fn enforce_ceilings(report: &DesignTokenScanReport, max_colors: usize, max_sizes: usize) -> Result<(), String> {
    let fmt_map = |m: &BTreeMap<String, usize>| m.iter().map(|(f, n)| format!("  {n:>4}  {f}")).collect::<Vec<_>>().join("\n");
    let mut errors = Vec::new();
    if !report.invalid_exemptions.is_empty() {
        errors.push(format!("malformed design-token exemption comment(s):\n  {}", report.invalid_exemptions.join("\n  ")));
    }
    if report.tokens_mod_raw_colors > 0 {
        errors.push(format!("`{TOKENS_MOD_REL}` contains {} raw colour literal(s); derive `Tokens` colours from `ThemeSeeds` in `crates/ui-egui/src/theme.rs` instead", report.tokens_mod_raw_colors));
    }
    if report.raw_colors > max_colors {
        errors.push(format!("raw UI colour count {} exceeds ceiling {max_colors}; use `Tokens` fields (or document canvas math with `// {EXEMPT_MARKER} <reason>`):\n{}", report.raw_colors, fmt_map(&report.per_file_colors)));
    }
    if report.raw_size_pairs > max_sizes {
        errors.push(format!("raw UI size-pair count {} exceeds ceiling {max_sizes}; use `LayoutMetrics` tokens (or document canvas geometry with `// {EXEMPT_MARKER} <reason>`):\n{}", report.raw_size_pairs, fmt_map(&report.per_file_sizes)));
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("\n\n")) }
}

fn collect_exemption_lines(rel: &str, raw_prod: &str, invalid_exemptions: &mut Vec<String>) -> BTreeSet<usize> {
    let (mut exempt, mut lex) = (BTreeSet::new(), RustLexer::new(raw_prod));
    while !lex.is_eof() {
        if let Some((line_no, text)) = lex.try_read_comment() {
            if let Some((_, tail)) = text.split_once(EXEMPT_MARKER) {
                if tail.trim().is_empty() {
                    invalid_exemptions.push(format!("{rel}:{line_no}: `{EXEMPT_MARKER}` requires a non-empty reason"));
                } else {
                    exempt.insert(line_no);
                }
            } else if text.contains(BARE_EXEMPT_MARKER) {
                invalid_exemptions.push(format!("{rel}:{line_no}: use `{EXEMPT_MARKER} <reason>` with an explicit justification"));
            }
        } else if lex.try_read_raw_string().is_none() && !lex.try_skip_char_literal() && lex.try_read_quoted_string().is_none() {
            lex.advance_one();
        }
    }
    exempt
}

fn is_call_exempt(line_starts: &[usize], exempt_lines: &BTreeSet<usize>, start_off: usize, end_off: usize) -> bool {
    exempt_lines.range(line_starts.partition_point(|&o| o <= start_off)..=line_starts.partition_point(|&o| o <= end_off)).next().is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokKind<'a> {
    Ident(&'a str),
    ColonColon,
    LParen,
    LBrace,
    Dot,
    Other,
}

#[derive(Debug, Clone, Copy)]
struct Tok<'a> {
    kind: TokKind<'a>,
    start: usize,
    end: usize,
}

fn tokenize_blanked_code(code: &str) -> Vec<Tok<'_>> {
    let (b, mut out, mut i) = (code.as_bytes(), Vec::new(), 0usize);
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            i += 1;
        } else if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            let start = i;
            i += 1;
            while i < b.len() && is_ident_byte(b[i]) {
                i += 1;
            }
            if let Some(ident) = code.get(start..i) {
                out.push(Tok { kind: TokKind::Ident(ident), start, end: i });
            }
        } else if b.get(i..i + 2) == Some(b"::") {
            out.push(Tok { kind: TokKind::ColonColon, start: i, end: i + 2 });
            i += 2;
        } else {
            let kind = match b[i] {
                b'(' => TokKind::LParen,
                b'{' => TokKind::LBrace,
                b'.' => TokKind::Dot,
                _ => TokKind::Other,
            };
            let step = code.get(i..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
            out.push(Tok { kind, start: i, end: i + step });
            i += step;
        }
    }
    out
}

fn balanced_body(code: &str, open_pos: usize, open: u8, close: u8) -> Option<(&str, usize)> {
    let end = find_matching_delim(code.as_bytes(), open_pos, open, close)?;
    Some((code.get(open_pos + 1..end)?, end))
}

fn count_color_calls(
    code: &str,
    raw_prod: &str,
    toks: &[Tok<'_>],
    line_starts: &[usize],
    exempt_lines: &BTreeSet<usize>,
    is_tokens_mod: bool,
) -> (usize, usize) {
    let (mut regular, mut tokens_mod, mut i) = (0usize, 0usize, 0usize);
    let mut record = |exempt: bool, has_literal: bool| {
        if is_tokens_mod {
            tokens_mod += usize::from(!exempt || has_literal);
        } else if !exempt {
            regular += 1;
        }
    };
    while i < toks.len() {
        let TokKind::Ident(head) = toks[i].kind else {
            i += 1;
            continue;
        };
        if (head == "Color32" || head == "Rgba8")
            && matches!(toks.get(i + 1).map(|t| t.kind), Some(TokKind::ColonColon))
            && let Some(Tok { kind: TokKind::Ident(member), end: member_end, .. }) = toks.get(i + 2).copied()
        {
            let is_ctor = (head == "Color32" && has_word(COLOR32_CTORS, member)) || (head == "Rgba8" && has_word(RGBA8_CTORS, member));
            if is_ctor && let Some(Tok { kind: TokKind::LParen, start: open_pos, .. }) = toks.get(i + 3).copied() {
                let body_opt = balanced_body(code, open_pos, b'(', b')');
                let has_lit =
                    body_opt.is_none_or(|(args, end)| has_numeric_token(args) || raw_prod.get(open_pos + 1..end).is_some_and(|r| r.contains('"')));
                record(is_call_exempt(line_starts, exempt_lines, toks[i].start, body_opt.map_or(toks[i].start, |(_, e)| e)), has_lit);
                i += 4;
                continue;
            }
            if head == "Color32" && has_word(COLOR32_CONSTS, member) {
                record(is_call_exempt(line_starts, exempt_lines, toks[i].start, member_end.saturating_sub(1)), true);
                i += 3;
                continue;
            }
        } else if head == "Rgba8"
            && let Some(Tok { kind: TokKind::LBrace, start: open_pos, .. }) = toks.get(i + 1).copied()
        {
            let body_opt = balanced_body(code, open_pos, b'{', b'}');
            record(
                is_call_exempt(line_starts, exempt_lines, toks[i].start, body_opt.map_or(toks[i].start, |(_, e)| e)),
                body_opt.is_none_or(|(b, _)| has_numeric_token(b)),
            );
            i += 2;
            continue;
        }
        i += 1;
    }
    (regular, tokens_mod)
}

fn count_raw_size_calls(code: &str, toks: &[Tok<'_>], line_starts: &[usize], exempt_lines: &BTreeSet<usize>) -> usize {
    let (mut count, mut i) = (0usize, 0usize);
    while i < toks.len() {
        let TokKind::Ident(head) = toks[i].kind else {
            i += 1;
            continue;
        };
        let (delim, open_pos, is_struct, step) = if head == "Vec2"
            && matches!(toks.get(i + 1).map(|t| t.kind), Some(TokKind::ColonColon))
            && matches!(toks.get(i + 2).map(|t| t.kind), Some(TokKind::Ident(m)) if has_word(VEC2_CTORS, m))
            && let Some(Tok { kind: TokKind::LParen, start, .. }) = toks.get(i + 3).copied()
        {
            ((b'(', b')'), start, false, 4)
        } else if head == "Vec2"
            && let Some(Tok { kind: TokKind::LBrace, start, .. }) = toks.get(i + 1).copied()
        {
            ((b'{', b'}'), start, true, 2)
        } else if head == "vec2"
            && !(i > 0 && toks[i - 1].kind == TokKind::Dot)
            && let Some(Tok { kind: TokKind::LParen, start, .. }) = toks.get(i + 1).copied()
        {
            ((b'(', b')'), start, false, 2)
        } else {
            i += 1;
            continue;
        };
        if let Some((body, end)) = balanced_body(code, open_pos, delim.0, delim.1)
            && !is_call_exempt(line_starts, exempt_lines, toks[i].start, end)
            && has_nonzero_literal(body, is_struct)
        {
            count += 1;
        }
        i += step;
    }
    count
}

fn has_numeric_token(s: &str) -> bool {
    let b = s.as_bytes();
    b.iter().enumerate().any(|(i, &c)| c.is_ascii_digit() && !(i > 0 && is_ident_byte(b[i - 1])))
}

fn strip_outer_parens(s: &str) -> Option<&str> {
    (s.starts_with('(') && find_matching_delim(s.as_bytes(), 0, b'(', b')') == Some(s.len() - 1)).then(|| s.get(1..s.len() - 1)).flatten()
}

fn strip_trailing_as_cast(s: &str) -> Option<&str> {
    let (b, mut depth, mut last_as) = (s.as_bytes(), 0usize, None);
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b'a' if depth == 0
                && b.get(i + 1) == Some(&b's')
                && (i == 0 || !is_ident_byte(b[i - 1]))
                && b.get(i + 2).is_none_or(|&n| !is_ident_byte(n)) =>
            {
                last_as = Some(i)
            }
            _ => {}
        }
    }
    let pos = last_as?;
    let rhs = s.get(pos + 2..)?.trim();
    (!rhs.is_empty() && rhs.bytes().all(is_ident_byte)).then(|| s.get(..pos).map(str::trim)).flatten()
}

fn parse_numeric_literal(part: &str) -> Option<f64> {
    const SUFFIXES: &str = "_f32 f32 _f64 f64 _u8 u8 _u16 u16 _u32 u32 _i32 i32 _usize usize";
    let mut t = part.trim();
    while let Some(inner) = strip_outer_parens(t).or_else(|| strip_trailing_as_cast(t)).or_else(|| t.strip_prefix(['-', '+'])) {
        t = inner.trim();
    }
    if let Some(stem) = SUFFIXES.split_ascii_whitespace().find_map(|sfx| t.strip_suffix(sfx)) {
        t = stem;
    }
    if !t.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        return None;
    }
    for (lo, hi, radix) in [("0x", "0X", 16), ("0b", "0B", 2), ("0o", "0O", 8)] {
        if let Some(d) = t.strip_prefix(lo).or_else(|| t.strip_prefix(hi)) {
            return u128::from_str_radix(&d.replace('_', ""), radix).ok().map(|v| v as f64);
        }
    }
    t.replace('_', "").parse::<f64>().ok()
}

fn has_nonzero_literal(body: &str, is_struct: bool) -> bool {
    let (mut depth, mut start) = (0usize, 0usize);
    let check_seg = |seg: &str| {
        let t = seg.trim();
        !(is_struct && (t.is_empty() || t.starts_with("..")))
            && parse_numeric_literal(if is_struct { t.split_once(':').map_or(t, |(_, r)| r) } else { t }).is_some_and(|v| v.abs() > 0.0)
    };
    for (idx, ch) in body.char_indices() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                if body.get(start..idx).is_some_and(check_seg) {
                    return true;
                }
                start = idx + 1;
            }
            _ => {}
        }
    }
    body.get(start..).is_some_and(check_seg)
}

#[cfg(test)]
mod tests {
    use super::{TOKENS_MOD_REL, enforce_ceilings, scan_sources};

    #[test]
    fn counts_raw_colors_sizes_exemptions_tokens_mod_and_rust_lex_forms() {
        let widget = concat!(
            "#[cfg(test)]\nmod tests;\npub fn w() {\n  /* Color32::from_rgb(1, 2, 3); vec2(9.0, 9.0); */\n  let _q = '\"';\n",
            "  let _c1 = Color32 :: from_rgb (10, 20, 30);\n  let _c2 = Color32::from_black_alpha(32);\n  let _c3 = Color32::from_hex(\"#123456\");\n",
            "  let _c4 = Color32::WHITE;\n  let _c5 = Rgba8 { r: 10, g: 20, b: 30, a: 255 };\n  let _ok_t = Color32::TRANSPARENT;\n",
            "  let _ok_ex = Color32::from_rgb(1, 2, 3); // design-token-",
            "exempt: crosshair\n",
            "  let _s1 = egui::vec2(14.0, 28.0);\n  let _s2 = vec2((14.0), (28.0));\n  let _s3 = vec2(14 as f32, 28 as f32);\n",
            "  let _s4 = Vec2::splat(16.0);\n  let _s5 = Vec2 { x: 12.0, y: 24.0 };\n  let _s6 = Vec2::new(\n    1_000.0,\n    2_500.0,\n  );\n",
            "  let _ok_tok = egui::vec2(tokens.a, tokens.b);\n  let _ok_zero = Vec2::new(0.0, 0.0);\n  let _ok_zstruct = Vec2 { x: 0.0, y: 0.0 };\n  let _ok_zsplat = Vec2::splat(0.0);\n}\n",
            "#[cfg(test)]\nmod inline_t { fn t() { let _ = Color32::from_rgb(1,2,3); let _ = vec2(9.0,9.0); } }\n",
        );
        let tokens_src = concat!(
            "pub const fn bridge(r: u8, g: u8, b: u8) { let _ = Color32::from_rgb(r, g, b); } // design-token-",
            "exempt: bridge\n",
            "pub fn bad1() { let _ = Color32::from_rgb(12, 34, 56); } // design-token-",
            "exempt: fake\n",
            "pub fn bad2() { let _ = Color32::from_hex(\"#ffffff\"); }\n",
            "pub fn bad3() { let _ = Rgba8 { r: 1, g: 2, b: 3, a: 255 }; }\n",
            "pub fn bad4() { let _ = Color32::WHITE; }\n",
            "pub fn bad_sz() { let _ = vec2(14.0, 28.0); } // design-token-",
            "exempt:\n",
        );
        let report = scan_sources(&[
            (String::from("crates/ui-egui/src/egui_shell/sample.rs"), widget.to_owned()),
            (String::from("crates/ui-egui/src/theme.rs"), String::from("fn s() { let _ = Rgba8::rgb(1,2,3); }")),
            (String::from("crates/ui-egui/src/dialogs/tests.rs"), String::from("fn f() { let _ = Rgba8::rgb(9,9,9); }")),
            (String::from(TOKENS_MOD_REL), tokens_src.to_owned()),
        ]);
        assert_eq!((report.raw_colors, report.raw_size_pairs, report.tokens_mod_raw_colors, report.invalid_exemptions.len()), (5, 7, 4, 1));
        let err = enforce_ceilings(&report, 4, 6).unwrap_err();
        for needle in
            [TOKENS_MOD_REL, "requires a non-empty reason", "raw UI colour count 5 exceeds ceiling 4", "raw UI size-pair count 7 exceeds ceiling 6"]
        {
            assert!(err.contains(needle), "missing `{needle}` in: {err}");
        }
    }
}
