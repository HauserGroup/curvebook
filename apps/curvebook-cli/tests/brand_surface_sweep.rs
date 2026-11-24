//! Relocated Wave 6 `R0.21` strict alpha branding + live UI / MCP / 16-locale `egui_shell` /
//! export metadata brand surface sweeps (moved from `xtask/src/brands.rs` and
//! `xtask/tests/r021_strict_alpha_branding.rs` so `xtask` stays decoupled from heavy app crates).

#![allow(dead_code)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use curvebook_chart::compile_graph;
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, ConstantValue, GraphSheet, InfoSheet, LayoutItem, LayoutSheet, LayoutSpec, ProjectDocument, ResultSheet, SheetId,
    TableKind,
};
use curvebook_engine::{Session, cmd::registry as engine_cmd_registry};
use curvebook_format::{ZipArchive, ZipLimits, save_project_bytes, save_template_bytes};
use curvebook_mcp::{GuiSessionHost, McpServerState, dispatch_json_value};
use curvebook_pdf::{PdfExportOptions, render_pdf};
use curvebook_svg::{SvgExportOptions, render_svg};
use curvebook_ui_egui::dialogs_canvas::{ANALYSIS_CATALOG, ActiveInspector, AnalysisCategory, CommandPaletteState};
use curvebook_ui_egui::i18n;
use curvebook_ui_egui::menus::{TOOLBAR_BUTTONS, UI_COMMANDS, menu_bar_tooltips, toolbar_tooltips};
use curvebook_ui_egui::workspace_nav::{SAMPLE_FIXTURES, TableCreationDraft, WelcomeMode, instantiate_sample_fixture};
use curvebook_ui_egui::{
    ActiveDialog, ClipboardBundle, EguiSurfaceCapture, EguiSurfaceScanner, UiApp, UiTreeSnapshot, ViewportMetrics, inspect_ui_tree,
};

fn decode_utf16(bytes: &[u8], little_endian: bool) -> String {
    let (pairs, remainder) = bytes.as_chunks::<2>();
    let units = pairs.iter().map(|&arr| if little_endian { u16::from_le_bytes(arr) } else { u16::from_be_bytes(arr) });
    let mut out = String::with_capacity(pairs.len());
    for ch in char::decode_utf16(units) {
        out.push(ch.unwrap_or('\u{FFFD}'));
    }
    if !remainder.is_empty() {
        out.push('\u{FFFD}');
    }
    out
}

fn forbidden_vendor_tokens() -> Vec<String> {
    vec![
        join(&["graph", "pad"]),
        join(&["dot", "matics"]),
        join(&["pri", "sm"]),
        join(&["in", "stat"]),
        join(&["stat", "mate"]),
        join(&["jm", "p"]),
        join(&["sp", "ss"]),
        join(&["mini", "tab"]),
        join(&["origin", "lab"]),
        join(&["origin", "pro"]),
        join(&["sigma", "plot"]),
        join(&["sys", "tat"]),
        join(&["statis", "tica"]),
        join(&["xl", "stat"]),
    ]
}

fn pre_rename_tokens() -> Vec<String> {
    vec![join(&["lumen", "craft"])]
}

const OPTICAL_MODIFIERS: &[&str] = &["optical", "refractive", "triangular", "glass", "crystal", "dispersive", "dispersion", "dichroic", "achromatic"];

fn suite_tokens() -> Vec<String> {
    vec![
        join(&["art", "craft"]),
        join(&["get", "art", "craft"]),
        join(&["story", "told"]),
        join(&["story", "teller"]),
        join(&["cra", "ft-fonts"]),
        join(&["cra", "ft_fonts"]),
        join(&["cra", "ft_fonts_dir"]),
        join(&["cra", "ft-common-actions"]),
        join(&["cra", "ft-contrib"]),
        join(&["cra", "ft-house-rules"]),
        join(&["cra", "ft-ui"]),
        join(&["cra", "ft_ui"]),
        join(&["cra", "ft-keymap"]),
        join(&["cra", "ft-update"]),
        join(&["cra", "ft-window"]),
    ]
}

fn adapted_upstream_token() -> String {
    join(&["vector", "craft"])
}

fn ext_token() -> String {
    join(&["pri", "sm"])
}

fn stat_it_token() -> String {
    join(&["statis", "tica"])
}

fn dsl_token() -> String {
    join(&["jm", "p"])
}

fn import_cmd_id() -> String {
    join(&["table.import", "Pri", "sm"])
}

fn import_cmd_short() -> String {
    join(&["import", "Pri", "sm"])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrandHit {
    pub path: String,
    pub line: usize,
    pub brand: String,
}

pub(crate) fn is_legal_policy_doc_path(path: &str) -> bool {
    let norm = path.replace('\\', "/");
    let trimmed = norm.strip_prefix("./").unwrap_or(&norm);
    matches!(trimmed, "AGENTS.md" | "docs/ROADMAP.md" | "docs/planning/gui-layout-spec.md" | "plan/curvebook-rename.md")
        || trimmed == "contribute/agents.html"
        || trimmed.ends_with("/contribute/agents.html")
}

pub(crate) fn is_attribution_or_notice_path(path: &str, allow_packaged: bool) -> bool {
    let norm = path.replace('\\', "/");
    let trimmed = norm.strip_prefix("./").unwrap_or(&norm);
    if matches!(trimmed, "ATTRIBUTION.md" | "NOTICE") {
        return true;
    }
    if !allow_packaged || is_generated_docs_html_path(trimmed) || trimmed.split(['/', '!', ':']).any(|seg| matches!(seg, "crates" | "apps")) {
        return false;
    }
    trimmed.rsplit(['/', ':', '!']).next().is_some_and(|name| matches!(name, "ATTRIBUTION.md" | "NOTICE"))
}

fn is_generated_docs_html_path(path: &str) -> bool {
    let norm = path.replace('\\', "/");
    let trimmed = norm.strip_prefix("./").unwrap_or(&norm);
    (trimmed.starts_with("docs/manual/v") && trimmed != "docs/manual/index.html")
        || trimmed.starts_with("docs-site/")
        || trimmed.starts_with("target/docs-site/")
}

#[derive(Clone, Copy)]
enum TokenPolicy<'a> {
    Strict,
    AllowExactBareExt,
    Standard,
    RustComment {
        allow_it_stat_word: bool,
    },
    RustIdent {
        allow_dsl_short: bool,
        allow_canonical_ext_ident: bool,
    },
    RustLiteral {
        allow_bare_ext: bool,
        allow_dot_ext: bool,
        allow_import_ext_cmd: bool,
        allow_legacy_xml: bool,
        allow_test_container_op: bool,
    },
    UiOrAppRustLiteral {
        allow_bare_ext: bool,
        allow_dot_ext: bool,
        allow_import_ext_cmd: bool,
        allow_legacy_xml: bool,
        allow_it_stat_word: bool,
        allow_test_container_op: bool,
    },
    UiTextRun {
        allow_dot_ext: bool,
        allow_it_stat_word: bool,
        allow_palette_cmd_id: bool,
    },
    SvgTitleOrDesc {
        prev_word: Option<&'a str>,
    },
}

fn collect_line_words(path: &str, line_no: usize, haystack: &str, words: &[String], policy: TokenPolicy<'_>, out: &mut Vec<BrandHit>) {
    let lower = haystack.to_ascii_lowercase();
    for brand in words {
        if has_word_match(haystack, &lower, brand, policy) {
            out.push(BrandHit { path: path.to_owned(), line: line_no, brand: brand.clone() });
        }
    }
}

fn has_word_match(raw: &str, lower: &str, word: &str, policy: TokenPolicy<'_>) -> bool {
    let mut found = false;
    collect_word_matches(raw, lower, word, policy, |_| {
        found = true;
    });
    found
}

fn collect_word_matches(raw: &str, lower: &str, word: &str, policy: TokenPolicy<'_>, mut on_match: impl FnMut(usize)) {
    if word.contains(['-', '_']) {
        collect_delimited_suite_matches(lower, word, &mut on_match);
    }
    collect_snake_kebab_compound_matches(raw, lower, word, policy, &mut on_match);
    let bytes = raw.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        if !bytes[idx].is_ascii_alphanumeric() {
            idx += 1;
            continue;
        }
        let run_start = idx;
        while idx < bytes.len() && bytes[idx].is_ascii_alphanumeric() {
            idx += 1;
        }
        let run_end = idx;
        let subs = split_alnum_subtokens(bytes, run_start, run_end);
        for s_idx in 0..subs.len() {
            for count in 1..=3 {
                if s_idx + count > subs.len() {
                    break;
                }
                let span_start = subs[s_idx].0;
                let span_end = subs[s_idx + count - 1].1;
                if lower.get(span_start..span_end) == Some(word) && !is_token_match_exempt(raw, lower, word, span_start, span_end, policy) {
                    on_match(span_start);
                }
            }
        }
    }
}

fn collect_snake_kebab_compound_matches(raw: &str, lower: &str, word: &str, policy: TokenPolicy<'_>, on_match: &mut impl FnMut(usize)) {
    if word.contains(['-', '_']) || word.len() < 4 {
        return;
    }
    let bytes = raw.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        if !bytes[idx].is_ascii_alphanumeric() {
            idx += 1;
            continue;
        }
        let chain_start = idx;
        let mut parts: Vec<(usize, usize)> = Vec::new();
        while idx < bytes.len() {
            let p_start = idx;
            while idx < bytes.len() && bytes[idx].is_ascii_alphanumeric() {
                idx += 1;
            }
            parts.push((p_start, idx));
            if idx < bytes.len() && matches!(bytes[idx], b'_' | b'-') && bytes.get(idx + 1).is_some_and(u8::is_ascii_alphanumeric) {
                idx += 1;
            } else {
                break;
            }
        }
        let _ = chain_start;
        if parts.len() >= 2 {
            for i in 0..parts.len() {
                let mut joined = String::new();
                for j in i..(i + 3).min(parts.len()) {
                    joined.push_str(lower.get(parts[j].0..parts[j].1).unwrap_or(""));
                    if j > i && joined == word && !is_token_match_exempt(raw, lower, word, parts[i].0, parts[j].1, policy) {
                        on_match(parts[i].0);
                    }
                }
            }
        }
    }
}

fn collect_delimited_suite_matches(lower: &str, word: &str, on_match: &mut impl FnMut(usize)) {
    let bytes = lower.as_bytes();
    let mut start = 0usize;
    while let Some(rel) = lower.get(start..).and_then(|s| s.find(word)) {
        let pos = start + rel;
        let end = pos + word.len();
        let left_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
        let right_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if left_ok && right_ok {
            on_match(pos);
        }
        start = pos + 1;
    }
}

fn split_alnum_subtokens(bytes: &[u8], start: usize, end: usize) -> Vec<(usize, usize)> {
    let mut subs = Vec::new();
    let mut sub_start = start;
    for i in (start + 1)..end {
        let prev = bytes[i - 1];
        let curr = bytes[i];
        let next = bytes.get(i + 1).copied();
        let camel = (prev.is_ascii_lowercase() || prev.is_ascii_digit()) && curr.is_ascii_uppercase();
        let acronym = prev.is_ascii_uppercase() && curr.is_ascii_uppercase() && next.is_some_and(|n| n.is_ascii_lowercase());
        let digit_boundary = (prev.is_ascii_digit() && curr.is_ascii_alphabetic()) || (prev.is_ascii_alphabetic() && curr.is_ascii_digit());
        if camel || acronym || digit_boundary {
            subs.push((sub_start, i));
            sub_start = i;
        }
    }
    subs.push((sub_start, end));
    subs
}

fn is_token_match_exempt(raw: &str, lower: &str, word: &str, pos: usize, end: usize, policy: TokenPolicy<'_>) -> bool {
    if word == ext_token() {
        return is_ext_match_exempt(raw, lower, pos, end, policy);
    }
    if word == stat_it_token() {
        return is_it_stat_exempt(raw, policy, pos, end);
    }
    if word == dsl_token() {
        return is_dsl_short_exempt(policy);
    }
    false
}

fn is_it_stat_exempt(raw: &str, policy: TokenPolicy<'_>, pos: usize, end: usize) -> bool {
    if raw.get(pos..end) != Some(stat_it_token().as_str()) {
        return false;
    }
    match policy {
        TokenPolicy::RustComment { allow_it_stat_word } => allow_it_stat_word,
        TokenPolicy::UiOrAppRustLiteral { allow_it_stat_word, .. } => allow_it_stat_word,
        TokenPolicy::UiTextRun { allow_it_stat_word, .. } => allow_it_stat_word,
        _ => false,
    }
}

fn is_dsl_short_exempt(policy: TokenPolicy<'_>) -> bool {
    matches!(policy, TokenPolicy::RustIdent { allow_dsl_short: true, .. })
}

fn is_ext_match_exempt(raw: &str, lower: &str, pos: usize, end: usize, policy: TokenPolicy<'_>) -> bool {
    let tok = ext_token();
    let is_lower_dot_ext = raw.as_bytes().get(pos..end) == Some(tok.as_bytes()) && is_dot_ext_token(lower.as_bytes(), pos, end);
    let is_cmd_ext = raw.get(pos.saturating_sub(12)..end) == Some(import_cmd_id().as_str())
        || raw.get(pos.saturating_sub(6)..end) == Some(import_cmd_short().as_str());
    let is_xml_ext_tag =
        raw.get(pos..pos + 9) == Some(join(&["Pri", "smFile"]).as_str()) || raw.get(pos..pos + 15) == Some(join(&["Pri", "smXMLVersion"]).as_str());
    match policy {
        TokenPolicy::Strict => false,
        TokenPolicy::AllowExactBareExt => raw.as_bytes().get(pos..end) == Some(tok.as_bytes()),
        TokenPolicy::Standard => is_lower_dot_ext,
        TokenPolicy::RustComment { .. } => is_lower_dot_ext,
        TokenPolicy::RustIdent { allow_canonical_ext_ident, .. } => allow_canonical_ext_ident,
        TokenPolicy::RustLiteral { allow_bare_ext, allow_dot_ext, allow_import_ext_cmd, allow_legacy_xml, allow_test_container_op } => {
            (allow_bare_ext && raw == tok)
                || (allow_dot_ext && is_lower_dot_ext)
                || (allow_import_ext_cmd && is_cmd_ext)
                || (allow_legacy_xml && is_xml_ext_tag)
                || (allow_test_container_op && raw.as_bytes().get(pos..end) == Some(tok.as_bytes()))
        }
        TokenPolicy::UiOrAppRustLiteral {
            allow_bare_ext, allow_dot_ext, allow_import_ext_cmd, allow_legacy_xml, allow_test_container_op, ..
        } => {
            (allow_bare_ext && raw == tok)
                || (allow_dot_ext && is_lower_dot_ext)
                || (allow_import_ext_cmd && is_cmd_ext)
                || (allow_legacy_xml && is_xml_ext_tag)
                || (allow_test_container_op && raw.as_bytes().get(pos..end) == Some(tok.as_bytes()))
        }
        TokenPolicy::UiTextRun { allow_dot_ext, allow_palette_cmd_id, .. } => {
            (allow_dot_ext && is_lower_dot_ext) || (allow_palette_cmd_id && is_cmd_ext)
        }
        TokenPolicy::SvgTitleOrDesc { prev_word } => is_lower_dot_ext || is_preceded_by_optical_modifier(lower.get(..pos).unwrap_or(""), prev_word),
    }
}

fn is_dot_ext_description(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed == join(&[".", "pri", "sm"]) || trimmed == join(&["*.", "pri", "sm"]) {
        return true;
    }
    if let Some(stem) = trimmed.strip_suffix(&join(&[".", "pri", "sm"]))
        && !stem.is_empty()
        && stem.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    if !lower.contains(&join(&[".", "pri", "sm"])) || lower.contains("http://") || lower.contains("https://") || lower.contains("www.") {
        return false;
    }
    const FORMAT_TOKENS: &[&str] = &[
        ".curvebook",
        ".cbtemplate",
        ".csv",
        ".tsv",
        "import",
        "export",
        "extension",
        "filter",
        "accept",
        "format",
        "archive",
        "file",
        "drop",
        "chunk",
        "table",
        "subcolumn",
        "project",
        "max_project_cells",
        "open",
    ];
    FORMAT_TOKENS.iter().any(|tok| lower.contains(tok))
}

fn is_dot_ext_token(bytes: &[u8], pos: usize, end: usize) -> bool {
    if pos == 0 || bytes[pos - 1] != b'.' {
        return false;
    }
    let followed_by_domain_segment = bytes.get(end) == Some(&b'.') && bytes.get(end + 1).is_some_and(u8::is_ascii_alphanumeric);
    !followed_by_domain_segment
}

fn is_preceded_by_optical_modifier(before_lower: &str, prev_word: Option<&str>) -> bool {
    let trimmed = before_lower.trim_end();
    let word = if trimmed.is_empty() { prev_word.unwrap_or("") } else { trailing_ascii_word(trimmed) };
    OPTICAL_MODIFIERS.contains(&word)
}

fn trailing_ascii_word(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut start = bytes.len();
    while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'-') {
        start -= 1;
    }
    s.get(start..).unwrap_or("")
}

fn check_single_string_with_extra(
    path: &str,
    line: usize,
    text: &str,
    token_policy: TokenPolicy<'_>,
    include_pre_rename: bool,
    out: &mut Vec<BrandHit>,
) {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let lower = text.to_ascii_lowercase();
    for brand in &suite_tokens() {
        if has_word_match(text, &lower, brand, TokenPolicy::Strict) && seen.insert(brand.clone()) {
            out.push(BrandHit { path: path.to_owned(), line, brand: brand.clone() });
        }
    }
    if !is_legal_policy_doc_path(path)
        && !is_attribution_or_notice_path(path, true)
        && has_word_match(text, &lower, &adapted_upstream_token(), TokenPolicy::Strict)
        && seen.insert(adapted_upstream_token())
    {
        out.push(BrandHit { path: path.to_owned(), line, brand: adapted_upstream_token() });
    }
    if include_pre_rename {
        for brand in &pre_rename_tokens() {
            if has_word_match(text, &lower, brand, TokenPolicy::Strict) && seen.insert(brand.clone()) {
                out.push(BrandHit { path: path.to_owned(), line, brand: brand.clone() });
            }
        }
    }
    for brand in &forbidden_vendor_tokens() {
        if has_word_match(text, &lower, brand, token_policy) && seen.insert(brand.clone()) {
            out.push(BrandHit { path: path.to_owned(), line, brand: brand.clone() });
        }
    }
}

fn check_single_string(path: &str, line: usize, text: &str, token_policy: TokenPolicy<'_>, out: &mut Vec<BrandHit>) {
    check_single_string_with_extra(path, line, text, token_policy, false, out);
}

/// Scan arbitrary UI, `egui_shell`, AccessKit, or tooltip `&str` slices for external
/// brands and suite names. Allows `.ext` / `*.ext` only inside explicit import/export
/// file-type descriptions.
#[must_use]
pub fn check_text_runs<I, S>(source: &str, runs: I) -> Vec<BrandHit>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out = Vec::new();
    for (idx, run) in runs.into_iter().enumerate() {
        let text = run.as_ref();
        let policy = TokenPolicy::UiTextRun { allow_dot_ext: is_dot_ext_description(text), allow_it_stat_word: false, allow_palette_cmd_id: false };
        check_single_string(source, idx + 1, text, policy, &mut out);
    }
    out
}

/// Strictly scan text runs, rejecting every external brand, suite name, and `"ext"` mention
/// (including `.ext`).
#[must_use]
pub fn check_text_runs_strict<I, S>(source: &str, runs: I) -> Vec<BrandHit>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out = Vec::new();
    for (idx, run) in runs.into_iter().enumerate() {
        check_single_string(source, idx + 1, run.as_ref(), TokenPolicy::Strict, &mut out);
    }
    out
}

/// Scan `(label, text)` pairs (for example `(node.id, node.a11y.label)` or `(key, text)`),
/// tagging each hit with `label`.
#[must_use]
pub fn check_labeled_text_runs<'a, I>(runs: I) -> Vec<BrandHit>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let mut out = Vec::new();
    for (idx, (label, text)) in runs.into_iter().enumerate() {
        let policy = TokenPolicy::UiTextRun { allow_dot_ext: is_dot_ext_description(text), allow_it_stat_word: false, allow_palette_cmd_id: false };
        check_single_string(label, idx + 1, text, policy, &mut out);
    }
    out
}

fn is_i18n_format_extension_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower.contains("file_filter")
        || lower.contains("format_filter")
        || lower.contains("file_extensions")
        || lower.contains("import_extensions")
        || lower.contains("export_extensions")
}

/// Scan `(key, locale, text)` i18n catalog entries across locales.
///
/// Rejects `"ext"` (including `.ext`) everywhere outside explicit file-extension/filter keys,
/// and permits lowercase Italian `"ext"` only inside `locale == "it"`.
#[must_use]
pub fn check_i18n_catalog_entries<'a, I>(entries: I) -> Vec<BrandHit>
where
    I: IntoIterator<Item = (&'a str, &'a str, &'a str)>,
{
    let mut out = Vec::new();
    for (key, locale, text) in entries {
        let allow_it_stat_word = locale == "it";
        let policy = if is_i18n_format_extension_key(key) {
            TokenPolicy::UiTextRun { allow_dot_ext: is_dot_ext_description(text), allow_it_stat_word, allow_palette_cmd_id: false }
        } else {
            TokenPolicy::UiTextRun { allow_dot_ext: false, allow_it_stat_word, allow_palette_cmd_id: false }
        };
        let path = format!("i18n:{locale}:{key}");
        check_single_string(&path, 1, text, policy, &mut out);
    }
    out
}

/// Scan all registered 16-locale `i18n::CATALOG` translations and locale display names.
#[must_use]
pub fn check_i18n_catalog() -> Vec<BrandHit> {
    let mut entries: Vec<(&str, &str, &str)> = Vec::with_capacity(i18n::CATALOG.len() * i18n::LOCALES.len() + i18n::LOCALES.len());
    for entry in i18n::CATALOG {
        for (idx, &locale) in i18n::LOCALES.iter().enumerate() {
            if let Some(&text) = entry.translations.get(idx) {
                entries.push((entry.key, locale, text));
            }
        }
    }
    for &locale in i18n::LOCALES {
        entries.push(("locale_display_name", locale, i18n::locale_display_name(locale)));
    }
    check_i18n_catalog_entries(entries)
}

fn decode_xml_or_html_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some((before, after_amp)) = rest.split_once('&') {
        out.push_str(before);
        if let Some((entity, tail)) = after_amp.split_once(';')
            && entity.len() <= 12
            && let Some(decoded) = decode_named_or_numeric_entity(entity)
        {
            out.push(decoded);
            rest = tail;
            continue;
        }
        out.push('&');
        rest = after_amp;
    }
    out.push_str(rest);
    out
}

fn decode_named_or_numeric_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        "colon" => Some(':'),
        "sol" => Some('/'),
        "period" => Some('.'),
        "quest" => Some('?'),
        "equals" => Some('='),
        "hellip" => Some('…'),
        "mdash" => Some('—'),
        "ndash" => Some('–'),
        _ => {
            let num = entity.strip_prefix('#')?;
            let code = if let Some(hex) = num.strip_prefix(['x', 'X']) { u32::from_str_radix(hex, 16).ok()? } else { num.parse::<u32>().ok()? };
            char::from_u32(code)
        }
    }
}

fn extract_tag_name(raw_tag_inner: &str) -> (bool, bool, &str) {
    let trimmed = raw_tag_inner.trim();
    let is_close = trimmed.starts_with('/');
    let self_closing = !is_close && trimmed.ends_with('/');
    let body = trimmed.trim_start_matches('/').trim_end_matches('/').trim();
    let raw_name = body.split(|c: char| c.is_ascii_whitespace() || c == '/').next().unwrap_or("");
    let local_name = raw_name.rsplit_once(':').map_or(raw_name, |(_, local)| local);
    (is_close, self_closing, local_name)
}

fn count_newlines(s: &str) -> usize {
    s.bytes().filter(|&b| b == b'\n').count()
}

fn scan_xml_text_fragment(source: &str, start_line: usize, raw: &str, policy: TokenPolicy<'_>, out: &mut Vec<BrandHit>) {
    let decoded = decode_xml_or_html_entities(raw);
    for (offset, line_text) in decoded.lines().enumerate() {
        check_single_string_with_extra(source, start_line + offset, line_text, policy, true, out);
    }
}

/// Inspect rendered SVG export metadata (`<title>`, `<desc>`, `<metadata>`) and text elements
/// strictly against external brands and suite names.
#[must_use]
pub fn check_svg_metadata(source: &str, svg: &str) -> Vec<BrandHit> {
    let mut out = Vec::new();
    let mut line = 1usize;
    let mut pos = 0usize;
    let mut meta_depth = 0usize;
    let mut skip_depth = 0usize;
    let mut text_depth = 0usize;
    let mut active_start_line = 1usize;
    let mut buf = String::new();

    while let Some(rest) = svg.get(pos..) {
        if rest.is_empty() {
            break;
        }
        if let Some(after) = rest.strip_prefix("<!--") {
            let (comment_body, end) = match after.split_once("-->") {
                Some((body, _)) => (body, pos + 4 + body.len() + 3),
                None => (after, svg.len()),
            };
            if !comment_body.trim().is_empty() {
                scan_xml_text_fragment(source, line, comment_body, TokenPolicy::Strict, &mut out);
            }
            line += count_newlines(svg.get(pos..end).unwrap_or(rest));
            pos = end;
            continue;
        }
        if let Some(after) = rest.strip_prefix("<![CDATA[") {
            let (cdata_body, end) = match after.split_once("]]>") {
                Some((body, _)) => (body, pos + 9 + body.len() + 3),
                None => (after, svg.len()),
            };
            if skip_depth == 0 && (meta_depth > 0 || text_depth > 0) {
                buf.push_str(cdata_body);
            }
            line += count_newlines(svg.get(pos..end).unwrap_or(rest));
            pos = end;
            continue;
        }
        if rest.starts_with('<')
            && let Some(gt_rel) = rest.find('>')
        {
            let tag_end = pos + gt_rel + 1;
            let raw_inner = rest.get(1..gt_rel).unwrap_or("");
            let (is_close, self_closing, name) = extract_tag_name(raw_inner);
            let lower = name.to_ascii_lowercase();
            let is_meta_tag = matches!(lower.as_str(), "title" | "desc" | "metadata");
            let is_text_tag = matches!(lower.as_str(), "text" | "tspan" | "textpath");
            let is_skip_tag = matches!(lower.as_str(), "style" | "image");

            if !is_close && !is_skip_tag {
                scan_svg_tag_attributes(source, line, raw_inner, &mut out);
            }
            if is_close {
                if is_skip_tag {
                    skip_depth = skip_depth.saturating_sub(1);
                } else if is_meta_tag {
                    meta_depth = meta_depth.saturating_sub(1);
                    if meta_depth == 0 && text_depth == 0 && !buf.trim().is_empty() {
                        scan_xml_text_fragment(source, active_start_line, &buf, TokenPolicy::Strict, &mut out);
                        buf.clear();
                    }
                } else if is_text_tag {
                    text_depth = text_depth.saturating_sub(1);
                    if meta_depth == 0 && text_depth == 0 && !buf.trim().is_empty() {
                        scan_xml_text_fragment(source, active_start_line, &buf, TokenPolicy::Strict, &mut out);
                        buf.clear();
                    }
                }
            } else if !self_closing {
                if is_skip_tag {
                    skip_depth += 1;
                } else if is_meta_tag {
                    if meta_depth == 0 && text_depth == 0 {
                        active_start_line = line;
                        buf.clear();
                    }
                    meta_depth += 1;
                } else if is_text_tag {
                    if meta_depth == 0 && text_depth == 0 {
                        active_start_line = line;
                        buf.clear();
                    }
                    text_depth += 1;
                }
            }
            line += count_newlines(svg.get(pos..tag_end).unwrap_or(""));
            pos = tag_end;
            continue;
        }
        let next_lt = rest.find('<').map_or(svg.len(), |rel| {
            let step = if rel == 0 { rest.chars().next().map_or(1, char::len_utf8) } else { rel };
            pos + step
        });
        if skip_depth == 0 && (meta_depth > 0 || text_depth > 0) {
            if buf.is_empty() {
                active_start_line = line;
            }
            buf.push_str(svg.get(pos..next_lt).unwrap_or(""));
        }
        line += count_newlines(svg.get(pos..next_lt).unwrap_or(""));
        pos = next_lt;
    }

    if !buf.trim().is_empty() {
        scan_xml_text_fragment(source, active_start_line, &buf, TokenPolicy::Strict, &mut out);
    }
    out
}

fn scan_svg_tag_attributes(source: &str, start_line: usize, raw_inner: &str, out: &mut Vec<BrandHit>) {
    let bytes = raw_inner.as_bytes();
    let mut i = 0usize;
    let mut cur_line = start_line;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            cur_line += 1;
            i += 1;
            continue;
        }
        let q = bytes[i];
        if q == b'"' || q == b'\'' {
            let attr_line = cur_line;
            i += 1;
            let val_start = i;
            while i < bytes.len() && bytes[i] != q {
                if bytes[i] == b'\n' {
                    cur_line += 1;
                }
                i += 1;
            }
            let val = raw_inner.get(val_start..i).unwrap_or("");
            if !val.to_ascii_lowercase().trim_start().starts_with("data:image/") {
                scan_xml_text_fragment(source, attr_line, val, TokenPolicy::Strict, out);
            }
            if i < bytes.len() {
                i += 1;
            }
            continue;
        }
        i += 1;
    }
}

fn is_pdf_stream_start(bytes: &[u8], pos: usize) -> Option<usize> {
    if !bytes.get(pos..pos + 6).is_some_and(|s| s == b"stream") {
        return None;
    }
    if pos > 0 && !bytes[pos - 1].is_ascii_whitespace() && bytes[pos - 1] != b'>' {
        return None;
    }
    if bytes.get(pos + 6..pos + 8) == Some(b"\r\n") {
        Some(pos + 8)
    } else if matches!(bytes.get(pos + 6), Some(b'\n' | b'\r')) {
        Some(pos + 7)
    } else {
        None
    }
}

fn find_pdf_endstream(bytes: &[u8], mut pos: usize, line: &mut usize) -> usize {
    while pos < bytes.len() {
        if bytes.get(pos..pos + 9) == Some(b"endstream") {
            return pos + 9;
        }
        if bytes[pos] == b'\n' {
            *line += 1;
        }
        pos += 1;
    }
    bytes.len()
}

fn decode_pdf_bytes_to_string(raw: &[u8]) -> String {
    if let Some(rest) = raw.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16(rest, false);
    }
    String::from_utf8_lossy(raw).into_owned()
}

fn decode_pdf_literal_string(bytes: &[u8], pos: &mut usize, line: &mut usize) -> String {
    *pos += 1; // '('
    let mut depth = 1usize;
    let mut raw = Vec::new();
    while *pos < bytes.len() && depth > 0 {
        let b = bytes[*pos];
        *pos += 1;
        match b {
            b'(' => {
                depth += 1;
                raw.push(b'(');
            }
            b')' => {
                depth -= 1;
                if depth > 0 {
                    raw.push(b')');
                }
            }
            b'\\' => decode_pdf_escape(bytes, pos, line, &mut raw),
            b'\n' => {
                *line += 1;
                raw.push(b'\n');
            }
            other => raw.push(other),
        }
    }
    decode_pdf_bytes_to_string(&raw)
}

fn decode_pdf_escape(bytes: &[u8], pos: &mut usize, line: &mut usize, raw: &mut Vec<u8>) {
    let Some(&esc) = bytes.get(*pos) else {
        return;
    };
    *pos += 1;
    match esc {
        b'n' => raw.push(b'\n'),
        b'r' => raw.push(b'\r'),
        b't' => raw.push(b'\t'),
        b'b' => raw.push(0x08),
        b'f' => raw.push(0x0C),
        b'(' | b')' | b'\\' => raw.push(esc),
        b'\r' => {
            if bytes.get(*pos) == Some(&b'\n') {
                *pos += 1;
            }
            *line += 1;
        }
        b'\n' => {
            *line += 1;
        }
        b'0'..=b'7' => {
            let mut val = u16::from(esc - b'0');
            for _ in 0..2 {
                if let Some(&d) = bytes.get(*pos).filter(|&&c| (b'0'..=b'7').contains(&c)) {
                    val = val * 8 + u16::from(d - b'0');
                    *pos += 1;
                } else {
                    break;
                }
            }
            raw.push((val & 0xFF) as u8);
        }
        other => raw.push(other),
    }
}

fn decode_pdf_hex_string(bytes: &[u8], pos: &mut usize, line: &mut usize) -> String {
    *pos += 1; // '<'
    let mut nibbles = Vec::new();
    while *pos < bytes.len() {
        let b = bytes[*pos];
        *pos += 1;
        if b == b'>' {
            break;
        }
        if b == b'\n' {
            *line += 1;
        }
        if let Some(n) = (b as char).to_digit(16)
            && let Ok(d) = u8::try_from(n)
        {
            nibbles.push(d);
        }
    }
    if nibbles.len() % 2 == 1 {
        nibbles.push(0);
    }
    let mut raw = Vec::with_capacity(nibbles.len() / 2);
    for pair in nibbles.as_chunks::<2>().0 {
        raw.push((pair[0] << 4) | pair[1]);
    }
    decode_pdf_bytes_to_string(&raw)
}

/// Inspect PDF document metadata (`/Creator`, `/Producer`, `/Title`, `/Author`, `/Subject`,
/// `/Keywords`), decoding both literal PDF strings and UTF-16BE hex strings.
#[must_use]
pub fn check_pdf_metadata(source: &str, pdf_bytes: &[u8]) -> Vec<BrandHit> {
    const PDF_INFO_KEYS: &[&str] = &["Creator", "Producer", "Title", "Author", "Subject", "Keywords"];
    let mut out = Vec::new();
    let mut line = 1usize;
    let mut pos = 0usize;

    while pos < pdf_bytes.len() {
        if let Some(stream_start) = is_pdf_stream_start(pdf_bytes, pos) {
            line += pdf_bytes[pos..stream_start].iter().filter(|&&b| b == b'\n').count();
            pos = find_pdf_endstream(pdf_bytes, stream_start, &mut line);
            continue;
        }
        let b = pdf_bytes[pos];
        if b == b'\n' {
            line += 1;
            pos += 1;
            continue;
        }
        if b == b'/' {
            pos += 1;
            let key_start = pos;
            while pos < pdf_bytes.len() && pdf_bytes[pos].is_ascii_alphanumeric() {
                pos += 1;
            }
            let key = String::from_utf8_lossy(&pdf_bytes[key_start..pos]);
            if PDF_INFO_KEYS.contains(&key.as_ref()) {
                while pos < pdf_bytes.len() && matches!(pdf_bytes[pos], b' ' | b'\t' | b'\r' | b'\n' | 0 | 0x0C) {
                    if pdf_bytes[pos] == b'\n' {
                        line += 1;
                    }
                    pos += 1;
                }
                let val_line = line;
                if pdf_bytes.get(pos) == Some(&b'(') {
                    let decoded = decode_pdf_literal_string(pdf_bytes, &mut pos, &mut line);
                    check_single_string(source, val_line, &decoded, TokenPolicy::Strict, &mut out);
                } else if pdf_bytes.get(pos) == Some(&b'<') && pdf_bytes.get(pos + 1) != Some(&b'<') {
                    let decoded = decode_pdf_hex_string(pdf_bytes, &mut pos, &mut line);
                    check_single_string(source, val_line, &decoded, TokenPolicy::Strict, &mut out);
                }
            }
            continue;
        }
        pos += 1;
    }
    out
}

fn decode_base64_ascii(b64: &str) -> Option<Vec<u8>> {
    decode_b64(b64)
}

fn scan_svg(source: &str, text: &str, out: &mut Vec<BrandHit>) {
    out.extend(check_svg_metadata(source, text));
}

fn check_png_metadata(source: &str, png_bytes: &[u8]) -> Vec<BrandHit> {
    const PNG_SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    let mut out = Vec::new();
    if !png_bytes.starts_with(PNG_SIG) {
        return out;
    }
    let mut pos = 8usize;
    let mut chunk_idx = 0usize;
    while pos + 12 <= png_bytes.len() {
        chunk_idx += 1;
        let Some(len_slice) = png_bytes.get(pos..pos + 4) else { break };
        let Ok(len_arr) = <[u8; 4]>::try_from(len_slice) else { break };
        let chunk_len = u32::from_be_bytes(len_arr) as usize;
        let Some(ctype) = png_bytes.get(pos + 4..pos + 8) else { break };
        let data_start = pos + 8;
        let Some(data_end) = data_start.checked_add(chunk_len) else { break };
        let Some(next_pos) = data_end.checked_add(4) else { break };
        if next_pos > png_bytes.len() {
            break;
        }
        let data = png_bytes.get(data_start..data_end).unwrap_or(&[]);
        if ctype == b"tEXt" || ctype == b"iTXt" {
            let text = String::from_utf8_lossy(data);
            check_single_string_with_extra(source, chunk_idx, &text, TokenPolicy::Strict, true, &mut out);
        } else if ctype == b"zTXt"
            && let Some(nul) = data.iter().position(|&b| b == 0)
        {
            let key = String::from_utf8_lossy(data.get(..nul).unwrap_or(&[]));
            check_single_string_with_extra(source, chunk_idx, &key, TokenPolicy::Strict, true, &mut out);
        }
        if ctype == b"IEND" {
            break;
        }
        pos = next_pos;
    }
    out
}

fn extract_html_attr(raw_tag_inner: &str, attr_name: &str) -> Option<String> {
    let lower = raw_tag_inner.to_ascii_lowercase();
    let mut search = 0usize;
    while let Some(rel) = lower.get(search..).and_then(|s| s.find(attr_name)) {
        let pos = search + rel;
        let end = pos + attr_name.len();
        let left_ok = pos == 0 || lower.as_bytes()[pos - 1].is_ascii_whitespace();
        if !left_ok {
            search = end;
            continue;
        }
        let after = raw_tag_inner.get(end..)?.trim_start();
        let Some(after_eq) = after.strip_prefix('=') else {
            search = end;
            continue;
        };
        let val_src = after_eq.trim_start();
        for q in ['"', '\''] {
            if let Some(inner) = val_src.strip_prefix(q)
                && let Some((val, _)) = inner.split_once(q)
            {
                return Some(decode_xml_or_html_entities(val));
            }
        }
        search = end;
    }
    None
}

pub fn check_html_metadata(source: &str, html: &str) -> Vec<BrandHit> {
    let mut out = Vec::new();
    let mut line = 1usize;
    let mut pos = 0usize;
    let mut strict_depth = 0usize;
    let mut in_script_or_style = false;
    let mut text_start_line = 1usize;
    let mut buf = String::new();

    while let Some(rest) = html.get(pos..) {
        if rest.is_empty() {
            break;
        }
        if let Some(after) = rest.strip_prefix("<!--") {
            let (comment_inner, end) = match rest.find("-->") {
                Some(rel) => {
                    let close_pos = pos + rel;
                    (html.get(pos + 4..close_pos.max(pos + 4)).unwrap_or(""), close_pos + 3)
                }
                None => (after, html.len()),
            };
            flush_html_text(source, line, comment_inner, true, &mut out);
            line += count_newlines(html.get(pos..end).unwrap_or(rest));
            pos = end;
            continue;
        }
        if in_script_or_style {
            let lower_rest = rest.to_ascii_lowercase();
            let close_pos = lower_rest.find("</script").or_else(|| lower_rest.find("</style"));
            let chunk_end = close_pos.map_or(html.len(), |rel| pos + rel);
            if chunk_end > pos {
                let chunk = html.get(pos..chunk_end).unwrap_or("");
                flush_html_text(source, line, chunk, false, &mut out);
                line += count_newlines(chunk);
            }
            if let Some(rel) = close_pos {
                let tag_start = pos + rel;
                let gt_end = html.get(tag_start..).and_then(|s| s.find('>')).map_or(html.len(), |g| tag_start + g + 1);
                line += count_newlines(html.get(tag_start..gt_end).unwrap_or(""));
                pos = gt_end;
                in_script_or_style = false;
            } else {
                pos = html.len();
            }
            continue;
        }
        if rest.starts_with('<')
            && let Some(gt_rel) = rest.find('>')
        {
            if !buf.trim().is_empty() {
                flush_html_text(source, text_start_line, &buf, strict_depth > 0, &mut out);
            }
            buf.clear();
            let tag_end = pos + gt_rel + 1;
            let raw_inner = rest.get(1..gt_rel).unwrap_or("");
            let (is_close, self_closing, name) = extract_tag_name(raw_inner);
            let lower = name.to_ascii_lowercase();
            let tag_line = line;

            if matches!(lower.as_str(), "script" | "style") {
                if !is_close && !self_closing {
                    in_script_or_style = true;
                }
            } else if !is_close {
                if lower == "meta"
                    && let Some(content) = extract_html_attr(raw_inner, "content")
                {
                    check_single_string_with_extra(source, tag_line, &content, TokenPolicy::Strict, true, &mut out);
                }
                for attr in ["aria-label", "alt", "title"] {
                    if let Some(val) = extract_html_attr(raw_inner, attr) {
                        check_single_string_with_extra(source, tag_line, &val, TokenPolicy::Strict, true, &mut out);
                    }
                }
                if matches!(lower.as_str(), "title" | "desc" | "metadata") {
                    strict_depth += 1;
                }
            } else if matches!(lower.as_str(), "title" | "desc" | "metadata") {
                strict_depth = strict_depth.saturating_sub(1);
            }
            line += count_newlines(html.get(pos..tag_end).unwrap_or(""));
            pos = tag_end;
            continue;
        }
        let next_lt = rest.find('<').map_or(html.len(), |rel| {
            let step = if rel == 0 { rest.chars().next().map_or(1, char::len_utf8) } else { rel };
            pos + step
        });
        if buf.is_empty() {
            text_start_line = line;
        }
        let text_chunk = html.get(pos..next_lt).unwrap_or("");
        buf.push_str(text_chunk);
        line += count_newlines(text_chunk);
        pos = next_lt;
    }

    if !buf.trim().is_empty() {
        flush_html_text(source, text_start_line, &buf, strict_depth > 0, &mut out);
    }
    out
}

fn flush_html_text(source: &str, start_line: usize, raw: &str, strict: bool, out: &mut Vec<BrandHit>) {
    let decoded = decode_xml_or_html_entities(raw);
    for (offset, line_text) in decoded.lines().enumerate() {
        let policy = if strict {
            TokenPolicy::Strict
        } else {
            TokenPolicy::UiTextRun { allow_dot_ext: is_dot_ext_description(line_text), allow_it_stat_word: false, allow_palette_cmd_id: false }
        };
        check_single_string_with_extra(source, start_line + offset, line_text, policy, strict, out);
    }
}

pub fn check_manifest_metadata(source: &str, manifest_json: &str) -> Vec<BrandHit> {
    let mut out = Vec::new();
    match serde_json::from_str::<serde_json::Value>(manifest_json) {
        Ok(val) => visit_json_strings(source, &val, 1, &mut out),
        Err(_) => {
            for (idx, line) in manifest_json.lines().enumerate() {
                check_single_string(source, idx + 1, line, TokenPolicy::Strict, &mut out);
            }
        }
    }
    out
}

fn visit_json_strings(source: &str, val: &serde_json::Value, idx_hint: usize, out: &mut Vec<BrandHit>) {
    match val {
        serde_json::Value::String(s) => {
            check_single_string(source, idx_hint, s, TokenPolicy::Strict, out);
        }
        serde_json::Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                visit_json_strings(source, item, i + 1, out);
            }
        }
        serde_json::Value::Object(map) => {
            for (i, (k, v)) in map.iter().enumerate() {
                check_single_string(source, i + 1, k, TokenPolicy::Strict, out);
                visit_json_strings(source, v, i + 1, out);
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

/// Inspect a desktop or browser window title string strictly.
#[must_use]
pub fn check_window_title(source: &str, title: &str) -> Vec<BrandHit> {
    let mut out = Vec::new();
    check_single_string(source, 1, title, TokenPolicy::Strict, &mut out);
    out
}

fn collect_snapshot_strings(snap: &UiTreeSnapshot, context: &str, hits: &mut Vec<BrandHit>) {
    let mut runs: Vec<(String, String)> = Vec::new();
    if let Some(banner) = &snap.status_banner {
        runs.push((format!("{context}:status_banner"), banner.clone()));
    }
    if let Some(target) = &snap.context_menu_target {
        runs.push((format!("{context}:context_menu_target"), target.clone()));
    }
    if let Some(diag) = &snap.result_diagnostic_banner {
        runs.push((format!("{context}:diag.summary"), diag.summary.clone()));
        runs.push((format!("{context}:diag.limitation"), diag.limitation_note.clone()));
        if let Some(metric) = &diag.diagnostic_metric {
            runs.push((format!("{context}:diag.metric"), metric.clone()));
        }
    }
    if let Some(menus) = &snap.menus {
        for m in menus {
            runs.push((format!("{context}:menu:{}", m.menu_key), m.title.clone()));
            for n in &m.items {
                push_widget_node_runs(context, n, &mut runs);
            }
        }
    }
    for (group, nodes) in [
        ("toolbar", snap.toolbar.as_deref()),
        ("tabs", snap.tabs.as_deref()),
        ("canvas", snap.canvas_elements.as_deref()),
        ("grid", snap.grid_cells.as_deref()),
    ] {
        if let Some(slice) = nodes {
            for n in slice {
                push_widget_node_runs(&format!("{context}:{group}"), n, &mut runs);
            }
        }
    }
    if let Some(nav) = &snap.navigator_sheets {
        for entry in nav {
            let nav_ctx = format!("{context}:nav:{}", entry.sheet_id);
            hits.extend(check_text_runs_strict(&nav_ctx, [&entry.title, &entry.label]));
        }
    }
    if let Some(dlg) = &snap.dialog {
        runs.push((format!("{context}:dialog:{}:title", dlg.dialog_kind), dlg.title.clone()));
        if let Some(stage) = &dlg.stage {
            runs.push((format!("{context}:dialog:{}:stage", dlg.dialog_kind), stage.clone()));
        }
        for n in dlg.tabs.iter().chain(&dlg.controls).chain(&dlg.buttons) {
            push_widget_node_runs(&format!("{context}:dialog:{}", dlg.dialog_kind), n, &mut runs);
        }
    }
    for (label, text) in &runs {
        let allow_it_stat_word = label.contains(":it:");
        let policy = TokenPolicy::UiTextRun { allow_dot_ext: is_dot_ext_description(text), allow_it_stat_word, allow_palette_cmd_id: false };
        check_single_string(label, 1, text, policy, hits);
    }
}

fn check_labeled_ui_pairs<'a>(runs: impl IntoIterator<Item = (&'a str, &'a str)>, allow_it_stat_word: bool, hits: &mut Vec<BrandHit>) {
    for (idx, (label, text)) in runs.into_iter().enumerate() {
        let policy = TokenPolicy::UiTextRun { allow_dot_ext: is_dot_ext_description(text), allow_it_stat_word, allow_palette_cmd_id: false };
        check_single_string(label, idx + 1, text, policy, hits);
    }
}

fn check_ui_strings_with_locale<'a>(source: &str, runs: impl IntoIterator<Item = &'a str>, allow_it_stat_word: bool, hits: &mut Vec<BrandHit>) {
    for (idx, text) in runs.into_iter().enumerate() {
        let policy = TokenPolicy::UiTextRun { allow_dot_ext: is_dot_ext_description(text), allow_it_stat_word, allow_palette_cmd_id: false };
        check_single_string(source, idx + 1, text, policy, hits);
    }
}

fn push_widget_node_runs(prefix: &str, node: &curvebook_ui_egui::inspect::UiWidgetNode, out: &mut Vec<(String, String)>) {
    out.push((format!("{prefix}:{}", node.widget_id), node.label.clone()));
    if let Some(reason) = &node.disabled_reason {
        out.push((format!("{prefix}:{}:disabled_reason", node.widget_id), reason.clone()));
    }
    if let Some(serde_json::Value::String(v)) = &node.value {
        out.push((format!("{prefix}:{}:value", node.widget_id), v.clone()));
    }
}

fn build_brands_sweep_document() -> Result<ProjectDocument, String> {
    let mut doc = ProjectDocument::new("CurveBook Brand Sweep").map_err(|e| format!("project init: {e}"))?;
    for (idx, fixture) in SAMPLE_FIXTURES.iter().enumerate() {
        let id = format!("data-{}", idx + 1);
        let ds = instantiate_sample_fixture(fixture.id, id).map_err(|e| format!("fixture {}: {e}", fixture.id))?;
        doc.add_data_sheet(ds).map_err(|e| format!("add fixture {}: {e}", fixture.id))?;
    }
    let mut info = InfoSheet::new_preseeded("info-1", "Protocol Constants").map_err(|e| format!("info sheet: {e}"))?;
    info.constants
        .push(curvebook_doc::InfoConstant::new("ControlEC50", ConstantValue::Number(12.5), Some(String::from("nM"))).map_err(|e| e.to_string())?);
    doc.add_info_sheet(info).map_err(|e| e.to_string())?;

    let spec =
        AnalysisSpec::new("res-1", "data-2", AnalysisKind::OneWayAnova(curvebook_doc::OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    let res = ResultSheet::new(spec).map_err(|e| e.to_string())?;
    doc.add_result_sheet(res).map_err(|e| e.to_string())?;

    if let Some(ds_xy) = doc.data_sheet(&SheetId::new("data-1")).cloned() {
        let mut g1 = GraphSheet::new_uninitialized_for_data("graph-1", &ds_xy).map_err(|e| e.to_string())?;
        g1.initialized = true;
        g1.title = String::from("Dose-Response Inhibition");
        doc.add_graph_sheet(g1).map_err(|e| e.to_string())?;

        let mut g2 = GraphSheet::new_uninitialized_for_data("graph-2", &ds_xy).map_err(|e| e.to_string())?;
        g2.initialized = true;
        g2.title = String::from("Secondary Curve Panel");
        doc.add_graph_sheet(g2).map_err(|e| e.to_string())?;
    }

    let mut layout_spec = LayoutSpec::default();
    layout_spec.items.push(LayoutItem::LiveGraph {
        id: String::from("panel-a"),
        graph_sheet_id: SheetId::new("graph-1"),
        bounds_pt: curvebook_doc::RectPt::new(18.0, 18.0, 220.0, 160.0),
        scale_percent: 100.0,
        badge_override: Some(String::from("A")),
    });
    let layout = LayoutSheet::new("layout-1", "Publication Panel Layout", layout_spec).map_err(|e| e.to_string())?;
    doc.add_layout_sheet(layout).map_err(|e| e.to_string())?;
    Ok(doc)
}

fn check_tooltips_and_catalogs(app: &mut UiApp, hits: &mut Vec<BrandHit>) {
    for &btn in TOOLBAR_BUTTONS {
        hits.extend(check_labeled_text_runs([("TOOLBAR_BUTTONS.id", btn.id), ("TOOLBAR_BUTTONS.label_key", btn.label_key)]));
    }
    for &locale in i18n::LOCALES {
        app.locale = locale.to_owned();
        let is_it = locale == "it";
        for section in menu_bar_tooltips(app) {
            let ctx = format!("menu:{locale}:{}", section.menu_key);
            check_ui_strings_with_locale(&ctx, [section.title], is_it, hits);
            for tip in section.items {
                let p_label = format!("{ctx}:{}:label", tip.command_id);
                let p_loc = format!("{ctx}:{}:localized", tip.command_id);
                let p_sum = format!("{ctx}:{}:summary", tip.command_id);
                let p_beg = format!("{ctx}:{}:beginner", tip.command_id);
                let p_path = format!("{ctx}:{}:menu_path", tip.command_id);
                check_labeled_ui_pairs(
                    [
                        (p_label.as_str(), tip.label),
                        (p_loc.as_str(), tip.localized_label),
                        (p_sum.as_str(), tip.summary),
                        (p_beg.as_str(), tip.beginner),
                        (p_path.as_str(), tip.menu_path),
                    ],
                    is_it,
                    hits,
                );
            }
        }
        for tb in toolbar_tooltips(app) {
            let ctx = format!("toolbar:{locale}:{}", tb.button.id);
            check_ui_strings_with_locale(&ctx, [tb.caption, tb.tooltip.localized_label, tb.tooltip.summary, tb.tooltip.beginner], is_it, hits);
        }
    }
    app.locale = String::from("en");

    for cat in AnalysisCategory::ALL {
        hits.extend(check_text_runs("AnalysisCategory", [cat.label()]));
    }
    for item in ANALYSIS_CATALOG {
        hits.extend(check_labeled_text_runs([
            ("ANALYSIS_CATALOG.label", item.label),
            ("ANALYSIS_CATALOG.summary", item.summary),
            ("ANALYSIS_CATALOG.keywords", item.keywords),
        ]));
    }
    for kind in TableKind::ALL {
        let draft = TableCreationDraft::for_kind(kind);
        hits.extend(check_text_runs("TableKind", [kind.label(), draft.title.as_str()]));
    }
    for fixture in SAMPLE_FIXTURES {
        hits.extend(check_labeled_text_runs([("SAMPLE_FIXTURES.title", fixture.title), ("SAMPLE_FIXTURES.summary", fixture.summary)]));
    }
    let palette = CommandPaletteState::default();
    for entry in palette.matches_with_shortcuts(Some(app.session()), &app.shortcuts) {
        hits.extend(check_labeled_text_runs([
            ("CommandPalette.label", entry.label),
            ("CommandPalette.menu_path", entry.menu_path),
            ("CommandPalette.summary", entry.summary),
            ("CommandPalette.beginner", entry.beginner),
        ]));
    }
}

fn check_all_dialogs_and_inspectors(app: &mut UiApp, hits: &mut Vec<BrandHit>) {
    let data_id = SheetId::new("data-1");
    let res_id = SheetId::new("res-1");
    let graph_id = SheetId::new("graph-1");
    let layout_id = SheetId::new("layout-1");

    for mode in [WelcomeMode::EmptyTypedTable, WelcomeMode::TutorialSample, WelcomeMode::FromTemplate, WelcomeMode::ClonePipeline] {
        app.nav.welcome.open = true;
        app.nav.welcome.mode = mode;
        let snap = inspect_ui_tree(app, None, true);
        collect_snapshot_strings(&snap, &format!("welcome:{mode:?}"), hits);
    }
    app.nav.welcome.open = false;

    for inspector in [
        ActiveInspector::None,
        ActiveInspector::FormatGraph,
        ActiveInspector::FormatAxes,
        ActiveInspector::PairwiseComparisons,
        ActiveInspector::BubblePlot,
        ActiveInspector::PageLayout,
        ActiveInspector::ResultDetails,
    ] {
        app.canvas_dialogs.active_inspector = inspector;
        hits.extend(check_text_runs("ActiveInspector", [inspector.label()]));
        let snap = inspect_ui_tree(app, None, true);
        collect_snapshot_strings(&snap, &format!("inspector:{inspector:?}"), hits);
    }

    let dialogs = [
        ActiveDialog::FormatAxes { sheet_id: graph_id.clone() },
        ActiveDialog::AnalyzeSheet { source_sheet_id: data_id.clone() },
        ActiveDialog::DuplicateAnalysisPrompt {
            source_sheet_id: data_id.clone(),
            existing_result_ids: vec![res_id.clone()],
            pending_entry_id: String::from("stats.one_way_anova"),
        },
        ActiveDialog::EditAnalysisParameters { result_sheet_id: res_id.clone() },
        ActiveDialog::ChangeDataAnalyzed { result_sheet_id: res_id.clone() },
        ActiveDialog::ResultViewer { result_sheet_id: res_id.clone() },
        ActiveDialog::ConstantHookPicker { target_sheet_id: res_id.clone() },
        ActiveDialog::ChangeGraphType { graph_sheet_id: graph_id.clone() },
        ActiveDialog::FormatGraph { graph_sheet_id: graph_id.clone() },
        ActiveDialog::PairwiseComparisons { graph_sheet_id: graph_id.clone() },
        ActiveDialog::BubblePlotInspector { graph_sheet_id: graph_id.clone() },
        ActiveDialog::ApplyStyleFrom { target_graph_id: graph_id.clone() },
        ActiveDialog::PageLayoutComposer { layout_sheet_id: layout_id.clone() },
        ActiveDialog::ExportFigure { sheet_id: graph_id.clone() },
        ActiveDialog::ConvertArchetype { sheet_id: data_id.clone() },
        ActiveDialog::ImportReshape,
        ActiveDialog::ExcludeWithReason { sheet_id: data_id.clone() },
        ActiveDialog::FormatSelectedPoints { graph_sheet_id: graph_id.clone() },
        ActiveDialog::PowerAndSampleSize,
        ActiveDialog::Print { sheet_id: Some(layout_id.clone()) },
        ActiveDialog::RandomizeGroups,
        ActiveDialog::RandomSampleRows { source_sheet_id: Some(data_id.clone()) },
        ActiveDialog::SimulateDistributions,
        ActiveDialog::InsertModelEquation { graph_sheet_id: graph_id.clone() },
        ActiveDialog::CreateGraph { source_sheet_id: Some(data_id.clone()) },
        ActiveDialog::EqualizeScaling { layout_sheet_id: layout_id.clone() },
        ActiveDialog::FormatEmbeddedTable { sheet_id: graph_id.clone(), target_index: 0, is_layout: false },
        ActiveDialog::FormatImage { sheet_id: graph_id.clone(), target_index: 0, is_layout: false },
        ActiveDialog::CommandPalette,
        ActiveDialog::Checkpoints,
        ActiveDialog::About,
    ];
    for dlg in dialogs {
        app.canvas_dialogs.active_dialog = dlg.clone();
        let snap = inspect_ui_tree(app, None, true);
        collect_snapshot_strings(&snap, "dialog", hits);
    }
    app.canvas_dialogs.active_dialog = ActiveDialog::None;
}

fn check_export_metadata_sweep(doc: &ProjectDocument, hits: &mut Vec<BrandHit>) -> Result<(), String> {
    let graph = doc.graph_sheet(&SheetId::new("graph-1")).ok_or_else(|| String::from("sweep graph-1 missing"))?;
    let mut scene = compile_graph(graph, doc).map_err(|e| format!("compile sweep graph: {e}"))?;
    scene.title = String::from("Dose-Response Inhibition");

    let svg = render_svg(&scene, &SvgExportOptions::default()).map_err(|e| format!("sweep render_svg: {e}"))?;
    hits.extend(check_svg_metadata("export:render_svg", &svg));

    let pdf_ascii = render_pdf(&scene, &PdfExportOptions::default()).map_err(|e| format!("sweep render_pdf: {e}"))?;
    hits.extend(check_pdf_metadata("export:render_pdf", &pdf_ascii));

    let mut unicode_scene = scene.clone();
    unicode_scene.title = String::from("Étude Dose–Réponse — αβγ");
    let pdf_unicode = render_pdf(&unicode_scene, &PdfExportOptions::default()).map_err(|e| format!("sweep unicode render_pdf: {e}"))?;
    hits.extend(check_pdf_metadata("export:render_pdf:unicode", &pdf_unicode));

    let bundle = ClipboardBundle::from_scene(&scene, 144)?;
    hits.extend(check_svg_metadata("clipboard:svg", &bundle.svg));
    hits.extend(check_pdf_metadata("clipboard:pdf", &bundle.pdf_bytes));
    hits.extend(check_html_metadata("clipboard:html", &bundle.html));

    for (label, bytes_res) in
        [("archive:project:manifest.json", save_project_bytes(doc)), ("archive:template:manifest.json", save_template_bytes(doc))]
    {
        let bytes = bytes_res.map_err(|e| format!("{label}: {e}"))?;
        let zip = ZipArchive::from_bytes_with_limits(&bytes, ZipLimits::default()).map_err(|e| format!("{label} zip: {e}"))?;
        let manifest = zip.get_utf8("manifest.json").map_err(|e| format!("{label}: {e}"))?.ok_or_else(|| format!("{label} missing manifest.json"))?;
        hits.extend(check_manifest_metadata(label, manifest));
    }

    hits.extend(check_window_title("desktop:window_title:untitled", "Untitled — CurveBook"));
    hits.extend(check_window_title("desktop:window_title:dirty", "study.curvebook * — CurveBook"));
    Ok(())
}

/// Sweep in-process UI strings across 16-locale `i18n`, tooltips, all 8 table archetypes,
/// all 26 `ActiveDialog` states, all 7 `ActiveInspector` tabs, live 16-locale `egui_shell` renders,
/// live MCP headless & `--gui-only` JSON-RPC surfaces, `CommandPalette`, engine/UI command catalogs,
/// and SVG/PDF/HTML/manifest/window export metadata.
pub fn check_ui_surfaces() -> Result<Vec<BrandHit>, String> {
    let mut hits = check_i18n_catalog();

    let mut empty_app = UiApp::new();
    let empty_snap = inspect_ui_tree(&empty_app, None, true);
    collect_snapshot_strings(&empty_snap, "empty_ui", &mut hits);
    check_tooltips_and_catalogs(&mut empty_app, &mut hits);
    check_engine_and_ui_commands(&mut hits);

    let doc = build_brands_sweep_document()?;
    check_export_metadata_sweep(&doc, &mut hits)?;

    let mut session = Session::from_document(doc);
    session.recalculate_all().map_err(|e| format!("recalculate sweep session: {e}"))?;
    let mut app = UiApp::from_session(session);
    check_all_dialogs_and_inspectors(&mut app, &mut hits);
    hits.extend(check_live_egui_shell_surfaces()?);
    hits.extend(check_mcp_surfaces()?);

    Ok(hits)
}

struct BrandsGuiSessionHost {
    app: UiApp,
}

impl BrandsGuiSessionHost {
    fn new(mut app: UiApp, gui_only: bool) -> Self {
        app.set_gui_only(gui_only);
        Self { app }
    }
}

impl GuiSessionHost for BrandsGuiSessionHost {
    fn call_gui(&mut self, method: &str, mut params: serde_json::Value) -> Result<serde_json::Value, curvebook_engine::EngineError> {
        if method == "ui.screenshot"
            && let Some(obj) = params.as_object_mut()
            && matches!(obj.get("target").and_then(serde_json::Value::as_str), Some("workspace" | "viewport"))
        {
            obj.insert(String::from("target"), serde_json::Value::String(String::from("full_window")));
        }
        curvebook_ui_egui::dispatch_gui_action(&mut self.app, method, params)
    }

    fn set_gui_only(&mut self, gui_only: bool) {
        self.app.set_gui_only(gui_only);
    }

    fn is_gui_only(&self) -> bool {
        self.app.is_gui_only()
    }

    fn hosted_session(&self) -> Option<&Session> {
        Some(self.app.session())
    }

    fn hosted_session_mut(&mut self) -> Option<&mut Session> {
        Some(self.app.session_mut())
    }

    fn sync_after_session_mutation(&mut self) {
        self.app.sync_views();
    }
}

#[derive(Clone, Copy)]
struct McpScanCtx<'a> {
    source: &'a str,
    method: &'a str,
    called_tool: Option<&'a str>,
    entry_tool_name: Option<&'a str>,
}

/// Inspect a serialized MCP JSON-RPC response (`initialize`, `tools/list`, `tools/call`,
/// `resources/list`, `resources/read`, `prompts/list`, `prompts/get`) for prohibited vendor,
/// product, retired-suite, or pre-rename tokens while preserving the narrow `.ext`/`.ext`
/// file-format boundary on `open_file` and `table_import_ext` / `table.importext`.
#[must_use]
pub fn check_mcp_rpc_response(source: &str, method: &str, called_tool: Option<&str>, response: &serde_json::Value) -> Vec<BrandHit> {
    let mut out = Vec::new();
    let ctx = McpScanCtx { source, method, called_tool, entry_tool_name: None };
    scan_mcp_json_value(ctx, "", response, &mut out);
    out
}

fn scan_mcp_json_value(ctx: McpScanCtx<'_>, ptr: &str, val: &serde_json::Value, out: &mut Vec<BrandHit>) {
    match val {
        serde_json::Value::Object(map) => scan_mcp_json_object(ctx, ptr, map, out),
        serde_json::Value::Array(items) => {
            for (idx, item) in items.iter().enumerate() {
                scan_mcp_json_value(ctx, &format!("{ptr}/{idx}"), item, out);
            }
        }
        serde_json::Value::String(s) => scan_mcp_leaf_string(ctx, ptr, s, out),
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

fn scan_mcp_json_object(ctx: McpScanCtx<'_>, ptr: &str, map: &serde_json::Map<String, serde_json::Value>, out: &mut Vec<BrandHit>) {
    if ctx.method == "tools/call"
        && map.get("type").and_then(serde_json::Value::as_str) == Some("image")
        && map.get("mimeType").and_then(serde_json::Value::as_str) == Some("image/png")
    {
        scan_mcp_inline_png_block(ctx.source, ptr, map, out);
        return;
    }
    let next_tool = if ctx.method == "tools/list" && ptr.starts_with("/result/tools/") && ctx.entry_tool_name.is_none() {
        map.get("name").and_then(serde_json::Value::as_str)
    } else {
        ctx.entry_tool_name
    };
    let child_ctx = McpScanCtx { entry_tool_name: next_tool, ..ctx };
    let key_source = format!("{}:{ptr}", ctx.source);
    for (idx, (k, v)) in map.iter().enumerate() {
        check_single_string_with_extra(&key_source, idx + 1, k, TokenPolicy::Strict, true, out);
        scan_mcp_json_value(child_ctx, &format!("{ptr}/{k}"), v, out);
    }
}

fn scan_mcp_inline_png_block(source: &str, ptr: &str, map: &serde_json::Map<String, serde_json::Value>, out: &mut Vec<BrandHit>) {
    let block_src = format!("{source}:{ptr}");
    for (idx, k) in map.keys().enumerate() {
        check_single_string_with_extra(&block_src, idx + 1, k, TokenPolicy::Strict, true, out);
    }
    if let Some(b64) = map.get("data").and_then(serde_json::Value::as_str)
        && let Some(decoded) = decode_base64_ascii(b64)
    {
        out.extend(check_png_metadata(&format!("{block_src}/data#png"), &decoded));
    }
}

fn is_top_tool_field_ptr(ptr: &str, field: &str) -> bool {
    let Some(rest) = ptr.strip_prefix("/result/tools/") else {
        return false;
    };
    let Some((idx_seg, tail)) = rest.split_once('/') else {
        return false;
    };
    !idx_seg.is_empty() && idx_seg.bytes().all(|b| b.is_ascii_digit()) && tail == field
}

fn scan_mcp_leaf_string(ctx: McpScanCtx<'_>, ptr: &str, text: &str, out: &mut Vec<BrandHit>) {
    if (ctx.method == "tools/call" || ctx.method == "resources/read") && ptr.ends_with("/text") {
        let trimmed = text.trim_start();
        if (trimmed.starts_with('{') || trimmed.starts_with('['))
            && let Ok(inner) = serde_json::from_str::<serde_json::Value>(text)
        {
            scan_mcp_json_value(ctx, &format!("{ptr}#json"), &inner, out);
            return;
        }
        if ctx.called_tool == Some("export") && text.contains("<svg") {
            let svg_src = format!("{}:{ptr}#svg", ctx.source);
            scan_svg(&svg_src, text, out);
            out.extend(check_svg_metadata(&svg_src, text));
            return;
        }
    }

    let full_source = format!("{}:{ptr}", ctx.source);
    if is_exact_canonical_mcp_import(ctx, ptr, text) {
        let lower = text.to_ascii_lowercase();
        for brand in suite_tokens().into_iter().chain(pre_rename_tokens()) {
            if has_word_match(text, &lower, &brand, TokenPolicy::Strict) {
                out.push(BrandHit { path: full_source.clone(), line: 1, brand });
            }
        }
        return;
    }

    let policy = mcp_leaf_token_policy(ctx, ptr, text);
    check_single_string_with_extra(&full_source, 1, text, policy, true, out);
}

fn mcp_import_tool() -> String {
    join(&["table_import_", "pri", "sm"])
}

fn is_exact_canonical_mcp_import(ctx: McpScanCtx<'_>, ptr: &str, text: &str) -> bool {
    let imp = mcp_import_tool();
    if text != imp {
        return false;
    }
    if ctx.method == "tools/list" && ctx.entry_tool_name == Some(imp.as_str()) && is_top_tool_field_ptr(ptr, "name") {
        return true;
    }
    ctx.method == "tools/call"
        && matches!(ctx.called_tool, Some("list_toolsets" | "command_list"))
        && (ptr.ends_with("/name") || ptr.ends_with("/mcp_tool"))
}

fn mcp_leaf_token_policy<'a>(ctx: McpScanCtx<'_>, ptr: &str, text: &str) -> TokenPolicy<'a> {
    if ctx.method == "tools/list" {
        if ctx.entry_tool_name == Some("open_file") {
            if is_top_tool_field_ptr(ptr, "description") {
                return TokenPolicy::UiTextRun {
                    allow_dot_ext: is_dot_ext_description(text),
                    allow_it_stat_word: false,
                    allow_palette_cmd_id: false,
                };
            }
            if ptr.contains("/inputSchema/properties/format/enum/") && text == ext_token() {
                return TokenPolicy::AllowExactBareExt;
            }
            return TokenPolicy::Strict;
        }
        if ctx.entry_tool_name == Some(mcp_import_tool().as_str()) && is_top_tool_field_ptr(ptr, "description") {
            return TokenPolicy::UiTextRun { allow_dot_ext: is_dot_ext_description(text), allow_it_stat_word: false, allow_palette_cmd_id: true };
        }
        return TokenPolicy::Strict;
    }

    if ctx.method == "tools/call" && matches!(ctx.called_tool, Some("list_toolsets" | "command_list" | "ui_inspect")) {
        if (ptr.ends_with("/command_id") || ptr.ends_with("/id") || ptr.ends_with("/widget_id"))
            && (text == import_cmd_id() || text.ends_with(".table.importPrism"))
        {
            return TokenPolicy::UiTextRun { allow_dot_ext: false, allow_it_stat_word: false, allow_palette_cmd_id: true };
        }
        if ptr.ends_with("/summary") || ptr.ends_with("/beginner") || ptr.ends_with("/params_doc") || ptr.ends_with("/label") {
            return TokenPolicy::UiTextRun { allow_dot_ext: is_dot_ext_description(text), allow_it_stat_word: false, allow_palette_cmd_id: false };
        }
    }

    TokenPolicy::Strict
}

fn send_mcp_rpc(state: &mut McpServerState, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    let req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    dispatch_json_value(req, state).ok_or_else(|| format!("MCP `{method}` produced no JSON-RPC response"))
}

fn ensure_mcp_ok<'a>(label: &str, resp: &'a serde_json::Value) -> Result<&'a serde_json::Value, String> {
    if let Some(err) = resp.get("error") {
        return Err(format!("MCP `{label}` returned unexpected JSON-RPC error: {err}"));
    }
    let result = resp.get("result").ok_or_else(|| format!("MCP `{label}` missing `result` field"))?;
    if result.get("isError").and_then(serde_json::Value::as_bool) == Some(true) {
        return Err(format!("MCP `{label}` returned tool execution error: {result}"));
    }
    Ok(result)
}

/// Drive actual in-memory `curvebook-mcp` JSON-RPC 2.0 sessions (both default headless mode and
/// strict `--gui-only` mode) over a populated 5-sheet `ProjectDocument`, scanning `initialize`,
/// `tools/list` (core + all progressive `CommandGroup`s + `gui`), `resources/list` + `resources/read`,
/// `prompts/list` + `prompts/get`, and `tools/call` (`doc_inspect`, `render_preview`, `export`,
/// `ui_inspect`, `ui_screenshot`).
pub fn check_mcp_surfaces() -> Result<Vec<BrandHit>, String> {
    let mut hits = Vec::new();
    let doc = build_brands_sweep_document()?;
    let mut session = Session::from_document(doc);
    session.recalculate_all().map_err(|e| format!("MCP sweep recalculate: {e}"))?;

    check_mcp_headless_mode(session.clone(), &mut hits)?;
    check_mcp_gui_only_mode(session, &mut hits)?;
    Ok(hits)
}

fn check_mcp_headless_mode(session: Session, hits: &mut Vec<BrandHit>) -> Result<(), String> {
    let mut state = McpServerState::from_session(session);

    let init = send_mcp_rpc(
        &mut state,
        "initialize",
        serde_json::json!({ "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "xtask-brands", "version": "0.1.0" } }),
    )?;
    ensure_mcp_ok("headless:initialize", &init)?;
    hits.extend(check_mcp_rpc_response("mcp:headless:initialize", "initialize", None, &init));

    let core_tools = send_mcp_rpc(&mut state, "tools/list", serde_json::json!({}))?;
    ensure_mcp_ok("headless:tools/list:core", &core_tools)?;
    hits.extend(check_mcp_rpc_response("mcp:headless:tools/list:core", "tools/list", None, &core_tools));

    let toolsets = send_mcp_rpc(&mut state, "tools/call", serde_json::json!({ "name": "list_toolsets", "arguments": {} }))?;
    ensure_mcp_ok("headless:list_toolsets", &toolsets)?;
    hits.extend(check_mcp_rpc_response("mcp:headless:list_toolsets", "tools/call", Some("list_toolsets"), &toolsets));

    let enable = send_mcp_rpc(
        &mut state,
        "tools/call",
        serde_json::json!({ "name": "enable_toolset", "arguments": { "group": "all", "groups": ["gui"], "enabled": true } }),
    )?;
    ensure_mcp_ok("headless:enable_toolset", &enable)?;
    hits.extend(check_mcp_rpc_response("mcp:headless:enable_toolset", "tools/call", Some("enable_toolset"), &enable));
    for notif in state.take_pending_notifications() {
        hits.extend(check_mcp_rpc_response("mcp:headless:notif", "notification", None, &notif));
    }

    let all_tools = send_mcp_rpc(&mut state, "tools/list", serde_json::json!({}))?;
    let all_res = ensure_mcp_ok("headless:tools/list:all", &all_tools)?;
    let tool_count = all_res.get("tools").and_then(serde_json::Value::as_array).map_or(0, Vec::len);
    if tool_count < 110 {
        return Err(format!("expected >= 110 enabled MCP tools in sweep, got {tool_count}"));
    }
    hits.extend(check_mcp_rpc_response("mcp:headless:tools/list:all", "tools/list", None, &all_tools));

    let cmd_list = send_mcp_rpc(&mut state, "tools/call", serde_json::json!({ "name": "command_list", "arguments": { "include_ui": true } }))?;
    ensure_mcp_ok("headless:command_list", &cmd_list)?;
    hits.extend(check_mcp_rpc_response("mcp:headless:command_list", "tools/call", Some("command_list"), &cmd_list));

    check_mcp_resources_and_prompts(&mut state, "mcp:headless", false, hits)?;
    check_mcp_headless_previews_and_inspect(&mut state, hits)?;
    Ok(())
}

fn check_mcp_headless_previews_and_inspect(state: &mut McpServerState, hits: &mut Vec<BrandHit>) -> Result<(), String> {
    let overview = send_mcp_rpc(state, "tools/call", serde_json::json!({ "name": "doc_inspect", "arguments": {} }))?;
    ensure_mcp_ok("headless:doc_inspect", &overview)?;
    hits.extend(check_mcp_rpc_response("mcp:headless:doc_inspect:root", "tools/call", Some("doc_inspect"), &overview));

    for sid in ["data-1", "info-1", "res-1", "graph-1", "layout-1"] {
        let sheet_ins =
            send_mcp_rpc(state, "tools/call", serde_json::json!({ "name": "doc_inspect", "arguments": { "sheet_id": sid, "include_cells": true } }))?;
        ensure_mcp_ok("headless:doc_inspect:sheet", &sheet_ins)?;
        hits.extend(check_mcp_rpc_response(&format!("mcp:headless:doc_inspect:{sid}"), "tools/call", Some("doc_inspect"), &sheet_ins));
    }

    for sid in ["graph-1", "layout-1"] {
        let preview = send_mcp_rpc(
            state,
            "tools/call",
            serde_json::json!({ "name": "render_preview", "arguments": { "sheet_id": sid, "width_px": 320, "height_px": 220, "dpi": 144 } }),
        )?;
        ensure_mcp_ok("headless:render_preview", &preview)?;
        hits.extend(check_mcp_rpc_response(&format!("mcp:headless:render_preview:{sid}"), "tools/call", Some("render_preview"), &preview));
    }

    let svg_export =
        send_mcp_rpc(state, "tools/call", serde_json::json!({ "name": "export", "arguments": { "format": "svg", "sheet_id": "graph-1" } }))?;
    ensure_mcp_ok("headless:export:svg", &svg_export)?;
    hits.extend(check_mcp_rpc_response("mcp:headless:export:svg", "tools/call", Some("export"), &svg_export));
    Ok(())
}

fn check_mcp_gui_only_mode(session: Session, hits: &mut Vec<BrandHit>) -> Result<(), String> {
    let mut app = UiApp::from_session(session);
    app.sync_views();
    let mut state = McpServerState::with_gui_host(Box::new(BrandsGuiSessionHost::new(app, true)));

    let init = send_mcp_rpc(
        &mut state,
        "initialize",
        serde_json::json!({ "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "xtask-brands-gui", "version": "0.1.0" } }),
    )?;
    ensure_mcp_ok("gui_only:initialize", &init)?;
    hits.extend(check_mcp_rpc_response("mcp:gui_only:initialize", "initialize", None, &init));

    let gui_tools = send_mcp_rpc(&mut state, "tools/list", serde_json::json!({}))?;
    let gui_res = ensure_mcp_ok("gui_only:tools/list", &gui_tools)?;
    let gui_count = gui_res.get("tools").and_then(serde_json::Value::as_array).map_or(0, Vec::len);
    if gui_count != 8 {
        return Err(format!("expected 8 --gui-only MCP tools, got {gui_count}"));
    }
    hits.extend(check_mcp_rpc_response("mcp:gui_only:tools/list", "tools/list", None, &gui_tools));

    check_mcp_resources_and_prompts(&mut state, "mcp:gui_only", true, hits)?;

    let ui_ins = send_mcp_rpc(&mut state, "tools/call", serde_json::json!({ "name": "ui_inspect", "arguments": { "scope": "all" } }))?;
    ensure_mcp_ok("gui_only:ui_inspect", &ui_ins)?;
    hits.extend(check_mcp_rpc_response("mcp:gui_only:ui_inspect", "tools/call", Some("ui_inspect"), &ui_ins));

    let ui_shot = send_mcp_rpc(
        &mut state,
        "tools/call",
        serde_json::json!({ "name": "ui_screenshot", "arguments": { "target": "workspace", "width_px": 320, "height_px": 200, "dpi": 144 } }),
    )?;
    ensure_mcp_ok("gui_only:ui_screenshot", &ui_shot)?;
    hits.extend(check_mcp_rpc_response("mcp:gui_only:ui_screenshot", "tools/call", Some("ui_screenshot"), &ui_shot));

    let blocked = send_mcp_rpc(&mut state, "tools/call", serde_json::json!({ "name": "doc_inspect", "arguments": {} }))?;
    hits.extend(check_mcp_rpc_response("mcp:gui_only:blocked_doc_inspect", "tools/call", Some("doc_inspect"), &blocked));
    Ok(())
}

fn check_mcp_resources_and_prompts(state: &mut McpServerState, prefix: &str, gui_only: bool, hits: &mut Vec<BrandHit>) -> Result<(), String> {
    let res_list = send_mcp_rpc(state, "resources/list", serde_json::json!({}))?;
    let res_ok = ensure_mcp_ok(&format!("{prefix}:resources/list"), &res_list)?;
    hits.extend(check_mcp_rpc_response(&format!("{prefix}:resources/list"), "resources/list", None, &res_list));

    let uris: Vec<String> = res_ok
        .get("resources")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("uri").and_then(serde_json::Value::as_str).map(str::to_owned))
        .collect();
    if !gui_only && (!uris.iter().any(|u| u.starts_with("sheet://")) || !uris.iter().any(|u| u.starts_with("result://"))) {
        return Err(String::from("expected both sheet:// and result:// resources in headless MCP sweep"));
    }
    if gui_only {
        let blocked_read = send_mcp_rpc(state, "resources/read", serde_json::json!({ "uri": "sheet://data-1" }))?;
        hits.extend(check_mcp_rpc_response(&format!("{prefix}:resources/read:blocked"), "resources/read", None, &blocked_read));
    } else {
        for uri in &uris {
            let read_resp = send_mcp_rpc(state, "resources/read", serde_json::json!({ "uri": uri }))?;
            ensure_mcp_ok(&format!("{prefix}:resources/read:{uri}"), &read_resp)?;
            hits.extend(check_mcp_rpc_response(&format!("{prefix}:resources/read:{uri}"), "resources/read", None, &read_resp));
        }
    }

    let prompts_list = send_mcp_rpc(state, "prompts/list", serde_json::json!({}))?;
    let prompts_ok = ensure_mcp_ok(&format!("{prefix}:prompts/list"), &prompts_list)?;
    hits.extend(check_mcp_rpc_response(&format!("{prefix}:prompts/list"), "prompts/list", None, &prompts_list));

    let names: Vec<String> = prompts_ok
        .get("prompts")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("name").and_then(serde_json::Value::as_str).map(str::to_owned))
        .collect();
    if names.is_empty() {
        return Err(format!("{prefix}: expected non-empty prompts/list"));
    }
    for name in &names {
        for (variant, args) in prompt_arg_variants(name) {
            let prompt_resp = send_mcp_rpc(state, "prompts/get", serde_json::json!({ "name": name, "arguments": args }))?;
            ensure_mcp_ok(&format!("{prefix}:prompts/get:{name}:{variant}"), &prompt_resp)?;
            hits.extend(check_mcp_rpc_response(&format!("{prefix}:prompts/get:{name}:{variant}"), "prompts/get", None, &prompt_resp));
        }
    }
    Ok(())
}

fn prompt_arg_variants(name: &str) -> Vec<(&'static str, serde_json::Value)> {
    match name {
        "dose_response_curve_fit" => {
            vec![("default", serde_json::json!({})), ("antagonist", serde_json::json!({ "sheet_id": "data-1", "response_type": "antagonist" }))]
        }
        "choose_two_group_test" => vec![
            ("default", serde_json::json!({})),
            ("paired_param", serde_json::json!({ "paired": "true", "parametric": "true" })),
            ("paired_nonparam", serde_json::json!({ "paired": "true", "parametric": "false" })),
            ("unpaired_param", serde_json::json!({ "paired": "false", "parametric": "true" })),
            ("unpaired_nonparam", serde_json::json!({ "paired": "false", "parametric": "false" })),
        ],
        "publication_figure_checklist" => {
            vec![("default", serde_json::json!({})), ("svg", serde_json::json!({ "graph_sheet_id": "graph-1", "target_format": "svg" }))]
        }
        _ => vec![("default", serde_json::json!({}))],
    }
}

fn check_engine_and_ui_commands(hits: &mut Vec<BrandHit>) {
    for spec in engine_cmd_registry().specs() {
        let is_import_ext_cmd = spec.id == import_cmd_id();
        let id_policy = TokenPolicy::UiTextRun { allow_dot_ext: false, allow_it_stat_word: false, allow_palette_cmd_id: is_import_ext_cmd };
        check_single_string(&format!("engine_cmd:{}:id", spec.id), 1, spec.id, id_policy, hits);
        let text_policy = TokenPolicy::UiTextRun { allow_dot_ext: true, allow_it_stat_word: false, allow_palette_cmd_id: false };
        for (field, val) in [("label", spec.label), ("menu_path", spec.menu_path), ("summary", spec.summary), ("beginner", spec.beginner)] {
            let p = TokenPolicy::UiTextRun { allow_dot_ext: is_dot_ext_description(val), allow_it_stat_word: false, allow_palette_cmd_id: false };
            check_single_string(&format!("engine_cmd:{}:{field}", spec.id), 1, val, p, hits);
        }
        if let Ok(schema_str) = serde_json::to_string(&(spec.input_schema)()) {
            check_single_string(&format!("engine_cmd:{}:input_schema", spec.id), 1, &schema_str, text_policy, hits);
        }
    }
    for cmd in UI_COMMANDS {
        let is_import_ext_cmd = cmd.id == import_cmd_id();
        let id_policy = TokenPolicy::UiTextRun { allow_dot_ext: false, allow_it_stat_word: false, allow_palette_cmd_id: is_import_ext_cmd };
        check_single_string(&format!("ui_cmd:{}:id", cmd.id), 1, cmd.id, id_policy, hits);
        for (field, val) in [("label", cmd.label), ("params_doc", cmd.params_doc), ("summary", cmd.summary), ("beginner", cmd.beginner)] {
            let p = TokenPolicy::UiTextRun { allow_dot_ext: is_dot_ext_description(val), allow_it_stat_word: false, allow_palette_cmd_id: false };
            check_single_string(&format!("ui_cmd:{}:{field}", cmd.id), 1, val, p, hits);
        }
    }
}

/// Sweep live `egui_shell` painted text runs, `FrameInspectNodeMeta`, and AccessKit strings
/// across ALL `16` `LOCALES`, all `5` sheet kinds, all `4` `WelcomeMode`s, all `7` `ActiveInspector`
/// tabs, all `26` `ActiveDialog` variants, open dropdown menus, toolbar overflow, split view, and
/// portfolio gallery.
pub fn check_live_egui_shell_surfaces() -> Result<Vec<BrandHit>, String> {
    let mut hits = Vec::new();
    let desktop_vp = ViewportMetrics::new(1440.0, 900.0, 1.0);
    let narrow_vp = ViewportMetrics::new(640.0, 560.0, 1.0);

    let mut empty_app = UiApp::new();
    let doc = build_brands_sweep_document()?;
    let mut session = Session::from_document(doc);
    session.recalculate_all().map_err(|e| format!("egui_shell sweep recalculate: {e}"))?;
    let mut app = UiApp::from_session(session);
    app.sync_views();

    let sheet_ids = ["data-1", "info-1", "res-1", "graph-1", "layout-1"];
    let menu_keys: Vec<&'static str> = menu_bar_tooltips(&app).into_iter().map(|s| s.menu_key).collect();
    let dialogs = sweep_dialog_states();

    for &locale in i18n::LOCALES {
        let is_it = locale == "it";
        let scanner = EguiSurfaceScanner::new();
        empty_app.set_locale(locale);
        for mode in [WelcomeMode::EmptyTypedTable, WelcomeMode::TutorialSample, WelcomeMode::FromTemplate, WelcomeMode::ClonePipeline] {
            empty_app.nav.welcome.open = true;
            empty_app.nav.welcome.mode = mode;
            let cap = scanner.scan(&mut empty_app, desktop_vp);
            collect_egui_surface_hits(&format!("egui_shell:{locale}:welcome:{mode:?}"), is_it, false, &cap, &mut hits);
        }
        empty_app.nav.welcome.open = false;

        app.set_locale(locale);
        app.nav.welcome.open = false;
        app.canvas_dialogs.active_dialog = ActiveDialog::None;
        app.canvas_dialogs.active_inspector = ActiveInspector::None;
        app.interaction.open_menu = None;
        app.egui_views.top_chrome.overflow_menu_open = false;
        app.split.enabled = false;
        app.nav.gallery.open = false;

        for sid in sheet_ids {
            app.execute_command("sheet.select", serde_json::json!({ "sheet_id": sid })).map_err(|e| format!("select sheet `{sid}`: {e}"))?;
            app.sync_views();
            let cap = scanner.scan(&mut app, desktop_vp);
            collect_egui_surface_hits(&format!("egui_shell:{locale}:sheet:{sid}"), is_it, false, &cap, &mut hits);
        }
        app.execute_command("sheet.select", serde_json::json!({ "sheet_id": "graph-1" })).map_err(|e| format!("select graph-1: {e}"))?;
        app.sync_views();

        for inspector in [
            ActiveInspector::None,
            ActiveInspector::FormatGraph,
            ActiveInspector::FormatAxes,
            ActiveInspector::PairwiseComparisons,
            ActiveInspector::BubblePlot,
            ActiveInspector::PageLayout,
            ActiveInspector::ResultDetails,
        ] {
            app.canvas_dialogs.active_inspector = inspector;
            let cap = scanner.scan(&mut app, desktop_vp);
            collect_egui_surface_hits(&format!("egui_shell:{locale}:inspector:{inspector:?}"), is_it, false, &cap, &mut hits);
        }
        app.canvas_dialogs.active_inspector = ActiveInspector::None;

        for dlg in &dialogs {
            app.canvas_dialogs.active_dialog = dlg.clone();
            if matches!(dlg, ActiveDialog::CommandPalette) {
                app.canvas_dialogs.command_palette.open();
                let cap_default = scanner.scan(&mut app, desktop_vp);
                collect_egui_surface_hits(&format!("egui_shell:{locale}:dialog:CommandPalette:default"), is_it, false, &cap_default, &mut hits);
                app.canvas_dialogs.command_palette.query = String::from("import");
                let cap_import = scanner.scan(&mut app, desktop_vp);
                collect_egui_surface_hits(&format!("egui_shell:{locale}:dialog:CommandPalette:import"), is_it, true, &cap_import, &mut hits);
                app.canvas_dialogs.command_palette.open();
            } else {
                let cap = scanner.scan(&mut app, desktop_vp);
                collect_egui_surface_hits(&format!("egui_shell:{locale}:dialog"), is_it, false, &cap, &mut hits);
            }
        }
        app.canvas_dialogs.active_dialog = ActiveDialog::None;

        for key in &menu_keys {
            app.interaction.open_menu = Some((*key).to_owned());
            let cap = scanner.scan(&mut app, desktop_vp);
            collect_egui_surface_hits(&format!("egui_shell:{locale}:menu:{key}"), is_it, false, &cap, &mut hits);
        }
        app.interaction.open_menu = None;

        app.egui_views.top_chrome.overflow_menu_open = true;
        let overflow_cap = scanner.scan(&mut app, narrow_vp);
        collect_egui_surface_hits(&format!("egui_shell:{locale}:toolbar_overflow"), is_it, false, &overflow_cap, &mut hits);
        app.egui_views.top_chrome.overflow_menu_open = false;

        app.split.enabled = true;
        app.split.primary_sheet_id = Some(SheetId::new("data-1"));
        app.split.secondary_sheet_id = Some(SheetId::new("graph-1"));
        let split_cap = scanner.scan(&mut app, desktop_vp);
        collect_egui_surface_hits(&format!("egui_shell:{locale}:split_workspace"), is_it, false, &split_cap, &mut hits);
        app.split.enabled = false;

        app.nav.gallery.open = true;
        let gallery_doc = app.document().clone();
        app.nav.sync_gallery(&gallery_doc);
        let gallery_cap = scanner.scan(&mut app, desktop_vp);
        collect_egui_surface_hits(&format!("egui_shell:{locale}:gallery"), is_it, false, &gallery_cap, &mut hits);
        app.nav.gallery.open = false;
    }

    Ok(hits)
}

fn sweep_dialog_states() -> Vec<ActiveDialog> {
    let data_id = SheetId::new("data-1");
    let res_id = SheetId::new("res-1");
    let graph_id = SheetId::new("graph-1");
    let layout_id = SheetId::new("layout-1");
    vec![
        ActiveDialog::FormatAxes { sheet_id: graph_id.clone() },
        ActiveDialog::AnalyzeSheet { source_sheet_id: data_id.clone() },
        ActiveDialog::DuplicateAnalysisPrompt {
            source_sheet_id: data_id.clone(),
            existing_result_ids: vec![res_id.clone()],
            pending_entry_id: String::from("stats.one_way_anova"),
        },
        ActiveDialog::EditAnalysisParameters { result_sheet_id: res_id.clone() },
        ActiveDialog::ChangeDataAnalyzed { result_sheet_id: res_id.clone() },
        ActiveDialog::ResultViewer { result_sheet_id: res_id.clone() },
        ActiveDialog::ConstantHookPicker { target_sheet_id: res_id.clone() },
        ActiveDialog::ChangeGraphType { graph_sheet_id: graph_id.clone() },
        ActiveDialog::FormatGraph { graph_sheet_id: graph_id.clone() },
        ActiveDialog::PairwiseComparisons { graph_sheet_id: graph_id.clone() },
        ActiveDialog::BubblePlotInspector { graph_sheet_id: graph_id.clone() },
        ActiveDialog::ApplyStyleFrom { target_graph_id: graph_id.clone() },
        ActiveDialog::PageLayoutComposer { layout_sheet_id: layout_id.clone() },
        ActiveDialog::ExportFigure { sheet_id: graph_id.clone() },
        ActiveDialog::ConvertArchetype { sheet_id: data_id.clone() },
        ActiveDialog::ImportReshape,
        ActiveDialog::ExcludeWithReason { sheet_id: data_id.clone() },
        ActiveDialog::FormatSelectedPoints { graph_sheet_id: graph_id.clone() },
        ActiveDialog::PowerAndSampleSize,
        ActiveDialog::Print { sheet_id: Some(layout_id.clone()) },
        ActiveDialog::RandomizeGroups,
        ActiveDialog::RandomSampleRows { source_sheet_id: Some(data_id.clone()) },
        ActiveDialog::SimulateDistributions,
        ActiveDialog::InsertModelEquation { graph_sheet_id: graph_id },
        ActiveDialog::CreateGraph { source_sheet_id: Some(data_id) },
        ActiveDialog::EqualizeScaling { layout_sheet_id: layout_id },
        ActiveDialog::FormatEmbeddedTable { sheet_id: SheetId::new("graph-1"), target_index: 0, is_layout: false },
        ActiveDialog::FormatImage { sheet_id: SheetId::new("graph-1"), target_index: 0, is_layout: false },
        ActiveDialog::CommandPalette,
        ActiveDialog::Checkpoints,
        ActiveDialog::About,
    ]
}

fn collect_egui_surface_hits(
    context: &str,
    allow_it_stat_word: bool,
    allow_palette_cmd_id: bool,
    cap: &EguiSurfaceCapture,
    hits: &mut Vec<BrandHit>,
) {
    for (idx, (owner, text)) in cap.text_runs.iter().enumerate() {
        let is_palette_cmd = allow_palette_cmd_id && (owner == "palette.status" || owner.starts_with("palette.item."));
        let run_ctx = format!("{context}:text_run:{owner}");
        let policy = TokenPolicy::UiTextRun { allow_dot_ext: is_dot_ext_description(text), allow_it_stat_word, allow_palette_cmd_id: is_palette_cmd };
        check_single_string(&run_ctx, idx + 1, text, policy, hits);
    }
    for (idx, entry) in cap.a11y_entries.iter().enumerate() {
        let a11y_ctx = format!("{context}:a11y:{}", entry.widget_id);
        for txt in [Some(entry.label.as_str()), entry.value_text.as_deref(), entry.disabled_reason.as_deref(), entry.full_label.as_deref()]
            .into_iter()
            .flatten()
        {
            let policy = TokenPolicy::UiTextRun { allow_dot_ext: is_dot_ext_description(txt), allow_it_stat_word, allow_palette_cmd_id: false };
            check_single_string(&a11y_ctx, idx + 1, txt, policy, hits);
        }
        if let Some(cmd) = &entry.command_id
            && cmd != &import_cmd_id()
        {
            hits.extend(check_text_runs_strict(&format!("{a11y_ctx}:command_id"), [cmd.as_str()]));
        }
    }
    for (idx, ak) in cap.accesskit_labels.iter().enumerate() {
        let policy = TokenPolicy::UiTextRun { allow_dot_ext: is_dot_ext_description(ak), allow_it_stat_word, allow_palette_cmd_id: false };
        check_single_string(&format!("{context}:accesskit"), idx + 1, ak, policy, hits);
    }
}

#[test]
fn live_16_locale_i18n_catalog_clean() {
    assert_eq!(check_i18n_catalog(), Vec::new());
}

#[test]
fn live_ui_mcp_egui_and_export_brand_surface_sweep() -> Result<(), String> {
    let hits = check_ui_surfaces()?;
    assert!(hits.is_empty(), "live surface brand hits: {hits:?}");
    Ok(())
}

fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| String::from("CARGO_MANIFEST_DIR has no parent"))
}

fn join(parts: &[&str]) -> String {
    parts.concat()
}

fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320u32 & mask);
        }
    }
    !crc
}

fn banned_probe_tokens() -> Vec<String> {
    vec![
        join(&["gr", "aph", "pad"]),
        join(&["dot", "mat", "ics"]),
        join(&["pr", "ism"]),
        join(&["in", "st", "at"]),
        join(&["stat", "ma", "te"]),
        join(&["ori", "gin", "lab"]),
        join(&["ori", "gin", "pro"]),
        join(&["sig", "ma", "plot"]),
        join(&["min", "it", "ab"]),
        join(&["sys", "ta", "t"]),
        join(&["stat", "is", "ti", "ca"]),
        join(&["xl", "st", "at"]),
        join(&["sp", "ss"]),
        join(&["j", "m", "p"]),
        join(&["art", "cr", "aft"]),
        join(&["sto", "ry", "told"]),
    ]
}

fn contains_ascii_word(haystack: &str, needle: &str) -> bool {
    let lower = haystack.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let nlen = needle.len();
    let mut start = 0usize;
    while let Some(rel) = lower.get(start..).and_then(|s| s.find(needle)) {
        let pos = start + rel;
        let end = pos + nlen;
        let left_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
        let right_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if left_ok && right_ok {
            return true;
        }
        start = pos + 1;
    }
    false
}

fn assert_clean_ui_string(ctx: &str, text: &str, locale: &str, allow_palette_cmd_id: bool) -> Result<(), String> {
    let normalized_text = if allow_palette_cmd_id {
        let cmd_id = join(&["table.", "im", "port", "Pr", "ism"]);
        text.replace(&cmd_id, "table.import_ext")
    } else {
        text.to_owned()
    };
    let check_str = if locale == "it" {
        let ital_word = join(&["stat", "is", "ti", "ca"]);
        let bytes = normalized_text.as_bytes();
        let wlen = ital_word.len();
        let mut masked = normalized_text.clone();
        let mut search = 0usize;
        while let Some(rel) = masked.get(search..).and_then(|s| s.find(&ital_word)) {
            let pos = search + rel;
            let end = pos + wlen;
            let left_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
            let right_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
            if left_ok && right_ok {
                masked.replace_range(pos..end, "statist___");
            }
            search = pos + 1;
        }
        masked
    } else {
        normalized_text
    };
    for probe in banned_probe_tokens() {
        if contains_ascii_word(&check_str, &probe) {
            return Err(format!("{ctx}: UI surface leaked `{probe}` in `{text}`"));
        }
    }
    Ok(())
}

fn assert_clean_egui_capture(ctx: &str, cap: &EguiSurfaceCapture, locale: &str, allow_palette_cmd_id: bool) -> Result<(), String> {
    for (owner, text) in &cap.text_runs {
        let is_palette_cmd = allow_palette_cmd_id && (owner == "palette.status" || owner.starts_with("palette.item."));
        let allow = is_palette_cmd || text.contains("*.prism");
        assert_clean_ui_string(&format!("{ctx}:{owner}"), text, locale, allow)?;
    }
    for entry in &cap.a11y_entries {
        for txt in [Some(entry.label.as_str()), entry.value_text.as_deref(), entry.disabled_reason.as_deref(), entry.full_label.as_deref()]
            .into_iter()
            .flatten()
        {
            assert_clean_ui_string(&format!("{ctx}:a11y:{}", entry.widget_id), txt, locale, txt.contains("*.prism"))?;
        }
    }
    for ak in &cap.accesskit_labels {
        assert_clean_ui_string(&format!("{ctx}:accesskit"), ak, locale, ak.contains("*.prism"))?;
    }
    Ok(())
}

#[test]
fn r021_live_egui_shell_and_a11y_clean_across_all_16_locales_and_5_sheets() -> Result<(), String> {
    let mut doc = ProjectDocument::new("R021 Live Egui Shell Sweep").map_err(|e| e.to_string())?;
    for (idx, fixture) in SAMPLE_FIXTURES.iter().enumerate() {
        let ds = instantiate_sample_fixture(fixture.id, format!("data-{}", idx + 1)).map_err(|e| e.to_string())?;
        doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    }
    let info = InfoSheet::new_preseeded("info-1", "Protocol Metadata").map_err(|e| e.to_string())?;
    doc.add_info_sheet(info).map_err(|e| e.to_string())?;
    let spec =
        AnalysisSpec::new("res-1", "data-2", AnalysisKind::OneWayAnova(curvebook_doc::OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    doc.add_result_sheet(ResultSheet::new(spec).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if let Some(ds_xy) = doc.data_sheet(&SheetId::new("data-1")).cloned() {
        let mut g1 = GraphSheet::new_uninitialized_for_data("graph-1", &ds_xy).map_err(|e| e.to_string())?;
        g1.initialized = true;
        doc.add_graph_sheet(g1).map_err(|e| e.to_string())?;
    }
    let mut lspec = LayoutSpec::default();
    lspec.items.push(LayoutItem::LiveGraph {
        id: String::from("p1"),
        graph_sheet_id: SheetId::new("graph-1"),
        bounds_pt: curvebook_doc::RectPt::new(16.0, 16.0, 200.0, 140.0),
        scale_percent: 100.0,
        badge_override: Some(String::from("A")),
    });
    doc.add_layout_sheet(LayoutSheet::new("layout-1", "Figure 1", lspec).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let mut session = Session::from_document(doc);
    session.recalculate_all().map_err(|e| e.to_string())?;
    let mut app = UiApp::from_session(session);
    app.sync_views();

    let mut empty_app = UiApp::new();
    let vp = ViewportMetrics::new(1440.0, 900.0, 1.0);
    let narrow_vp = ViewportMetrics::new(640.0, 560.0, 1.0);

    let dialogs = [
        ActiveDialog::FormatAxes { sheet_id: SheetId::new("graph-1") },
        ActiveDialog::AnalyzeSheet { source_sheet_id: SheetId::new("data-1") },
        ActiveDialog::DuplicateAnalysisPrompt {
            source_sheet_id: SheetId::new("data-1"),
            existing_result_ids: vec![SheetId::new("res-1")],
            pending_entry_id: String::from("stats.one_way_anova"),
        },
        ActiveDialog::EditAnalysisParameters { result_sheet_id: SheetId::new("res-1") },
        ActiveDialog::ChangeDataAnalyzed { result_sheet_id: SheetId::new("res-1") },
        ActiveDialog::ResultViewer { result_sheet_id: SheetId::new("res-1") },
        ActiveDialog::ConstantHookPicker { target_sheet_id: SheetId::new("res-1") },
        ActiveDialog::ChangeGraphType { graph_sheet_id: SheetId::new("graph-1") },
        ActiveDialog::FormatGraph { graph_sheet_id: SheetId::new("graph-1") },
        ActiveDialog::PairwiseComparisons { graph_sheet_id: SheetId::new("graph-1") },
        ActiveDialog::BubblePlotInspector { graph_sheet_id: SheetId::new("graph-1") },
        ActiveDialog::ApplyStyleFrom { target_graph_id: SheetId::new("graph-1") },
        ActiveDialog::PageLayoutComposer { layout_sheet_id: SheetId::new("layout-1") },
        ActiveDialog::ExportFigure { sheet_id: SheetId::new("graph-1") },
        ActiveDialog::ConvertArchetype { sheet_id: SheetId::new("data-1") },
        ActiveDialog::ImportReshape,
        ActiveDialog::ExcludeWithReason { sheet_id: SheetId::new("data-1") },
        ActiveDialog::FormatSelectedPoints { graph_sheet_id: SheetId::new("graph-1") },
        ActiveDialog::PowerAndSampleSize,
        ActiveDialog::Print { sheet_id: Some(SheetId::new("layout-1")) },
        ActiveDialog::RandomizeGroups,
        ActiveDialog::RandomSampleRows { source_sheet_id: Some(SheetId::new("data-1")) },
        ActiveDialog::SimulateDistributions,
        ActiveDialog::InsertModelEquation { graph_sheet_id: SheetId::new("graph-1") },
        ActiveDialog::CreateGraph { source_sheet_id: Some(SheetId::new("data-1")) },
        ActiveDialog::EqualizeScaling { layout_sheet_id: SheetId::new("layout-1") },
        ActiveDialog::FormatEmbeddedTable { sheet_id: SheetId::new("graph-1"), target_index: 0, is_layout: false },
        ActiveDialog::FormatImage { sheet_id: SheetId::new("graph-1"), target_index: 0, is_layout: false },
        ActiveDialog::CommandPalette,
        ActiveDialog::About,
    ];
    let menu_keys: Vec<&'static str> = menu_bar_tooltips(&app).into_iter().map(|s| s.menu_key).collect();

    for &locale in i18n::LOCALES {
        let scanner = EguiSurfaceScanner::new();
        empty_app.set_locale(locale);
        for mode in [WelcomeMode::EmptyTypedTable, WelcomeMode::TutorialSample, WelcomeMode::FromTemplate, WelcomeMode::ClonePipeline] {
            empty_app.nav.welcome.open = true;
            empty_app.nav.welcome.mode = mode;
            let cap = scanner.scan(&mut empty_app, vp);
            assert_clean_egui_capture(&format!("welcome:{locale}:{mode:?}"), &cap, locale, false)?;
        }

        app.set_locale(locale);
        for sid in ["data-1", "info-1", "res-1", "graph-1", "layout-1"] {
            app.execute_command("sheet.select", serde_json::json!({ "sheet_id": sid })).map_err(|e| e.to_string())?;
            app.sync_views();
            let cap = scanner.scan(&mut app, vp);
            assert_clean_egui_capture(&format!("sheet:{locale}:{sid}"), &cap, locale, false)?;
        }
        for insp in [
            ActiveInspector::None,
            ActiveInspector::FormatGraph,
            ActiveInspector::FormatAxes,
            ActiveInspector::PairwiseComparisons,
            ActiveInspector::BubblePlot,
            ActiveInspector::PageLayout,
            ActiveInspector::ResultDetails,
        ] {
            app.canvas_dialogs.active_inspector = insp;
            let cap = scanner.scan(&mut app, vp);
            assert_clean_egui_capture(&format!("insp:{locale}:{insp:?}"), &cap, locale, false)?;
        }
        app.canvas_dialogs.active_inspector = ActiveInspector::None;

        for dlg in &dialogs {
            app.canvas_dialogs.active_dialog = dlg.clone();
            if matches!(dlg, ActiveDialog::CommandPalette) {
                app.canvas_dialogs.command_palette.open();
                let cap_default = scanner.scan(&mut app, vp);
                assert_clean_egui_capture(&format!("dlg:{locale}:palette_default"), &cap_default, locale, false)?;
                app.canvas_dialogs.command_palette.query = String::from("import");
                let cap_filtered = scanner.scan(&mut app, vp);
                assert_clean_egui_capture(&format!("dlg:{locale}:palette_import"), &cap_filtered, locale, true)?;
                app.canvas_dialogs.command_palette.open();
            } else {
                let cap = scanner.scan(&mut app, vp);
                assert_clean_egui_capture(&format!("dlg:{locale}"), &cap, locale, false)?;
            }
        }
        app.canvas_dialogs.active_dialog = ActiveDialog::None;

        for key in &menu_keys {
            app.interaction.open_menu = Some((*key).to_owned());
            let cap = scanner.scan(&mut app, vp);
            assert_clean_egui_capture(&format!("menu:{locale}:{key}"), &cap, locale, false)?;
        }
        app.interaction.open_menu = None;

        app.egui_views.top_chrome.overflow_menu_open = true;
        let overflow_cap = scanner.scan(&mut app, narrow_vp);
        assert_clean_egui_capture(&format!("overflow:{locale}"), &overflow_cap, locale, false)?;
        app.egui_views.top_chrome.overflow_menu_open = false;
    }

    // Vector export & archive manifest metadata generated from the same document.
    let graph = app.document().graph_sheet(&SheetId::new("graph-1")).ok_or_else(|| String::from("graph-1 missing"))?;
    let scene = compile_graph(graph, app.document()).map_err(|e| e.to_string())?;
    let svg = render_svg(&scene, &SvgExportOptions::default()).map_err(|e| e.to_string())?;
    let pdf = render_pdf(&scene, &PdfExportOptions::default()).map_err(|e| e.to_string())?;
    let bundle = ClipboardBundle::from_scene(&scene, 144)?;
    for probe in banned_probe_tokens() {
        if contains_ascii_word(&svg, &probe) || contains_ascii_word(&bundle.html, &probe) {
            return Err(format!("export vector/html leaked `{probe}`"));
        }
        let pdf_lossy = String::from_utf8_lossy(&pdf);
        if contains_ascii_word(&pdf_lossy, &probe) {
            return Err(format!("export pdf leaked `{probe}`"));
        }
    }
    for (kind, bytes) in [
        ("project", save_project_bytes(app.document()).map_err(|e| e.to_string())?),
        ("template", save_template_bytes(app.document()).map_err(|e| e.to_string())?),
    ] {
        let zip = ZipArchive::from_bytes_with_limits(&bytes, ZipLimits::default()).map_err(|e| e.to_string())?;
        let manifest = zip.get_utf8("manifest.json").map_err(|e| e.to_string())?.ok_or_else(|| format!("{kind} missing manifest"))?;
        for probe in banned_probe_tokens() {
            if contains_ascii_word(manifest, &probe) {
                return Err(format!("{kind} manifest leaked `{probe}`"));
            }
        }
    }
    for spec in engine_cmd_registry().specs() {
        assert_clean_ui_string(&format!("engine_cmd:{}", spec.id), spec.label, "en", false)?;
        assert_clean_ui_string(&format!("engine_cmd_summary:{}", spec.id), spec.summary, "en", false)?;
    }
    for cmd in UI_COMMANDS {
        assert_clean_ui_string(&format!("ui_cmd:{}", cmd.id), cmd.label, "en", false)?;
        assert_clean_ui_string(&format!("ui_cmd_summary:{}", cmd.id), cmd.summary, "en", false)?;
    }

    Ok(())
}

#[test]
fn r021_mcp_packaging_and_command_artefacts_strictly_clean() -> Result<(), String> {
    let root = workspace_root()?;
    let dot_ext = join(&[".pr", "ism"]);
    let mime_proj = join(&["application/x-pr", "ism-project"]);
    let mime_xml = join(&["application/x-pr", "ism-xml"]);
    let cmd_id = join(&["table.", "im", "port", "Pr", "ism"]);

    // 1. Verify packaging files carry zero brand waivers and zero product titles.
    for pkg_rel in [
        "packaging/macos/Info.plist.in",
        "packaging/linux/org.curvebook.CurveBook.desktop",
        "packaging/linux/org.curvebook.CurveBook.mime.xml",
        "packaging/windows/curvebook.wxs",
        "packaging/web/_headers",
        "packaging/web/.htaccess",
        "packaging/web/README.md",
    ] {
        let raw = fs::read_to_string(root.join(pkg_rel)).map_err(|e| format!("read {pkg_rel}: {e}"))?;
        let waiver = join(&["br", "and-ok"]);
        if raw.contains(&waiver) {
            return Err(format!("{pkg_rel} still contains `{waiver}` waiver"));
        }
        let cleaned = raw
            .replace(&dot_ext, ".ext")
            .replace(&mime_proj, "application/x-ext")
            .replace(&mime_xml, "application/x-ext-xml")
            .replace(&format!("<string>{}</string>", join(&["pr", "ism"])), "<string>ext</string>");
        for probe in banned_probe_tokens() {
            if contains_ascii_word(&cleaned, &probe) {
                return Err(format!("{pkg_rel} leaked prohibited token `{probe}`"));
            }
        }
    }

    // 2. Verify MCP source modules, docs/commands.md, docs/index.html, docs/site.css, and JSON schemas.
    let mcp_dir = root.join("crates/mcp/src");
    for entry in fs::read_dir(&mcp_dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let txt = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let cleaned =
                txt.replace(&cmd_id, "table.import_ext").replace(&dot_ext, ".ext").replace(&format!("\"{}\"", join(&["pr", "ism"])), "\"ext\"");
            for probe in banned_probe_tokens() {
                if contains_ascii_word(&cleaned, &probe) {
                    return Err(format!("{} leaked `{probe}`", path.display()));
                }
            }
        }
    }
    for rel in ["docs/commands.md", "docs/index.html", "docs/site.css"] {
        let txt = fs::read_to_string(root.join(rel)).map_err(|e| e.to_string())?;
        let cleaned = txt.replace(&cmd_id, "table.import_ext").replace(&dot_ext, ".ext");
        for probe in banned_probe_tokens() {
            if contains_ascii_word(&cleaned, &probe) {
                return Err(format!("{rel} leaked `{probe}`"));
            }
        }
    }

    let schemas_dir = root.join("docs/schemas");
    if schemas_dir.is_dir() {
        for entry in fs::read_dir(&schemas_dir).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                let txt = fs::read_to_string(&path).map_err(|e| e.to_string())?;
                let cleaned = txt.replace(&cmd_id, "table.import_ext").replace(&dot_ext, ".ext");
                for probe in banned_probe_tokens() {
                    if contains_ascii_word(&cleaned, &probe) {
                        return Err(format!("{} leaked `{probe}`", path.display()));
                    }
                }
            }
        }
    }

    Ok(())
}

#[test]
fn r021_asset_provenance_png_ico_crc_and_edited_docs_links_valid() -> Result<(), String> {
    let root = workspace_root()?;
    let assets_md = fs::read_to_string(root.join("ASSETS.md")).map_err(|e| e.to_string())?;
    let row_count = assets_md.lines().filter(|l| l.trim_start().starts_with("| `")).count();
    if row_count < 51 {
        return Err(format!("expected >= 51 ASSETS.md rows, found {row_count}"));
    }
    let pre_rename = join(&["lum", "encraft"]);
    if contains_ascii_word(&assets_md, &pre_rename) {
        return Err(String::from("ASSETS.md contains pre-rename product token"));
    }
    for probe in banned_probe_tokens() {
        if contains_ascii_word(&assets_md, &probe) {
            return Err(format!("ASSETS.md contains prohibited token `{probe}`"));
        }
    }

    let expected_png_sig = b"\x89PNG\r\n\x1a\n";
    let mut verified_pngs = 0usize;
    let icon_root = root.join("assets/app-icon");
    let mut stack = vec![icon_root];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) == Some("png") {
                let bytes = fs::read(&path).map_err(|e| e.to_string())?;
                verify_png_chunks(&path, &bytes, expected_png_sig, &pre_rename)?;
                verified_pngs += 1;
            } else if path.extension().and_then(|e| e.to_str()) == Some("ico") {
                let bytes = fs::read(&path).map_err(|e| e.to_string())?;
                let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
                for i in 0..count {
                    let off = 6 + i * 16;
                    let sz = u32::from_le_bytes([bytes[off + 8], bytes[off + 9], bytes[off + 10], bytes[off + 11]]) as usize;
                    let img_off = u32::from_le_bytes([bytes[off + 12], bytes[off + 13], bytes[off + 14], bytes[off + 15]]) as usize;
                    let slice = &bytes[img_off..img_off + sz];
                    if slice.starts_with(expected_png_sig) {
                        verify_png_chunks(&path, slice, expected_png_sig, &pre_rename)?;
                    }
                }
            }
        }
    }
    if verified_pngs < 12 {
        return Err(format!("expected >= 12 PNGs under assets/app-icon, checked {verified_pngs}"));
    }

    let dot_ext = join(&[".pr", "ism"]);
    let cmd_id = join(&["table.", "im", "port", "Pr", "ism"]);
    let parity_doc = fs::read_to_string(root.join("docs/evidence/overview.md")).map_err(|e| e.to_string())?;
    if !parity_doc.contains("../../AGENTS.md#limited-local-jpg-reference-policy") || !parity_doc.contains("### Calibration") {
        return Err(String::from("docs/evidence/overview.md missing required policy link or Calibration heading"));
    }
    for doc_rel in [
        "docs/evidence/overview.md",
        "docs/evidence/curve-fitting.md",
        "docs/evidence/statistics.md",
        "docs/evidence/graphing.md",
        "docs/implementation/curve-fitting-implementation.md",
        "docs/implementation/data-tables-and-transforms.md",
        "docs/implementation/graphing-and-layouts.md",
        "docs/implementation/statistics-implementation.md",
        "docs/history/release/alpha-branding-clearance.md",
    ] {
        let content = fs::read_to_string(root.join(doc_rel)).map_err(|e| format!("read {doc_rel}: {e}"))?;
        let masked = content.replace(&dot_ext, ".fmt_ext").replace(&cmd_id, "table.import_ext");
        for probe in banned_probe_tokens() {
            if contains_ascii_word(&masked, &probe) {
                return Err(format!("{doc_rel} still contains prohibited term `{probe}`"));
            }
        }
    }

    Ok(())
}

fn verify_png_chunks(path: &Path, bytes: &[u8], png_sig: &[u8; 8], pre_rename: &str) -> Result<(), String> {
    if !bytes.starts_with(png_sig) {
        return Err(format!("{}: invalid PNG signature", path.display()));
    }
    let mut pos = 8usize;
    while pos + 12 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize;
        let ctype = &bytes[pos + 4..pos + 8];
        let cdata = &bytes[pos + 8..pos + 8 + len];
        let stored_crc = u32::from_be_bytes([bytes[pos + 8 + len], bytes[pos + 9 + len], bytes[pos + 10 + len], bytes[pos + 11 + len]]);
        let computed_crc = crc32_ieee(&[ctype, cdata].concat());
        if stored_crc != computed_crc {
            return Err(format!("{}: PNG chunk {:?} CRC mismatch", path.display(), ctype));
        }
        if matches!(ctype, b"tEXt" | b"iTXt") {
            let text = String::from_utf8_lossy(cdata);
            if contains_ascii_word(&text, pre_rename) {
                return Err(format!("{}: PNG metadata still contains pre-rename token", path.display()));
            }
        }
        pos += 12 + len;
        if ctype == b"IEND" {
            break;
        }
    }
    Ok(())
}

struct TestGuiHost {
    app: UiApp,
}

impl GuiSessionHost for TestGuiHost {
    fn call_gui(&mut self, method: &str, mut params: serde_json::Value) -> Result<serde_json::Value, curvebook_engine::EngineError> {
        if method == "ui.screenshot"
            && let Some(obj) = params.as_object_mut()
            && matches!(obj.get("target").and_then(serde_json::Value::as_str), Some("workspace" | "viewport"))
        {
            obj.insert(String::from("target"), serde_json::Value::String(String::from("full_window")));
        }
        curvebook_ui_egui::dispatch_gui_action(&mut self.app, method, params)
    }

    fn set_gui_only(&mut self, gui_only: bool) {
        self.app.set_gui_only(gui_only);
    }

    fn is_gui_only(&self) -> bool {
        self.app.is_gui_only()
    }

    fn hosted_session(&self) -> Option<&Session> {
        Some(self.app.session())
    }

    fn hosted_session_mut(&mut self) -> Option<&mut Session> {
        Some(self.app.session_mut())
    }

    fn sync_after_session_mutation(&mut self) {
        self.app.sync_views();
    }
}

fn check_mcp_json_strict(ctx: &str, method: &str, called_tool: Option<&str>, resp: &serde_json::Value) -> Result<(), String> {
    let pre_rename = join(&["lum", "encraft"]);
    let raw_ext = join(&["pr", "ism"]);
    let dot_ext = join(&[".pr", "ism"]);
    let cmd_id = join(&["table.", "im", "port", "Pr", "ism"]);
    let mcp_tool_id = join(&["table_im", "port_pr", "ism"]);
    let serialized = serde_json::to_string(resp).map_err(|e| e.to_string())?;

    let mut masked = serialized;
    if method == "tools/list" {
        if let Some(tools) = resp.pointer("/result/tools").and_then(serde_json::Value::as_array) {
            for tool in tools {
                let name = tool.get("name").and_then(serde_json::Value::as_str).unwrap_or("");
                if name == "open_file" {
                    let desc = tool.get("description").and_then(serde_json::Value::as_str).unwrap_or("");
                    if desc.contains(&dot_ext) {
                        let clean_desc = desc.replace(&dot_ext, ".ext");
                        masked = masked.replace(desc, &clean_desc);
                    }
                    let fmt_enum_pat =
                        format!("\"enum\":[\"curvebook\",\"curvebook.d\",\"cbtemplate\",\"{}\",\"pzfx\",\"xlsx\",\"csv\",\"tsv\"]", raw_ext);
                    masked = masked.replace(&fmt_enum_pat, "\"enum\":[\"curvebook\",\"pzfx\"]");
                } else if name == mcp_tool_id {
                    masked = masked.replace(&mcp_tool_id, "table_import_ext");
                    masked = masked.replace(&format!("[{cmd_id}]"), "[table.import_ext]");
                }
            }
        }
    } else if method == "tools/call" && matches!(called_tool, Some("list_toolsets" | "command_list" | "ui_inspect")) {
        masked = masked.replace(&mcp_tool_id, "table_import_ext");
        masked = masked.replace(&cmd_id, "table.import_ext");
    }

    // Strip base64 PNG image data blocks after verifying their binary PNG chunks cleanly.
    if method == "tools/call"
        && let Some(blocks) = resp.pointer("/result/content").and_then(serde_json::Value::as_array)
    {
        for block in blocks {
            if block.get("type").and_then(serde_json::Value::as_str) == Some("image")
                && let Some(b64) = block.get("data").and_then(serde_json::Value::as_str)
            {
                let png = decode_b64(b64).ok_or_else(|| format!("{ctx}: invalid base64 PNG"))?;
                verify_png_chunks(Path::new(ctx), &png, b"\x89PNG\r\n\x1a\n", &pre_rename)?;
                masked = masked.replace(b64, "CLEAN_PNG_DATA");
            }
        }
    }

    if contains_ascii_word(&masked, &pre_rename) {
        return Err(format!("{ctx}: MCP response leaked pre-rename token"));
    }
    for probe in banned_probe_tokens() {
        if contains_ascii_word(&masked, &probe) {
            return Err(format!("{ctx}: MCP response leaked prohibited token `{probe}`"));
        }
    }
    Ok(())
}

fn decode_b64(b64: &str) -> Option<Vec<u8>> {
    let mut vals = Vec::with_capacity(b64.len());
    for &b in b64.as_bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => 255,
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        vals.push(v);
    }
    if !vals.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity((vals.len() / 4) * 3);
    let (pairs, _) = vals.as_chunks::<4>();
    for &[a, b, c, d] in pairs {
        if a > 63 || b > 63 {
            return None;
        }
        out.push((a << 2) | (b >> 4));
        if c <= 63 {
            out.push((b << 4) | (c >> 2));
            if d <= 63 {
                out.push((c << 6) | d);
            }
        }
    }
    Some(out)
}

#[test]
fn r021_mcp_jsonrpc_headless_and_gui_only_positive_and_negative_regression() -> Result<(), String> {
    let mut doc = ProjectDocument::new("R021 MCP Sweep").map_err(|e| e.to_string())?;
    for (idx, fixture) in SAMPLE_FIXTURES.iter().enumerate() {
        let ds = instantiate_sample_fixture(fixture.id, format!("data-{}", idx + 1)).map_err(|e| e.to_string())?;
        doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    }
    doc.add_info_sheet(InfoSheet::new_preseeded("info-1", "Study Info").map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let spec =
        AnalysisSpec::new("res-1", "data-2", AnalysisKind::OneWayAnova(curvebook_doc::OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    doc.add_result_sheet(ResultSheet::new(spec).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if let Some(ds_xy) = doc.data_sheet(&SheetId::new("data-1")).cloned() {
        let mut g1 = GraphSheet::new_uninitialized_for_data("graph-1", &ds_xy).map_err(|e| e.to_string())?;
        g1.initialized = true;
        doc.add_graph_sheet(g1).map_err(|e| e.to_string())?;
    }
    let mut lspec = LayoutSpec::default();
    lspec.items.push(LayoutItem::LiveGraph {
        id: String::from("p1"),
        graph_sheet_id: SheetId::new("graph-1"),
        bounds_pt: curvebook_doc::RectPt::new(16.0, 16.0, 200.0, 140.0),
        scale_percent: 100.0,
        badge_override: Some(String::from("A")),
    });
    doc.add_layout_sheet(LayoutSheet::new("layout-1", "Figure 1", lspec).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let mut session = Session::from_document(doc);
    session.recalculate_all().map_err(|e| e.to_string())?;

    // 1. Positive headless MCP JSON-RPC verification across initialize, tools, resources, prompts, doc_inspect, render_preview, and export.
    let mut headless = McpServerState::from_session(session.clone());
    let rpc = |st: &mut McpServerState, method: &str, params: serde_json::Value| -> Result<serde_json::Value, String> {
        dispatch_json_value(serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }), st)
            .ok_or_else(|| format!("no reply for {method}"))
    };

    let init = rpc(&mut headless, "initialize", serde_json::json!({ "protocolVersion": "2024-11-05" }))?;
    if init.pointer("/result/serverInfo/name").and_then(serde_json::Value::as_str) != Some("curvebook") {
        return Err(String::from("unexpected MCP serverInfo.name"));
    }
    check_mcp_json_strict("headless:initialize", "initialize", None, &init)?;

    let core_tools = rpc(&mut headless, "tools/list", serde_json::json!({}))?;
    check_mcp_json_strict("headless:tools/list:core", "tools/list", None, &core_tools)?;

    let tsets = rpc(&mut headless, "tools/call", serde_json::json!({ "name": "list_toolsets", "arguments": {} }))?;
    check_mcp_json_strict("headless:list_toolsets", "tools/call", Some("list_toolsets"), &tsets)?;

    let _ = rpc(
        &mut headless,
        "tools/call",
        serde_json::json!({ "name": "enable_toolset", "arguments": { "group": "all", "groups": ["gui"], "enabled": true } }),
    )?;
    let all_tools = rpc(&mut headless, "tools/list", serde_json::json!({}))?;
    check_mcp_json_strict("headless:tools/list:all", "tools/list", None, &all_tools)?;

    let res_list = rpc(&mut headless, "resources/list", serde_json::json!({}))?;
    check_mcp_json_strict("headless:resources/list", "resources/list", None, &res_list)?;
    let uris: Vec<String> = res_list
        .pointer("/result/resources")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|r| r.get("uri").and_then(serde_json::Value::as_str).map(str::to_owned))
        .collect();
    if uris.len() < 6 {
        return Err(format!("expected >= 6 MCP resources, got {}", uris.len()));
    }
    for uri in &uris {
        let rread = rpc(&mut headless, "resources/read", serde_json::json!({ "uri": uri }))?;
        check_mcp_json_strict(&format!("headless:resources/read:{uri}"), "resources/read", None, &rread)?;
    }

    let plist = rpc(&mut headless, "prompts/list", serde_json::json!({}))?;
    check_mcp_json_strict("headless:prompts/list", "prompts/list", None, &plist)?;
    let pnames: Vec<String> = plist
        .pointer("/result/prompts")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|p| p.get("name").and_then(serde_json::Value::as_str).map(str::to_owned))
        .collect();
    for pname in &pnames {
        let pget = rpc(&mut headless, "prompts/get", serde_json::json!({ "name": pname, "arguments": {} }))?;
        check_mcp_json_strict(&format!("headless:prompts/get:{pname}"), "prompts/get", None, &pget)?;
    }

    let doc_ins = rpc(&mut headless, "tools/call", serde_json::json!({ "name": "doc_inspect", "arguments": {} }))?;
    check_mcp_json_strict("headless:doc_inspect", "tools/call", Some("doc_inspect"), &doc_ins)?;

    let preview = rpc(
        &mut headless,
        "tools/call",
        serde_json::json!({ "name": "render_preview", "arguments": { "sheet_id": "graph-1", "width_px": 240, "height_px": 180, "dpi": 144 } }),
    )?;
    check_mcp_json_strict("headless:render_preview", "tools/call", Some("render_preview"), &preview)?;

    let svg_exp = rpc(&mut headless, "tools/call", serde_json::json!({ "name": "export", "arguments": { "format": "svg", "sheet_id": "graph-1" } }))?;
    check_mcp_json_strict("headless:export:svg", "tools/call", Some("export"), &svg_exp)?;

    // 2. Positive strict --gui-only MCP JSON-RPC verification.
    let mut app = UiApp::from_session(session);
    app.sync_views();
    app.set_gui_only(true);
    let mut gui_mcp = McpServerState::with_gui_host(Box::new(TestGuiHost { app }));
    let g_init = rpc(&mut gui_mcp, "initialize", serde_json::json!({ "protocolVersion": "2024-11-05" }))?;
    check_mcp_json_strict("gui_only:initialize", "initialize", None, &g_init)?;
    let g_tools = rpc(&mut gui_mcp, "tools/list", serde_json::json!({}))?;
    check_mcp_json_strict("gui_only:tools/list", "tools/list", None, &g_tools)?;
    let g_prompts = rpc(&mut gui_mcp, "prompts/list", serde_json::json!({}))?;
    check_mcp_json_strict("gui_only:prompts/list", "prompts/list", None, &g_prompts)?;
    for pname in &pnames {
        let gpget = rpc(&mut gui_mcp, "prompts/get", serde_json::json!({ "name": pname, "arguments": {} }))?;
        check_mcp_json_strict(&format!("gui_only:prompts/get:{pname}"), "prompts/get", None, &gpget)?;
    }
    let g_ins = rpc(&mut gui_mcp, "tools/call", serde_json::json!({ "name": "ui_inspect", "arguments": { "scope": "all" } }))?;
    check_mcp_json_strict("gui_only:ui_inspect", "tools/call", Some("ui_inspect"), &g_ins)?;
    let g_shot = rpc(
        &mut gui_mcp,
        "tools/call",
        serde_json::json!({ "name": "ui_screenshot", "arguments": { "target": "workspace", "width_px": 240, "height_px": 180, "dpi": 144 } }),
    )?;
    check_mcp_json_strict("gui_only:ui_screenshot", "tools/call", Some("ui_screenshot"), &g_shot)?;

    // 3. Negative regression probes proving contaminated MCP instructions, prompts, resources, and tool descriptions fail.
    let vendor_gp = join(&["Gr", "aph", "Pad"]);
    let prod_pr = join(&["Pr", "ism"]);
    let vendor_op = join(&["Ori", "gin", "Pro"]);
    let suite_ac = join(&["Art", "Cr", "aft"]);
    let old_lc = join(&["Lum", "en", "Craft"]);

    let bad_init = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": { "serverInfo": { "name": "curvebook" }, "instructions": format!("MCP helper for {vendor_gp} workflows") }
    });
    if check_mcp_json_strict("neg:init", "initialize", None, &bad_init).is_ok() {
        return Err(String::from("contaminated MCP instructions failed to trigger rejection"));
    }

    let bad_prompt = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "result": { "description": "fit", "messages": [{ "role": "user", "content": { "type": "text", "text": format!("Compare with {vendor_op}") } }] }
    });
    if check_mcp_json_strict("neg:prompt", "prompts/get", None, &bad_prompt).is_ok() {
        return Err(String::from("contaminated MCP prompt failed to trigger rejection"));
    }

    let bad_res = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "result": { "contents": [{ "uri": "sheet://data-1", "mimeType": "application/json", "text": format!("{{\"title\":\"{suite_ac} Study\"}}") }] }
    });
    if check_mcp_json_strict("neg:resource", "resources/read", None, &bad_res).is_ok() {
        return Err(String::from("contaminated MCP resource failed to trigger rejection"));
    }

    let bad_tool_desc = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 4,
        "result": { "tools": [{ "name": "open_file", "description": format!("Open {prod_pr} (.prism) or {old_lc} files.") }] }
    });
    if check_mcp_json_strict("neg:tool", "tools/list", None, &bad_tool_desc).is_ok() {
        return Err(String::from("contaminated MCP open_file product description failed to trigger rejection"));
    }

    Ok(())
}

#[test]
fn r021_icns_gif_svg_docs_images_and_windows_ico_geometry_intact() -> Result<(), String> {
    let root = workspace_root()?;
    let png_sig = b"\x89PNG\r\n\x1a\n";
    let pre_rename = join(&["lum", "encraft"]);

    // 1. Verify docs/images/curvebook.png and assets/app-icon/slack/curvebook.png chunks + CRCs.
    for rel in ["docs/images/curvebook.png", "assets/app-icon/slack/curvebook.png"] {
        let bytes = fs::read(root.join(rel)).map_err(|e| format!("read {rel}: {e}"))?;
        verify_png_chunks(&root.join(rel), &bytes, png_sig, &pre_rename)?;
    }

    // 2. Verify curvebook.icns header, embedded PNG frames, and non-PNG `info` plist chunk.
    let icns = fs::read(root.join("assets/app-icon/curvebook.icns")).map_err(|e| e.to_string())?;
    if !icns.starts_with(b"icns") || icns.len() < 8 {
        return Err(String::from("curvebook.icns missing `icns` magic"));
    }
    let icns_total = u32::from_be_bytes([icns[4], icns[5], icns[6], icns[7]]) as usize;
    if icns_total != icns.len() {
        return Err(format!("curvebook.icns length mismatch: header={icns_total}, actual={}", icns.len()));
    }
    let mut pos = 8usize;
    let mut png_subicons = 0usize;
    while pos + 8 <= icns.len() {
        let tag = &icns[pos..pos + 4];
        let elen = u32::from_be_bytes([icns[pos + 4], icns[pos + 5], icns[pos + 6], icns[pos + 7]]) as usize;
        if elen < 8 || pos + elen > icns.len() {
            return Err(String::from("curvebook.icns malformed sub-chunk bounds"));
        }
        let payload = &icns[pos + 8..pos + elen];
        if payload.starts_with(png_sig) {
            verify_png_chunks(Path::new("curvebook.icns"), payload, png_sig, &pre_rename)?;
            png_subicons += 1;
        } else if tag == b"info" {
            let lossy = String::from_utf8_lossy(payload);
            if contains_ascii_word(&lossy, &pre_rename) {
                return Err(String::from("curvebook.icns `info` chunk contains pre-rename token"));
            }
            for probe in banned_probe_tokens() {
                if contains_ascii_word(&lossy, &probe) {
                    return Err(format!("curvebook.icns `info` chunk leaked `{probe}`"));
                }
            }
        }
        pos += elen;
    }
    if png_subicons < 8 {
        return Err(format!("expected >= 8 PNG sub-icons inside curvebook.icns, found {png_subicons}"));
    }

    // 3. Verify curvebook.svg, curvebook-small.svg, and org.curvebook.CurveBook.svg (outer XML + base64 PNG).
    for rel in
        ["assets/app-icon/curvebook.svg", "assets/app-icon/curvebook-small.svg", "assets/app-icon/hicolor/scalable/apps/org.curvebook.CurveBook.svg"]
    {
        let svg = fs::read_to_string(root.join(rel)).map_err(|e| format!("read {rel}: {e}"))?;
        let marker = "data:image/png;base64,";
        let Some((before, after)) = svg.split_once(marker) else {
            return Err(format!("{rel} missing embedded base64 PNG"));
        };
        let Some((b64, tail)) = after.split_once('"') else {
            return Err(format!("{rel} unterminated data URI"));
        };
        let decoded = decode_b64(b64).ok_or_else(|| format!("{rel} invalid base64 PNG"))?;
        verify_png_chunks(Path::new(rel), &decoded, png_sig, &pre_rename)?;
        let outer = format!("{before}\"{tail}");
        if contains_ascii_word(&outer, &pre_rename) {
            return Err(format!("{rel} outer XML contains pre-rename token"));
        }
        for probe in banned_probe_tokens() {
            if contains_ascii_word(&outer, &probe) {
                return Err(format!("{rel} outer XML leaked `{probe}`"));
            }
        }
    }

    // 4. Verify curvebook-subtle.gif header & absence of legacy/vendor strings, plus github-pages.md.
    let gif = fs::read(root.join("assets/app-icon/slack/curvebook-subtle.gif")).map_err(|e| e.to_string())?;
    if !gif.starts_with(b"GIF89a") {
        return Err(String::from("curvebook-subtle.gif missing GIF89a header"));
    }
    let gif_lossy = String::from_utf8_lossy(&gif).to_ascii_lowercase();
    if gif_lossy.contains(&pre_rename) {
        return Err(String::from("curvebook-subtle.gif contains pre-rename token"));
    }
    let pages_doc = fs::read_to_string(root.join("docs/release/github-pages.md")).map_err(|e| e.to_string())?;
    let dot_ext = format!(".{}", join(&["pr", "ism"]));
    let pages_cleaned = pages_doc.replace(&dot_ext, ".ext");
    for probe in banned_probe_tokens() {
        if contains_ascii_word(&pages_cleaned, &probe) {
            return Err(format!("docs/release/github-pages.md leaked `{probe}`"));
        }
    }

    // 5. Verify curvebook.ico directory invariants matching packaging/windows/check-icons.ps1.
    let ico = fs::read(root.join("assets/app-icon/curvebook.ico")).map_err(|e| e.to_string())?;
    let reserved = u16::from_le_bytes([ico[0], ico[1]]);
    let icon_type = u16::from_le_bytes([ico[2], ico[3]]);
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    if reserved != 0 || icon_type != 1 || count != 7 {
        return Err(format!("invalid ICONDIR header: reserved={reserved}, type={icon_type}, count={count}"));
    }
    let dir_end = 6 + 16 * count;
    for i in 0..count {
        let entry = 6 + 16 * i;
        let entry_w = if ico[entry] == 0 { 256u32 } else { u32::from(ico[entry]) };
        let entry_h = if ico[entry + 1] == 0 { 256u32 } else { u32::from(ico[entry + 1]) };
        if ico[entry + 3] != 0 {
            return Err(format!("ICO frame {i} reserved byte non-zero"));
        }
        let sz = u32::from_le_bytes([ico[entry + 8], ico[entry + 9], ico[entry + 10], ico[entry + 11]]) as usize;
        let off = u32::from_le_bytes([ico[entry + 12], ico[entry + 13], ico[entry + 14], ico[entry + 15]]) as usize;
        if off < dir_end || off + sz > ico.len() {
            return Err(format!("ICO frame {i} out of bounds"));
        }
        let frame = &ico[off..off + sz];
        if !frame.starts_with(png_sig) || &frame[12..16] != b"IHDR" {
            return Err(format!("ICO frame {i} missing PNG/IHDR"));
        }
        let png_w = u32::from_be_bytes([frame[16], frame[17], frame[18], frame[19]]);
        let png_h = u32::from_be_bytes([frame[20], frame[21], frame[22], frame[23]]);
        if png_w != entry_w || png_h != entry_h {
            return Err(format!("ICO frame {i} dimension mismatch: dir={entry_w}x{entry_h}, ihdr={png_w}x{png_h}"));
        }
    }

    Ok(())
}
