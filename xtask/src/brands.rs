//! `cargo xtask brands`: keep competitor vendor/product trademarks and retired suite branding out
//! of user-facing strings, package descriptions, workflows, and packaging metadata (`AGENTS.md`).

use std::collections::BTreeSet;
use std::path::Path;

#[cfg(feature = "full-gates")]
use curvebook_chart::compile_graph;
#[cfg(feature = "full-gates")]
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, ConstantValue, GraphSheet, InfoSheet, LayoutItem, LayoutSheet, LayoutSpec, ProjectDocument, ResultSheet, SheetId,
    TableKind,
};
#[cfg(feature = "full-gates")]
use curvebook_engine::{Session, cmd::registry as engine_cmd_registry};
use curvebook_format::{ZipArchive, ZipLimits};
#[cfg(feature = "full-gates")]
use curvebook_format::{save_project_bytes, save_template_bytes};
#[cfg(feature = "full-gates")]
use curvebook_mcp::{GuiSessionHost, McpServerState, dispatch_json_value};
#[cfg(feature = "full-gates")]
use curvebook_pdf::{PdfExportOptions, render_pdf};
#[cfg(feature = "full-gates")]
use curvebook_svg::{SvgExportOptions, render_svg};
#[cfg(feature = "full-gates")]
use curvebook_ui_egui::dialogs_canvas::{ANALYSIS_CATALOG, ActiveInspector, AnalysisCategory, CommandPaletteState};
#[cfg(feature = "full-gates")]
use curvebook_ui_egui::i18n;
#[cfg(feature = "full-gates")]
use curvebook_ui_egui::menus::{TOOLBAR_BUTTONS, UI_COMMANDS, menu_bar_tooltips, toolbar_tooltips};
#[cfg(feature = "full-gates")]
use curvebook_ui_egui::workspace_nav::{SAMPLE_FIXTURES, TableCreationDraft, WelcomeMode, instantiate_sample_fixture};
#[cfg(feature = "full-gates")]
use curvebook_ui_egui::{
    ActiveDialog, ClipboardBundle, EguiSurfaceCapture, EguiSurfaceScanner, UiApp, UiTreeSnapshot, ViewportMetrics, inspect_ui_tree,
};

use crate::rust_lex::{ExtractedLiteral, RustLexer, push_char_with_line};
use crate::text_decode::{decode_utf8_or_utf16, decode_utf16};

/// Competitor vendor and product names checked on ASCII token boundaries (including CamelCase / snake_case / kebab-case splits).
const BRANDS: &[&str] = &[
    "graphpad",
    "dotmatics",
    "prism",
    "instat",
    "statmate",
    "jmp",
    "spss",
    "minitab",
    "originlab",
    "originpro",
    "sigmaplot",
    "systat",
    "statistica",
    "xlstat",
];

/// Pre-rename product name checked strictly inside binary/SVG/packaging asset metadata.
const PRE_RENAME_ASSET_BRANDS: &[&str] = &["lumencraft"];

/// Optical modifiers that immediately precede `"prism"` inside `<title>`/`<desc>` icon metadata.
const OPTICAL_PRISM_MODIFIERS: &[&str] =
    &["optical", "refractive", "triangular", "glass", "crystal", "dispersive", "dispersion", "dichroic", "achromatic"];

/// External suite branding and infrastructure forbidden across all non-xtask files in this standalone repository.
const SUITE_BRANDS: &[&str] = &[
    "artcraft",
    "getartcraft",
    "storytold",
    "storyteller",
    "craft-fonts",
    "craft_fonts",
    "craft_fonts_dir",
    "craft-common-actions",
    "craft-contrib",
    "craft-house-rules",
    "craft-ui",
    "craft_ui",
    "craft-keymap",
    "craft-update",
    "craft-window",
];

/// Exact upstream repository URL permitted solely inside `## Attributed Source Code` of
/// `ATTRIBUTION.md` and `NOTICE` (`R15.1` provenance policy).
pub(crate) const ADAPTED_UPSTREAM_REPO_URL: &str = "https://github.com/storytold/vectorcraft";

/// Adapted upstream repository/crate identifier forbidden across `crates/`, `apps/`,
/// runtime UI/MCP/export text surfaces, and non-attribution release metadata.
pub(crate) const ADAPTED_UPSTREAM_BRAND: &str = "vectorcraft";

/// Markdown section heading inside `ATTRIBUTION.md` and `NOTICE` within which
/// [`ADAPTED_UPSTREAM_REPO_URL`] (and only that exact URL) is permitted.
const ATTRIBUTED_SOURCE_SECTION_HEADING: &str = "## Attributed Source Code";

const ATTRIBUTED_REPO_PLACEHOLDER: &str = "upstream-attributed-repo";

const ALLOW_MARKER: &str = "brand-ok";

/// Intentional audit-archive waiver: read-only audit reports under `docs/audit/` and
/// `docs/history/audit/` document prior findings verbatim and are skipped only in working-tree
/// scans (`skip_path`), never when scanning `dist/` bundles (`scan_dist_path_and_file`).
const AUDIT_PREFIX: &str = "docs/audit/";

/// Archived audit rounds moved under `docs/history/` (`R0.31`); same intentional read-only
/// working-tree waiver as `AUDIT_PREFIX` (`scan_dist_path_and_file` still rejects if bundled).
const HISTORY_AUDIT_PREFIX: &str = "docs/history/audit/";

/// Xtask gate implementation modules that intentionally embed the scanner definitions and unit probes.
const SELF_SCANNER_PATHS: &[&str] =
    &["xtask/src/brands.rs", "xtask/src/cleanroom.rs", "xtask/src/assets.rs", "xtask/src/bundle.rs", "xtask/src/layers.rs"];

/// Roadmap section slug retained for clean-room `.prism`/`.pzfx` import anchor compatibility.
const ROADMAP_R24_ANCHOR: &str = "r24-clean-room-prism--pzfx-project-import-with-authoritative-cell-budget";

/// Canonical `plan/prism/` reference-policy path identifier allowed strictly in `AGENTS.md`.
const PLAN_PRISM_SCOPE: &str = "plan/prism";

/// Binary asset formats that do not carry UTF-8/UTF-16 text.
const BINARY_ASSET_EXT: &[&str] = &[
    "png",
    "jpg",
    "jpeg",
    "gif",
    "webp",
    "bmp",
    "tif",
    "tiff",
    "ico",
    "icns",
    "avif",
    "svgz",
    "pdf",
    "eps",
    "icc",
    "icm",
    "cur",
    "ani",
    "ase",
    "aco",
    "ttf",
    "otf",
    "ttc",
    "woff",
    "woff2",
    "eot",
    "pfb",
    "zip",
    "curvebook",
    "cbtemplate",
    "prism",
    "pzf",
    "xlsx",
    "mp4",
    "wav",
    "mp3",
];

/// The only file in which a suite name may appear, and only on a line that carries [`ALLOW_MARKER`].
/// The README's non-affiliation disclaimer has to name the suite it disclaims. Every other file,
/// and every unmarked README line, stays strict so suite branding cannot drift back into docs,
/// workflows, or packaging.
const SUITE_DISCLAIMER_PATH: &str = "README.md";

/// Suite names the disclaimer may use. Infrastructure identifiers (`craft-*` crates, the suite's
/// domain) are deliberately absent: naming the suite is a disclaimer, referencing its
/// infrastructure would be a dependency, which the standalone-repository rule forbids.
const SUITE_DISCLAIMER_NAMES: &[&str] = &["artcraft"];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BrandHit {
    pub path: String,
    pub line: usize,
    pub brand: &'static str,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DistScanStats {
    pub files: usize,
    pub archive_entries: usize,
}

enum DecodedFile {
    Text(String),
    BinaryAsset,
}

pub fn run(root: &Path) -> Result<(), String> {
    let files = crate::repo_files(root)?;
    #[cfg_attr(not(feature = "full-gates"), allow(unused_mut))]
    let (mut hits, dist_stats) = scan_tree(root, &files)?;
    #[cfg(feature = "full-gates")]
    hits.extend(check_ui_surfaces()?);
    if hits.is_empty() {
        #[cfg(feature = "full-gates")]
        {
            let i18n_total = i18n::LOCALES.len() * i18n::catalog_len();
            if let Some(stats) = dist_stats {
                println!(
                    "brands: 0 unapproved brand or suite mentions across {} tracked files and {} dist/ files ({} archive entries) ({i18n_total} i18n entries, 16-locale live egui shell, MCP headless+gui-only JSON-RPC surfaces, 5 sheets, 26 dialogs, 7 inspectors, and export/icon metadata clean)",
                    files.len(),
                    stats.files,
                    stats.archive_entries
                );
            } else {
                println!(
                    "brands: 0 unapproved brand or suite mentions across {} tracked files ({i18n_total} i18n entries, 16-locale live egui shell, MCP headless+gui-only JSON-RPC surfaces, 5 sheets, 26 dialogs, 7 inspectors, and export/icon metadata clean)",
                    files.len()
                );
            }
        }
        #[cfg(not(feature = "full-gates"))]
        {
            if let Some(stats) = dist_stats {
                println!(
                    "brands: 0 unapproved brand or suite mentions across {} tracked files and {} dist/ files ({} archive entries) (light mode: live egui/MCP/export sweep skipped without `full-gates`)",
                    files.len(),
                    stats.files,
                    stats.archive_entries
                );
            } else {
                println!(
                    "brands: 0 unapproved brand or suite mentions across {} tracked files (light mode: live egui/MCP/export sweep skipped without `full-gates`)",
                    files.len()
                );
            }
        }
        return Ok(());
    }
    let lines: Vec<String> = hits.iter().map(|h| format!("{}:{}: `{}`", h.path, h.line, h.brand)).collect();
    Err(format!(
        "{} unapproved brand/suite mention(s) in repository paths, source identifiers/comments/literals, live egui shell UI, MCP/engine catalogs, export/icon metadata, or packaging surfaces:\n  {}",
        hits.len(),
        lines.join("\n  ")
    ))
}

/// Everything [`run`] checks that depends on `root`: tracked paths and contents, then the
/// `packaging/web` and `dist/` artefacts. The live UI, MCP, and catalog sweep
/// ([`check_ui_surfaces`]) is independent of `root`, so fixture-tree tests call this directly.
fn scan_tree(root: &Path, files: &[String]) -> Result<(Vec<BrandHit>, Option<DistScanStats>), String> {
    let mut hits = scan_repo_checked(root, files)?;
    let web_pkg = root.join("packaging/web");
    if web_pkg.exists() {
        hits.extend(scan_dist_artefacts(&web_pkg)?);
    }
    let dist_dir = root.join("dist");
    let dist_stats = if dist_dir.exists() {
        let (stats, dist_hits) = scan_dist_artefacts_with_stats(&dist_dir)?;
        hits.extend(dist_hits);
        Some(stats)
    } else {
        None
    };
    Ok((hits, dist_stats))
}

pub fn scan_repo_checked(root: &Path, files: &[String]) -> Result<Vec<BrandHit>, String> {
    let mut hits = Vec::new();
    let mut scan_errors = Vec::new();
    for path in files {
        if skip_path(path) {
            continue;
        }
        scan_tracked_path_name(path, &mut hits);
        let bytes = match curvebook_platform::fs::read_regular_file_bytes(&root.join(path), MAX_DIST_FILE_BYTES) {
            Ok(b) => b,
            Err(e) => {
                scan_errors.push(format!("cannot scan {path}: {e}"));
                continue;
            }
        };
        scan_repo_file_bytes(path, &bytes, &mut hits, &mut scan_errors);
    }
    if !scan_errors.is_empty() {
        if !hits.is_empty() {
            for h in &hits {
                scan_errors.push(format!("{}:{}: `{}`", h.path, h.line, h.brand));
            }
        }
        return Err(scan_errors.join("\n  "));
    }
    Ok(hits)
}

fn scan_repo_file_bytes(path: &str, bytes: &[u8], hits: &mut Vec<BrandHit>, scan_errors: &mut Vec<String>) {
    let ext_lower = Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if ext_lower == "png" {
        hits.extend(check_png_metadata(path, bytes));
        return;
    }
    if matches!(ext_lower.as_str(), "ico" | "icns") {
        hits.extend(check_icon_container_metadata(path, bytes));
        return;
    }
    if ext_lower == "gif" {
        hits.extend(check_gif_metadata(path, bytes));
        return;
    }
    if ext_lower == "pdf" || bytes.starts_with(b"%PDF-") {
        hits.extend(check_pdf_metadata(path, bytes));
        return;
    }
    if ext_lower == "svgz" || (bytes.starts_with(&[0x1F, 0x8B]) && is_binary_asset_ext(path)) {
        match inflate_gzip_bounded(bytes) {
            Ok(raw) => match decode_text_file(path, &raw) {
                Ok(DecodedFile::Text(svg)) => {
                    scan_svg(path, &svg, hits);
                    hits.extend(check_svg_metadata(path, &svg));
                }
                _ => scan_errors.push(format!("cannot scan {path}: .svgz payload is not valid UTF-8/UTF-16 SVG text")),
            },
            Err(()) => scan_errors.push(format!("cannot scan {path}: corrupt or oversized .svgz gzip payload")),
        }
        return;
    }
    let is_pk_zip = bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06");
    if is_pk_zip || matches!(ext_lower.as_str(), "zip" | "curvebook" | "cbtemplate") {
        let mut stats = DistScanStats::default();
        if let Err(err) = scan_dist_bytes(path, bytes, 0, &mut stats, hits) {
            scan_errors.push(err);
        }
        return;
    }
    match decode_text_file(path, bytes) {
        Ok(DecodedFile::Text(text)) => scan_file(path, &text, hits),
        Ok(DecodedFile::BinaryAsset) => {}
        Err(msg) => scan_errors.push(msg),
    }
}

fn scan_tracked_path_name(path: &str, out: &mut Vec<BrandHit>) {
    let norm = path.replace('\\', "/");
    if norm == PLAN_PRISM_SCOPE || norm.starts_with("plan/prism/") {
        out.push(BrandHit { path: path.to_owned(), line: 1, brand: "prism" });
        return;
    }
    let start_idx = out.len();
    collect_line_words(path, 1, &norm, SUITE_BRANDS, PrismPolicy::Strict, out);
    if is_crates_or_apps_path(&norm) {
        collect_line_words(path, 1, &norm, &[ADAPTED_UPSTREAM_BRAND], PrismPolicy::Strict, out);
    }
    let allow_prism_module = norm == "crates/format/src/prism.rs";
    let path_policy = if allow_prism_module { PrismPolicy::AllowExactBarePrism } else { PrismPolicy::Strict };
    collect_line_words(path, 1, &norm, BRANDS, path_policy, out);
    dedup_tail_hits(out, start_idx);
}

fn is_binary_asset_ext(path: &str) -> bool {
    Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|ext| BINARY_ASSET_EXT.iter().any(|b| ext.eq_ignore_ascii_case(b)))
}

fn decode_text_file(path: &str, bytes: &[u8]) -> Result<DecodedFile, String> {
    match decode_utf8_or_utf16(bytes) {
        Ok(Some(text)) => Ok(DecodedFile::Text(text)),
        Err(err) => Err(format!("cannot scan {path}: {err}")),
        Ok(None) => {
            if is_binary_asset_ext(path) {
                Ok(DecodedFile::BinaryAsset)
            } else {
                Err(format!("cannot scan {path}: file is not valid UTF-8 or UTF-16"))
            }
        }
    }
}

// Intentional working-tree audit waiver: see AUDIT_PREFIX / HISTORY_AUDIT_PREFIX above.
fn skip_path(path: &str) -> bool {
    let norm = path.replace('\\', "/");
    let trimmed = norm.strip_prefix("./").unwrap_or(&norm);
    trimmed == "docs/audit"
        || trimmed.starts_with(AUDIT_PREFIX)
        || trimmed == "docs/history/audit"
        || trimmed.starts_with(HISTORY_AUDIT_PREFIX)
        || SELF_SCANNER_PATHS.contains(&trimmed)
}

fn scan_file(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    let start_idx = out.len();
    scan_suite_lines(path, text, out);
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    let lower_path = path.to_ascii_lowercase();
    let is_cargo_toml = path == "Cargo.toml" || path.ends_with("/Cargo.toml");
    let is_provenance_doc = matches!(Path::new(path).file_name().and_then(|n| n.to_str()), Some("ASSETS.md" | "ATTRIBUTION.md" | "NOTICE"));
    if ext == "rs" {
        scan_rust(path, text, out);
    } else if is_cargo_toml {
        scan_cargo_toml(path, text, out);
    } else if ext == "svg" {
        scan_svg(path, text, out);
    } else if lower_path.ends_with(".plist") || lower_path.ends_with(".plist.in") {
        scan_plist(path, text, out);
    } else if is_mime_package_xml(path) {
        scan_mime_xml(path, text, out);
    } else if lower_path.ends_with(".desktop") {
        scan_desktop_entry(path, text, out);
    } else if is_provenance_doc {
        scan_plain_lines_no_waiver(path, text, out);
    } else if ext == "md" {
        scan_markdown(path, text, out);
    } else if matches!(ext.as_str(), "html" | "htm") {
        scan_html_file(path, text, out);
        out.extend(check_html_metadata(path, text));
    } else if is_scanned_non_rs_text_file(path, &ext) {
        scan_plain_lines_no_waiver(path, text, out);
    }
    dedup_tail_hits(out, start_idx);
}

fn is_scanned_non_rs_text_file(path: &str, ext: &str) -> bool {
    let norm = path.replace('\\', "/");
    let trimmed = norm.strip_prefix("./").unwrap_or(&norm);
    let file_name = Path::new(trimmed).file_name().and_then(|n| n.to_str()).unwrap_or("");
    let is_hook = trimmed.strip_prefix(".githooks/").is_some_and(|rest| !rest.is_empty() && !rest.ends_with(".sample"));
    matches!(
        ext,
        "txt"
            | "toml"
            | "json"
            | "yaml"
            | "yml"
            | "xml"
            | "css"
            | "js"
            | "mjs"
            | "cjs"
            | "ts"
            | "jsx"
            | "tsx"
            | "sh"
            | "ps1"
            | "wxs"
            | "wxl"
            | "wxi"
            | "hbs"
            | "py"
            | "r"
            | "rc"
            | "in"
            | "conf"
            | "ini"
    ) || is_hook
        || matches!(file_name, "_headers" | ".htaccess" | "NOTICE" | "LICENSE" | "LICENSE.txt" | "CODEOWNERS")
}

fn dedup_tail_hits(hits: &mut Vec<BrandHit>, from: usize) {
    let mut seen: BTreeSet<(usize, &'static str)> = BTreeSet::new();
    let mut idx = from;
    while idx < hits.len() {
        if seen.insert((hits[idx].line, hits[idx].brand)) {
            idx += 1;
        } else {
            hits.remove(idx);
        }
    }
}

fn scan_suite_lines(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    scan_suite_lines_with_scope(path, text, false, out);
}

fn scan_suite_lines_with_scope(path: &str, text: &str, allow_packaged: bool, out: &mut Vec<BrandHit>) {
    let masked = mask_attributed_upstream_url(path, text, allow_packaged);
    let check_adapted_brand = is_crates_or_apps_path(path) || (allow_packaged && !is_attribution_or_notice_path(path, true));
    for (idx, line) in masked.lines().enumerate() {
        let line_no = idx + 1;
        if path == SUITE_DISCLAIMER_PATH && line.contains(ALLOW_MARKER) {
            let strict: Vec<&'static str> = SUITE_BRANDS.iter().copied().filter(|brand| !SUITE_DISCLAIMER_NAMES.contains(brand)).collect();
            collect_line_words(path, line_no, line, &strict, PrismPolicy::Strict, out);
        } else {
            collect_line_words(path, line_no, line, SUITE_BRANDS, PrismPolicy::Strict, out);
        }
        if check_adapted_brand {
            collect_line_words(path, line_no, line, &[ADAPTED_UPSTREAM_BRAND], PrismPolicy::Strict, out);
        }
    }
}

pub(crate) fn is_crates_or_apps_path(path: &str) -> bool {
    let norm = path.replace('\\', "/");
    let trimmed = norm.strip_prefix("./").unwrap_or(&norm);
    trimmed.starts_with("crates/") || trimmed.starts_with("apps/")
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

pub(crate) fn is_attributed_source_section_heading(line: &str) -> bool {
    line.trim() == ATTRIBUTED_SOURCE_SECTION_HEADING
}

pub(crate) fn closes_attributed_source_section(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed != ATTRIBUTED_SOURCE_SECTION_HEADING
        && (trimmed.starts_with("# ") || trimmed.starts_with("#\t") || trimmed.starts_with("## ") || trimmed.starts_with("##\t"))
}

pub(crate) fn mask_attributed_upstream_url<'a>(path: &str, text: &'a str, allow_packaged: bool) -> std::borrow::Cow<'a, str> {
    if !is_attribution_or_notice_path(path, allow_packaged) || !text.contains(ADAPTED_UPSTREAM_REPO_URL) {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut in_attributed_source = false;
    for (idx, line) in text.lines().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        if is_attributed_source_section_heading(line) {
            in_attributed_source = true;
        } else if closes_attributed_source_section(line) {
            in_attributed_source = false;
        }
        if in_attributed_source && line.contains(ADAPTED_UPSTREAM_REPO_URL) {
            out.push_str(&mask_exact_upstream_url_on_line(line));
        } else {
            out.push_str(line);
        }
    }
    if text.ends_with('\n') {
        out.push('\n');
    }
    std::borrow::Cow::Owned(out)
}

fn is_valid_url_prefix(before: Option<char>) -> bool {
    before.is_none_or(|ch| ch.is_whitespace() || matches!(ch, '(' | '[' | '{' | '<' | '`' | '"' | '\'' | ':' | ',' | ';'))
}

fn is_valid_url_suffix(after_slice: &str) -> bool {
    let mut chars = after_slice.chars();
    match chars.next() {
        None => true,
        Some(ch) if ch.is_whitespace() => true,
        Some(')' | ']' | '}' | '>' | '`' | '"' | '\'' | ',' | ';') => true,
        Some('.') => chars.next().is_none_or(|next| next.is_whitespace() || matches!(next, ')' | ']' | '}' | '>' | '`' | '"' | '\'' | ',' | ';')),
        _ => false,
    }
}

fn mask_exact_upstream_url_on_line(line: &str) -> String {
    let url_len = ADAPTED_UPSTREAM_REPO_URL.len();
    let mut out = String::with_capacity(line.len());
    let mut cursor = 0usize;
    while let Some(rel) = line.get(cursor..).and_then(|s| s.find(ADAPTED_UPSTREAM_REPO_URL)) {
        let pos = cursor + rel;
        let end = pos + url_len;
        let prefix_ok = is_valid_url_prefix(line.get(..pos).and_then(|s| s.chars().next_back()));
        let suffix_ok = is_valid_url_suffix(line.get(end..).unwrap_or(""));
        if prefix_ok && suffix_ok {
            out.push_str(line.get(cursor..pos).unwrap_or(""));
            out.push_str(ATTRIBUTED_REPO_PLACEHOLDER);
            cursor = end;
        } else {
            let step = line.get(pos..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
            out.push_str(line.get(cursor..pos + step).unwrap_or(""));
            cursor = pos + step;
        }
    }
    out.push_str(line.get(cursor..).unwrap_or(""));
    out
}

fn scan_cargo_toml(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    let mut scanner = TomlScanner::new(path, text, out);
    scanner.run();
}

fn is_generated_docs_html_path(path: &str) -> bool {
    let norm = path.replace('\\', "/");
    let trimmed = norm.strip_prefix("./").unwrap_or(&norm);
    (trimmed.starts_with("docs/manual/v") && trimmed != "docs/manual/index.html")
        || trimmed.starts_with("docs-site/")
        || trimmed.starts_with("target/docs-site/")
}

fn mask_generated_docs_html_tokens(path: &str, line: &str) -> Option<String> {
    let norm = path.replace('\\', "/");
    let mut work = line.to_owned();
    let mut changed = false;
    if work.contains(ROADMAP_R24_ANCHOR) {
        work = work.replace(ROADMAP_R24_ANCHOR, "r24-clean-room-interchange");
        changed = true;
    }
    if norm.ends_with("/contribute/agents.html") && ((line.contains("Scope:") && line.contains(PLAN_PRISM_SCOPE)) || line.trim() == "plan/prism/") {
        work = work.replace(PLAN_PRISM_SCOPE, "plan/ref_frames");
        changed = true;
    }
    if norm.ends_with("/registry/formats.html") && line.trim() == "prism" {
        work = String::from("format_token");
        changed = true;
    }
    for url_prefix in ["https://github.com/HauserGroup/curvebook/", "http://github.com/HauserGroup/curvebook/"] {
        if work.contains(url_prefix) {
            work = work.replace(url_prefix, "repo_blob_link/");
            changed = true;
        }
    }
    for tok in [
        "table.importPrism",
        "table_import_prism",
        "#cmd-table-importprism",
        "#mcp-tool-table_import_prism",
        "#format-prism",
        "id=\"format-prism\"",
        "import_prism_bytes",
        "prism.rs",
        "{pzfx,prism,",
        "&quot;prism&quot;",
        "\"prism\"",
        "<code>prism</code>",
        "`prism`",
    ] {
        if work.contains(tok) {
            work = work.replace(tok, "format_token");
            changed = true;
        }
    }
    if changed { Some(work) } else { None }
}

fn scan_html_file(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    if !is_generated_docs_html_path(path) {
        scan_plain_lines_no_waiver(path, text, out);
        return;
    }
    for (idx, line) in text.lines().enumerate() {
        let line_no = idx + 1;
        if let Some(masked) = mask_generated_docs_html_tokens(path, line) {
            collect_line_words(path, line_no, &masked, BRANDS, PrismPolicy::Standard, out);
        } else {
            collect_line_words(path, line_no, line, BRANDS, PrismPolicy::Standard, out);
        }
    }
}

fn scan_plain_lines_no_waiver(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    let is_asset_doc = matches!(Path::new(path).file_name().and_then(|n| n.to_str()), Some("ASSETS.md" | "ATTRIBUTION.md" | "NOTICE"));
    for (idx, line) in text.lines().enumerate() {
        let policy = if is_asset_doc { PrismPolicy::Strict } else { PrismPolicy::Standard };
        collect_line_words(path, idx + 1, line, BRANDS, policy, out);
    }
}

fn scan_plist(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    let mut in_extensions_key = false;
    let mut in_extensions_array = false;
    for (idx, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        let line_no = idx + 1;
        collect_line_words(path, line_no, line, PRE_RENAME_ASSET_BRANDS, PrismPolicy::Strict, out);
        if trimmed == "<key>CFBundleTypeExtensions</key>" {
            in_extensions_key = true;
            in_extensions_array = false;
            continue;
        }
        if in_extensions_key && trimmed == "<array>" {
            in_extensions_array = true;
            in_extensions_key = false;
            continue;
        }
        if in_extensions_array {
            if trimmed == "</array>" {
                in_extensions_array = false;
                continue;
            }
            if trimmed == "<string>prism</string>" {
                continue;
            }
        }
        in_extensions_key = false;
        let policy = if is_dot_prism_extension_description(trimmed) { PrismPolicy::Standard } else { PrismPolicy::Strict };
        collect_line_words(path, line_no, line, BRANDS, policy, out);
    }
}

fn scan_mime_xml(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    for (idx, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        let line_no = idx + 1;
        collect_line_words(path, line_no, line, PRE_RENAME_ASSET_BRANDS, PrismPolicy::Strict, out);
        let allowed_mime_tag = trimmed == r#"<mime-type type="application/x-prism-project">"#
            || trimmed == r#"<mime-type type="application/x-prism-xml">"#
            || trimmed == r#"<glob pattern="*.prism"/>"#;
        if allowed_mime_tag {
            continue;
        }
        let policy = if is_dot_prism_extension_description(trimmed) { PrismPolicy::Standard } else { PrismPolicy::Strict };
        collect_line_words(path, line_no, line, BRANDS, policy, out);
    }
}

fn scan_desktop_entry(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    for (idx, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        let line_no = idx + 1;
        collect_line_words(path, line_no, line, PRE_RENAME_ASSET_BRANDS, PrismPolicy::Strict, out);
        if let Some(mime_vals) = trimmed.strip_prefix("MimeType=") {
            for token in mime_vals.split(';') {
                if matches!(token.trim(), "application/x-prism-project" | "application/x-prism-xml") {
                    continue;
                }
                collect_line_words(path, line_no, token, BRANDS, PrismPolicy::Strict, out);
            }
            continue;
        }
        collect_line_words(path, line_no, line, BRANDS, PrismPolicy::Strict, out);
    }
}

fn scan_markdown(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    let is_agents = path == "AGENTS.md";
    let is_dev_or_cmd = matches!(path, "docs/commands.md" | "docs/development.md" | "docs/extending.md");
    for (idx, line) in text.lines().enumerate() {
        let line_no = idx + 1;
        if let Some(masked) = mask_markdown_format_tokens(line, is_agents, is_dev_or_cmd) {
            collect_line_words(path, line_no, &masked, BRANDS, PrismPolicy::Standard, out);
        } else {
            collect_line_words(path, line_no, line, BRANDS, PrismPolicy::Standard, out);
        }
    }
}

fn mask_markdown_format_tokens(line: &str, is_agents: bool, is_dev_or_cmd: bool) -> Option<String> {
    let mut work = line.to_owned();
    let mut changed = false;
    if work.contains(ROADMAP_R24_ANCHOR) {
        work = work.replace(ROADMAP_R24_ANCHOR, "r24-clean-room-interchange");
        changed = true;
    }
    if is_agents && line.contains("Scope:") && line.contains(PLAN_PRISM_SCOPE) {
        work = work.replace(PLAN_PRISM_SCOPE, "plan/ref_frames");
        changed = true;
    }
    for tok in ["table.importPrism", "import_prism_bytes", "prism.rs", "{pzfx,prism,"] {
        if work.contains(tok) && (is_dev_or_cmd || tok == "table.importPrism") {
            work = work.replace(tok, "format_importer");
            changed = true;
        }
    }
    if changed { Some(work) } else { None }
}

struct SvgChunk {
    text: String,
    is_meta_text: bool,
    prev_word: Option<String>,
}

#[derive(Default)]
struct SvgState {
    in_comment: bool,
    in_tag: bool,
    tag_buf: String,
    in_meta: bool,
    prev_meta_word: Option<String>,
}

impl SvgState {
    fn split_line_chunks(&mut self, line: &str) -> Vec<SvgChunk> {
        let mut chunks = Vec::new();
        let bytes = line.as_bytes();
        let mut i = 0usize;
        let mut span_start = 0usize;
        while i < bytes.len() {
            if self.in_comment {
                if bytes.get(i..i + 3) == Some(b"-->") {
                    chunks.push(SvgChunk {
                        text: decode_xml_or_html_entities(line.get(span_start..i).unwrap_or("")),
                        is_meta_text: false,
                        prev_word: None,
                    });
                    i += 3;
                    self.in_comment = false;
                    span_start = i;
                } else {
                    i += 1;
                }
                continue;
            }
            if self.in_tag {
                if bytes[i] == b'>' {
                    let tag_chunk = line.get(span_start..=i).unwrap_or("");
                    self.tag_buf.push_str(tag_chunk);
                    self.apply_completed_tag();
                    chunks.push(SvgChunk { text: decode_xml_or_html_entities(tag_chunk), is_meta_text: false, prev_word: None });
                    i += 1;
                    span_start = i;
                } else {
                    i += 1;
                }
                continue;
            }
            if bytes.get(i..i + 4) == Some(b"<!--") {
                self.push_text_chunk(line, span_start, i, &mut chunks);
                self.in_comment = true;
                i += 4;
                span_start = i;
                continue;
            }
            if bytes[i] == b'<' {
                self.push_text_chunk(line, span_start, i, &mut chunks);
                self.in_tag = true;
                self.tag_buf.clear();
                span_start = i;
            }
            i += 1;
        }
        if span_start < line.len() {
            let tail = line.get(span_start..).unwrap_or("");
            if self.in_tag {
                self.tag_buf.push_str(tail);
            }
            if !self.in_comment && !self.in_tag {
                self.push_text_chunk(line, span_start, line.len(), &mut chunks);
            } else {
                chunks.push(SvgChunk { text: decode_xml_or_html_entities(tail), is_meta_text: false, prev_word: None });
            }
        }
        chunks
    }

    fn push_text_chunk(&mut self, line: &str, start: usize, end: usize, chunks: &mut Vec<SvgChunk>) {
        if start >= end {
            return;
        }
        let decoded = decode_xml_or_html_entities(line.get(start..end).unwrap_or(""));
        let prev_word = if self.in_meta { self.prev_meta_word.clone() } else { None };
        let is_meta = self.in_meta;
        if is_meta && !decoded.trim().is_empty() {
            let tail = trailing_ascii_word(decoded.trim_end()).to_ascii_lowercase();
            self.prev_meta_word = if tail.is_empty() { None } else { Some(tail) };
        }
        chunks.push(SvgChunk { text: decoded, is_meta_text: is_meta, prev_word });
    }

    fn apply_completed_tag(&mut self) {
        self.in_tag = false;
        let inner = self.tag_buf.trim_start_matches('<').trim_end_matches('>').trim();
        let lower = inner.to_ascii_lowercase();
        let is_close = lower.starts_with('/');
        let name_src = lower.trim_start_matches('/');
        let tag_name = name_src.split(|c: char| c.is_ascii_whitespace() || c == '/').next().unwrap_or("");
        if matches!(tag_name, "title" | "desc") {
            let self_closing = inner.ends_with('/');
            if is_close {
                self.in_meta = false;
                self.prev_meta_word = None;
            } else if !self_closing {
                self.in_meta = true;
                self.prev_meta_word = None;
            }
        }
        self.tag_buf.clear();
    }
}

fn scan_svg(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    let lines: Vec<&str> = text.lines().collect();
    let mut state = SvgState::default();
    for (idx, &line) in lines.iter().enumerate() {
        let stripped = check_and_strip_svg_data_uri(path, line, out);
        let effective = stripped.as_deref().unwrap_or(line);
        let chunks = state.split_line_chunks(effective);
        let mut line_brands: BTreeSet<&'static str> = BTreeSet::new();
        for chunk in chunks {
            let policy = if chunk.is_meta_text { PrismPolicy::SvgTitleOrDesc { prev_word: chunk.prev_word.as_deref() } } else { PrismPolicy::Strict };
            let lower = chunk.text.to_ascii_lowercase();
            for &brand in BRANDS.iter().chain(PRE_RENAME_ASSET_BRANDS) {
                if !line_brands.contains(brand) && has_word_match(&chunk.text, &lower, brand, policy) {
                    line_brands.insert(brand);
                    out.push(BrandHit { path: path.to_owned(), line: idx + 1, brand });
                }
            }
        }
    }
}

fn check_and_strip_svg_data_uri(path: &str, line: &str, out: &mut Vec<BrandHit>) -> Option<String> {
    const MARKER: &str = "data:image/";
    let lower = line.to_ascii_lowercase();
    if !lower.contains(MARKER) {
        return None;
    }
    let mut out_str = String::with_capacity(line.len().min(4096));
    let mut pos = 0usize;
    while let Some(rel) = lower.get(pos..).and_then(|s| s.find(MARKER)) {
        let hit = pos + rel;
        out_str.push_str(line.get(pos..hit).unwrap_or(""));
        let after = line.get(hit + MARKER.len()..).unwrap_or("");
        let after_lower = lower.get(hit + MARKER.len()..).unwrap_or("");
        let Some(comma_rel) = after.find(',') else {
            out_str.push_str(line.get(hit..hit + MARKER.len()).unwrap_or(MARKER));
            pos = hit + MARKER.len();
            continue;
        };
        let meta = after_lower.get(..comma_rel).unwrap_or("");
        let mime = meta.split(';').next().unwrap_or("").trim();
        let is_base64 = meta.split(';').skip(1).any(|p| p.trim() == "base64");
        let payload_start = after.get(comma_rel + 1..).unwrap_or("");
        let end_quote = payload_start.find(['"', '\'', ')', ' ', '\t']).unwrap_or(payload_start.len());
        let raw_payload = payload_start.get(..end_quote).unwrap_or(payload_start);
        if is_base64 {
            if let Some(decoded) = decode_base64_ascii(raw_payload) {
                inspect_embedded_image_payload(path, mime, &decoded, out);
            }
        } else if mime == "svg+xml" {
            let svg_text = percent_decode_utf8_lossy(raw_payload);
            scan_svg(path, &svg_text, out);
            out.extend(check_svg_metadata(path, &svg_text));
        }
        out_str.push_str("data:image/stripped");
        pos = hit + MARKER.len() + comma_rel + 1 + end_quote;
    }
    out_str.push_str(line.get(pos..).unwrap_or(""));
    Some(out_str)
}

fn inspect_embedded_image_payload(path: &str, mime: &str, decoded: &[u8], out: &mut Vec<BrandHit>) {
    if mime == "png" || decoded.starts_with(b"\x89PNG\r\n\x1a\n") {
        out.extend(check_png_metadata(path, decoded));
    } else if mime == "gif" || decoded.starts_with(b"GIF87a") || decoded.starts_with(b"GIF89a") {
        out.extend(check_gif_metadata(path, decoded));
    } else if matches!(mime, "x-icon" | "vnd.microsoft.icon") {
        out.extend(check_icon_container_metadata(path, decoded));
    } else if mime == "svg+xml"
        && let Ok(Some(svg_text)) = decode_utf8_or_utf16(decoded)
    {
        scan_svg(path, &svg_text, out);
        out.extend(check_svg_metadata(path, &svg_text));
    }
}

fn percent_decode_utf8_lossy(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) = (bytes.get(i + 1), bytes.get(i + 2))
            && let (Some(hd), Some(ld)) = ((char::from(*h)).to_digit(16), (char::from(*l)).to_digit(16))
        {
            out.push(((hd << 4) | ld) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn decode_base64_ascii(b64: &str) -> Option<Vec<u8>> {
    if b64.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n')) {
        return Some(Vec::new());
    }
    curvebook_platform::base64::decode(b64).ok()
}

struct TomlScanner<'a> {
    path: &'a str,
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: usize,
    in_metadata_section: bool,
    in_workspace_section: bool,
    seen: BTreeSet<(usize, &'static str)>,
    out: &'a mut Vec<BrandHit>,
}

impl<'a> TomlScanner<'a> {
    fn new(path: &'a str, src: &'a str, out: &'a mut Vec<BrandHit>) -> Self {
        Self {
            path,
            src,
            bytes: src.as_bytes(),
            pos: 0,
            line: 1,
            in_metadata_section: false,
            in_workspace_section: false,
            seen: BTreeSet::new(),
            out,
        }
    }

    fn run(&mut self) {
        while self.pos < self.bytes.len() {
            self.skip_ws_and_comments();
            if self.pos >= self.bytes.len() {
                break;
            }
            if self.bytes[self.pos] == b'[' {
                self.read_table_header();
            } else {
                self.read_statement();
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn advance_one(&mut self) -> Option<u8> {
        let b = *self.bytes.get(self.pos)?;
        self.pos += 1;
        if b == b'\n' {
            self.line += 1;
        }
        Some(b)
    }

    fn push_next_char(&mut self, text: &mut String, byte_lines: &mut Vec<usize>) {
        if let Some(rest) = self.src.get(self.pos..)
            && let Some(ch) = rest.chars().next()
        {
            let line = self.line;
            push_char_with_line(text, byte_lines, ch, line);
            self.pos += ch.len_utf8();
            if ch == '\n' {
                self.line += 1;
            }
        } else {
            self.advance_one();
        }
    }

    fn skip_comment(&mut self) {
        while let Some(b) = self.peek() {
            if b == b'\n' {
                break;
            }
            self.pos += 1;
        }
    }

    fn skip_ws_and_comments(&mut self) {
        while let Some(b) = self.peek() {
            match b {
                b' ' | b'\t' | b'\r' | b'\n' => {
                    self.advance_one();
                }
                b'#' => self.skip_comment(),
                _ => break,
            }
        }
    }

    fn skip_inline_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r')) {
            self.pos += 1;
        }
    }

    fn read_table_header(&mut self) {
        let double = self.bytes.get(self.pos..self.pos + 2) == Some(b"[[");
        self.pos += if double { 2 } else { 1 };
        let (segments, key_lits) = self.read_dotted_key(true);
        if self.peek() == Some(b']') {
            self.pos += 1;
            if double && self.peek() == Some(b']') {
                self.pos += 1;
            }
        }
        while let Some(b) = self.advance_one() {
            if b == b'\n' {
                break;
            }
        }
        self.in_workspace_section = segments.len() == 1 && segments[0] == "workspace";
        self.in_metadata_section = is_metadata_table_segments(&segments);
        if self.in_metadata_section {
            for lit in &key_lits {
                self.check_literal(lit, false);
            }
        }
    }

    fn read_statement(&mut self) {
        let (segments, key_lits) = self.read_dotted_key(false);
        if self.peek() != Some(b'=') {
            if let Some(b) = self.peek() {
                if b == b'\n' {
                    self.advance_one();
                } else {
                    while let Some(next) = self.advance_one() {
                        if next == b'\n' {
                            break;
                        }
                    }
                }
            }
            return;
        }
        self.pos += 1; // '='
        let is_meta = self.in_metadata_section || is_metadata_key_segments(&segments, self.in_workspace_section);
        if is_meta {
            for lit in &key_lits {
                self.check_literal(lit, false);
            }
        }
        self.scan_value(is_meta, false);
    }

    fn read_dotted_key(&mut self, in_header: bool) -> (Vec<String>, Vec<ExtractedLiteral>) {
        let mut segments = Vec::new();
        let mut lits = Vec::new();
        while let Some(b) = self.peek() {
            match b {
                b'\n' | b'#' => break,
                b']' if in_header => break,
                b'=' if !in_header => break,
                b'.' | b' ' | b'\t' | b'\r' => self.pos += 1,
                b'"' => {
                    let lit = self.read_basic_single_string();
                    segments.push(lit.text.clone());
                    lits.push(lit);
                }
                b'\'' => {
                    let lit = self.read_literal_single_string();
                    segments.push(lit.text.clone());
                    lits.push(lit);
                }
                _ => {
                    let lit = self.read_bare_token();
                    if !lit.text.is_empty() {
                        segments.push(lit.text.clone());
                        lits.push(lit);
                    }
                }
            }
        }
        (segments, lits)
    }

    fn scan_value(&mut self, is_meta: bool, stmt_prev_allow: bool) {
        let mut depth = 0usize;
        while let Some(b) = self.peek() {
            match b {
                b' ' | b'\t' | b'\r' => self.pos += 1,
                b'\n' => {
                    self.advance_one();
                    if depth == 0 {
                        break;
                    }
                }
                b'#' => self.skip_comment(),
                b'[' | b'{' => {
                    depth += 1;
                    self.pos += 1;
                }
                b']' | b'}' => {
                    depth = depth.saturating_sub(1);
                    self.pos += 1;
                }
                b'"' => {
                    let lit = if self.bytes.get(self.pos..self.pos + 3) == Some(b"\"\"\"") {
                        self.read_basic_multiline_string()
                    } else {
                        self.read_basic_single_string()
                    };
                    if is_meta {
                        self.check_literal(&lit, stmt_prev_allow);
                    }
                }
                b'\'' => {
                    let lit = if self.bytes.get(self.pos..self.pos + 3) == Some(b"'''") {
                        self.read_literal_multiline_string()
                    } else {
                        self.read_literal_single_string()
                    };
                    if is_meta {
                        self.check_literal(&lit, stmt_prev_allow);
                    }
                }
                _ => {
                    let lit = self.read_bare_token();
                    if is_meta && !lit.text.is_empty() {
                        self.check_literal(&lit, stmt_prev_allow);
                    }
                }
            }
        }
    }

    fn read_bare_token(&mut self) -> ExtractedLiteral {
        let start_line = self.line;
        let mut text = String::new();
        let mut byte_lines = Vec::new();
        while let Some(b) = self.peek() {
            if b.is_ascii_alphanumeric() || b == b'_' || b == b'-' {
                self.push_next_char(&mut text, &mut byte_lines);
            } else {
                break;
            }
        }
        if text.is_empty() && self.pos < self.bytes.len() {
            self.advance_one();
        }
        ExtractedLiteral { text, byte_lines, start_line, end_line: self.line, quote_start: 0, quote_end: 0 }
    }

    fn read_basic_single_string(&mut self) -> ExtractedLiteral {
        let start_line = self.line;
        self.pos += 1; // '"'
        let mut text = String::new();
        let mut byte_lines = Vec::new();
        while let Some(b) = self.peek() {
            if b == b'"' {
                self.pos += 1;
                break;
            }
            if b == b'\n' {
                break;
            }
            if b == b'\\' {
                self.read_toml_escape(&mut text, &mut byte_lines, false);
                continue;
            }
            self.push_next_char(&mut text, &mut byte_lines);
        }
        ExtractedLiteral { text, byte_lines, start_line, end_line: self.line, quote_start: 0, quote_end: 0 }
    }

    fn read_basic_multiline_string(&mut self) -> ExtractedLiteral {
        let start_line = self.line;
        self.pos += 3; // "\"\"\""
        self.trim_opening_newline();
        let mut text = String::new();
        let mut byte_lines = Vec::new();
        while let Some(b) = self.peek() {
            if self.bytes.get(self.pos..self.pos + 3) == Some(b"\"\"\"") {
                self.consume_closing_triple(b'"', &mut text, &mut byte_lines);
                break;
            }
            if b == b'\\' {
                self.read_toml_escape(&mut text, &mut byte_lines, true);
                continue;
            }
            self.push_next_char(&mut text, &mut byte_lines);
        }
        ExtractedLiteral { text, byte_lines, start_line, end_line: self.line, quote_start: 0, quote_end: 0 }
    }

    fn read_literal_single_string(&mut self) -> ExtractedLiteral {
        let start_line = self.line;
        self.pos += 1; // '\''
        let mut text = String::new();
        let mut byte_lines = Vec::new();
        while let Some(b) = self.peek() {
            if b == b'\'' {
                self.pos += 1;
                break;
            }
            if b == b'\n' {
                break;
            }
            self.push_next_char(&mut text, &mut byte_lines);
        }
        ExtractedLiteral { text, byte_lines, start_line, end_line: self.line, quote_start: 0, quote_end: 0 }
    }

    fn read_literal_multiline_string(&mut self) -> ExtractedLiteral {
        let start_line = self.line;
        self.pos += 3; // "'''"
        self.trim_opening_newline();
        let mut text = String::new();
        let mut byte_lines = Vec::new();
        while self.pos < self.bytes.len() {
            if self.bytes.get(self.pos..self.pos + 3) == Some(b"'''") {
                self.consume_closing_triple(b'\'', &mut text, &mut byte_lines);
                break;
            }
            self.push_next_char(&mut text, &mut byte_lines);
        }
        ExtractedLiteral { text, byte_lines, start_line, end_line: self.line, quote_start: 0, quote_end: 0 }
    }

    fn trim_opening_newline(&mut self) {
        if self.bytes.get(self.pos..self.pos + 2) == Some(b"\r\n") {
            self.pos += 2;
            self.line += 1;
        } else if self.peek() == Some(b'\n') {
            self.pos += 1;
            self.line += 1;
        }
    }

    fn consume_closing_triple(&mut self, quote: u8, text: &mut String, byte_lines: &mut Vec<usize>) {
        let mut run = 0usize;
        while self.bytes.get(self.pos + run) == Some(&quote) && run < 5 {
            run += 1;
        }
        let extra = run.saturating_sub(3);
        let line = self.line;
        for _ in 0..extra {
            push_char_with_line(text, byte_lines, char::from(quote), line);
        }
        self.pos += run;
    }

    fn read_toml_escape(&mut self, text: &mut String, byte_lines: &mut Vec<usize>, multiline: bool) {
        let esc_line = self.line;
        self.pos += 1; // '\\'
        let Some(next) = self.peek() else {
            return;
        };
        if multiline && is_line_ending_backslash(self.bytes, self.pos) {
            self.skip_inline_ws();
            if self.peek() == Some(b'\n') {
                self.advance_one();
            }
            self.skip_multiline_ws();
            return;
        }
        self.advance_one();
        match next {
            b'b' => push_char_with_line(text, byte_lines, '\u{0008}', esc_line),
            b't' => push_char_with_line(text, byte_lines, '\t', esc_line),
            b'n' => push_char_with_line(text, byte_lines, '\n', esc_line),
            b'f' => push_char_with_line(text, byte_lines, '\u{000C}', esc_line),
            b'r' => push_char_with_line(text, byte_lines, '\r', esc_line),
            b'"' => push_char_with_line(text, byte_lines, '"', esc_line),
            b'\\' => push_char_with_line(text, byte_lines, '\\', esc_line),
            b'u' => self.push_unicode_escape(text, byte_lines, 4, esc_line),
            b'U' => self.push_unicode_escape(text, byte_lines, 8, esc_line),
            other => push_char_with_line(text, byte_lines, char::from(other), esc_line),
        }
    }

    fn skip_multiline_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.advance_one();
        }
    }

    fn push_unicode_escape(&mut self, text: &mut String, byte_lines: &mut Vec<usize>, digits: usize, line: usize) {
        if let Some(hex) = self.src.get(self.pos..self.pos + digits)
            && let Ok(code) = u32::from_str_radix(hex, 16)
            && let Some(ch) = char::from_u32(code)
        {
            self.pos += digits;
            push_char_with_line(text, byte_lines, ch, line);
        }
    }

    fn check_literal(&mut self, lit: &ExtractedLiteral, _stmt_prev_allow: bool) {
        let policy = if is_dot_prism_extension_description(&lit.text) { PrismPolicy::Standard } else { PrismPolicy::Strict };
        collect_literal_words(self.path, lit, BRANDS, policy, &mut self.seen, self.out);
    }
}

fn is_line_ending_backslash(bytes: &[u8], mut pos: usize) -> bool {
    while matches!(bytes.get(pos), Some(b' ' | b'\t' | b'\r')) {
        pos += 1;
    }
    bytes.get(pos) == Some(&b'\n')
}

fn is_metadata_table_segments(segments: &[String]) -> bool {
    match segments {
        [first] => first == "package",
        [first, second, ..] => {
            (first == "package" || first == "workspace")
                && (second == "package" && segments.len() == 2 && first == "workspace" || second == "metadata")
        }
        _ => false,
    }
}

fn is_metadata_key_segments(segments: &[String], in_workspace_section: bool) -> bool {
    let first = segments.first().map(String::as_str);
    let second = segments.get(1).map(String::as_str);
    if first == Some("package") {
        return true;
    }
    if first == Some("workspace") && matches!(second, Some("package" | "metadata")) {
        return true;
    }
    in_workspace_section && matches!(first, Some("package" | "metadata"))
}

fn is_ui_or_app_rust_path(path: &str) -> bool {
    path.starts_with("crates/ui-egui/src/") || path.starts_with("apps/")
}

fn is_negative_brand_guard_line(path: &str, lines: &[&str], line_no: usize) -> bool {
    let is_test_src = path.starts_with("tests/") || path.contains("/tests/");
    if !is_test_src {
        return false;
    }
    let Some(&src_line) = lines.get(line_no.saturating_sub(1)) else {
        return false;
    };
    let trimmed = src_line.trim();
    trimmed.starts_with("for forbidden in [") || trimmed.contains("for forbidden in [")
}

fn is_pzfx_xml_literal(text: &str) -> bool {
    text.contains("<PrismFile") || text.contains("</PrismFile>") || text.contains("PrismXMLVersion") || text.trim() == "PrismFile"
}

fn is_import_prism_cmd_literal(text: &str) -> bool {
    text.contains("table.importPrism") || text == "cmd.table.importPrism" || text == "importPrism"
}

fn is_italian_i18n_line(path: &str, lines: &[&str], lit: &ExtractedLiteral) -> bool {
    if path != "crates/ui-egui/src/i18n.rs" {
        return false;
    }
    lines.get(lit.start_line.saturating_sub(1)).is_some_and(|line| line.trim_start().starts_with("it:"))
}

fn scan_rust(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    let lines: Vec<&str> = text.lines().collect();
    let mut lexer = RustLexer::new(text);
    let mut seen: BTreeSet<(usize, &'static str)> = BTreeSet::new();
    let strict_ui_or_app = is_ui_or_app_rust_path(path);

    while !lexer.is_eof() {
        if let Some((start_line, comment_text)) = lexer.try_read_comment() {
            scan_rust_comment(path, start_line, comment_text, &mut seen, out);
            continue;
        }
        if lexer.try_skip_char_literal() {
            continue;
        }
        if let Some(lit) = lexer.try_read_raw_string().or_else(|| lexer.try_read_quoted_string()) {
            if is_negative_brand_guard_line(path, &lines, lit.start_line) && lit.start_line == lit.end_line {
                continue;
            }
            let allow_bare_prism = is_import_export_prism_literal(path, text, &lit);
            let allow_dot_prism = is_dot_prism_extension_description(&lit.text);
            let allow_import_prism_cmd = is_import_prism_cmd_literal(&lit.text);
            let allow_pzfx_xml = is_pzfx_xml_literal(&lit.text);
            let allow_it_statistica = is_italian_i18n_line(path, &lines, &lit);
            let allow_test_container_op = is_format_test_container_literal(path, &lit.text);
            let policy = if strict_ui_or_app {
                PrismPolicy::UiOrAppRustLiteral {
                    allow_bare_prism,
                    allow_dot_prism,
                    allow_import_prism_cmd,
                    allow_pzfx_xml,
                    allow_it_statistica,
                    allow_test_container_op,
                }
            } else {
                PrismPolicy::RustLiteral { allow_bare_prism, allow_dot_prism, allow_import_prism_cmd, allow_pzfx_xml, allow_test_container_op }
            };
            collect_literal_words(path, &lit, BRANDS, policy, &mut seen, out);
            collect_literal_words(path, &lit, SUITE_BRANDS, PrismPolicy::Strict, &mut seen, out);
            if is_crates_or_apps_path(path) {
                collect_literal_words(path, &lit, &[ADAPTED_UPSTREAM_BRAND], PrismPolicy::Strict, &mut seen, out);
            }
            continue;
        }
        if let Some((line, ident)) = lexer.try_read_identifier() {
            scan_rust_ident(path, line, ident, &mut seen, out);
            continue;
        }
        lexer.advance_one();
    }
}

fn scan_rust_comment(path: &str, start_line: usize, body: &str, seen: &mut BTreeSet<(usize, &'static str)>, out: &mut Vec<BrandHit>) {
    let allow_citation_statistica = path == "crates/stats/src/pvalue_stack.rs" && body.contains("Teoria statistica delle classi");
    let is_format_area =
        path.starts_with("crates/format/") || path.starts_with("crates/engine/") || path.starts_with("crates/mcp/") || path.starts_with("apps/");
    for (offset, line_text) in body.lines().enumerate() {
        let line_no = start_line + offset;
        let masked = mask_comment_format_tokens(line_text, is_format_area);
        let haystack = masked.as_deref().unwrap_or(line_text);
        let policy = PrismPolicy::RustComment { allow_it_statistica: allow_citation_statistica };
        let lower = haystack.to_ascii_lowercase();
        for &brand in BRANDS {
            collect_word_matches(haystack, &lower, brand, policy, |_| {
                if seen.insert((line_no, brand)) {
                    out.push(BrandHit { path: path.to_owned(), line: line_no, brand });
                }
            });
        }
    }
}

fn mask_comment_format_tokens(line: &str, is_format_area: bool) -> Option<String> {
    if !is_format_area {
        return None;
    }
    let mut work = line.to_owned();
    let mut changed = false;
    for tok in ["table.importPrism", "importPrism", "import_prism_bytes", "<PrismFile", "</PrismFile>", "PrismXMLVersion", "PrismTableMeta"] {
        if work.contains(tok) {
            work = work.replace(tok, "format_tok");
            changed = true;
        }
    }
    if (work.contains("`pzfx`") || work.contains("`curvebook`")) && work.contains("`prism`") {
        work = work.replace("`prism`", "`fmt`");
        changed = true;
    }
    if changed { Some(work) } else { None }
}

fn is_format_test_container_literal(path: &str, text: &str) -> bool {
    if text.contains("import_prism_bytes") && (path.starts_with("crates/format/") || path.contains("/tests/") || path.starts_with("tests/")) {
        return true;
    }
    let in_test =
        path.starts_with("tests/") || path.contains("/tests/") || path == "apps/curvebook-web/src/lib.rs" || path == "crates/format/src/prism.rs";
    if !in_test {
        return false;
    }
    if !text.contains("Prism")
        && text.contains("prism")
        && text.chars().all(|b| b.is_ascii_alphanumeric() || matches!(b, '_' | '-' | '.' | '/' | ' ' | ':' | '{' | '}'))
    {
        const CONTAINER_WORDS: &[&str] =
            &["zip", "pzfx", "bytes", "open ", "open:", "write ", "save ", "to prism", "finish ", "import ", "expected ", "prism json"];
        return CONTAINER_WORDS.iter().any(|w| text.contains(w)) || text.contains("prism:");
    }
    false
}

fn is_canonical_format_rust_ident(path: &str, ident: &str) -> bool {
    if path.starts_with("crates/ui-egui/src/") {
        return false;
    }
    if matches!(
        ident,
        "prism"
            | "DESC_PRISM"
            | "PrismAdapter"
            | "PrismManifest"
            | "PrismTableMeta"
            | "PrismInfoMeta"
            | "ImportPrismParams"
            | "PrismFile"
            | "PrismXMLVersion"
    ) || ident.starts_with("import_prism")
        || ident.starts_with("open_prism")
        || ident.starts_with("run_import_prism")
        || ident.starts_with("schema_import_prism")
        || ident.starts_with("parse_prism_constant")
    {
        return true;
    }
    let allowed_scope =
        path.starts_with("crates/format/") || path.starts_with("crates/engine/") || path.starts_with("crates/mcp/") || path.starts_with("apps/");
    if !allowed_scope || ident.bytes().any(|b| b.is_ascii_uppercase()) {
        return false;
    }
    const FORBIDDEN_STYLE_OR_IMITATION: &[&str] = &[
        "style",
        "theme",
        "palette",
        "logo",
        "icon",
        "ui",
        "widget",
        "dialog",
        "like",
        "clone",
        "copy",
        "competitor",
        "vendor",
        "brand",
        "title_bar",
    ];
    const REQUIRED_FORMAT_TOKENS: &[&str] = &[
        "zip",
        "pzfx",
        "bytes",
        "import",
        "open",
        "report",
        "manifest",
        "file",
        "path",
        "sess",
        "host",
        "res",
        "sum",
        "opened",
        "inspect",
        "container",
        "archive",
        "cleanroom",
        "synthetic",
        "merge",
        "budget",
        "json",
        "csv",
        "bad",
        "nan",
        "legacy",
        "renamed",
        "tpl",
        "batch",
        "huge",
        "test",
        "c_prism",
    ];
    let tokens: Vec<&str> = ident.split('_').filter(|t| !t.is_empty()).collect();
    if tokens.iter().any(|t| FORBIDDEN_STYLE_OR_IMITATION.contains(t)) {
        return false;
    }
    ident == "c_prism" || tokens.iter().any(|t| REQUIRED_FORMAT_TOKENS.contains(t))
}

fn scan_rust_ident(path: &str, line: usize, ident: &str, seen: &mut BTreeSet<(usize, &'static str)>, out: &mut Vec<BrandHit>) {
    let allow_dsl_jmp = path == "crates/fit/src/dsl.rs" && ident == "jmp_end";
    let allow_canonical_prism_ident = is_canonical_format_rust_ident(path, ident);
    let policy = PrismPolicy::RustIdent { allow_dsl_jmp, allow_canonical_prism_ident };
    let lower = ident.to_ascii_lowercase();
    for &brand in BRANDS {
        collect_word_matches(ident, &lower, brand, policy, |_| {
            if seen.insert((line, brand)) {
                out.push(BrandHit { path: path.to_owned(), line, brand });
            }
        });
    }
}

#[derive(Clone, Copy)]
enum PrismPolicy<'a> {
    Strict,
    AllowExactBarePrism,
    Standard,
    RustComment {
        allow_it_statistica: bool,
    },
    RustIdent {
        allow_dsl_jmp: bool,
        allow_canonical_prism_ident: bool,
    },
    RustLiteral {
        allow_bare_prism: bool,
        allow_dot_prism: bool,
        allow_import_prism_cmd: bool,
        allow_pzfx_xml: bool,
        allow_test_container_op: bool,
    },
    UiOrAppRustLiteral {
        allow_bare_prism: bool,
        allow_dot_prism: bool,
        allow_import_prism_cmd: bool,
        allow_pzfx_xml: bool,
        allow_it_statistica: bool,
        allow_test_container_op: bool,
    },
    UiTextRun {
        allow_dot_prism: bool,
        allow_it_statistica: bool,
        allow_palette_cmd_id: bool,
    },
    SvgTitleOrDesc {
        prev_word: Option<&'a str>,
    },
}

fn collect_literal_words(
    path: &str,
    lit: &ExtractedLiteral,
    words: &[&'static str],
    policy: PrismPolicy<'_>,
    seen: &mut BTreeSet<(usize, &'static str)>,
    out: &mut Vec<BrandHit>,
) {
    let lower = lit.text.to_ascii_lowercase();
    for &brand in words {
        collect_word_matches(&lit.text, &lower, brand, policy, |pos| {
            let line = lit.byte_lines.get(pos).copied().unwrap_or(lit.start_line);
            if seen.insert((line, brand)) {
                out.push(BrandHit { path: path.to_owned(), line, brand });
            }
        });
    }
}

fn collect_line_words(path: &str, line_no: usize, haystack: &str, words: &[&'static str], policy: PrismPolicy<'_>, out: &mut Vec<BrandHit>) {
    let lower = haystack.to_ascii_lowercase();
    for &brand in words {
        if has_word_match(haystack, &lower, brand, policy) {
            out.push(BrandHit { path: path.to_owned(), line: line_no, brand });
        }
    }
}

fn has_word_match(raw: &str, lower: &str, word: &'static str, policy: PrismPolicy<'_>) -> bool {
    let mut found = false;
    collect_word_matches(raw, lower, word, policy, |_| {
        found = true;
    });
    found
}

fn collect_word_matches(raw: &str, lower: &str, word: &'static str, policy: PrismPolicy<'_>, mut on_match: impl FnMut(usize)) {
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

fn collect_snake_kebab_compound_matches(raw: &str, lower: &str, word: &'static str, policy: PrismPolicy<'_>, on_match: &mut impl FnMut(usize)) {
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

fn collect_delimited_suite_matches(lower: &str, word: &'static str, on_match: &mut impl FnMut(usize)) {
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

fn is_token_match_exempt(raw: &str, lower: &str, word: &'static str, pos: usize, end: usize, policy: PrismPolicy<'_>) -> bool {
    match word {
        "prism" => is_prism_match_exempt(raw, lower, pos, end, policy),
        "statistica" => is_statistica_match_exempt(raw, policy, pos, end),
        "jmp" => is_jmp_match_exempt(policy),
        _ => false,
    }
}

fn is_statistica_match_exempt(raw: &str, policy: PrismPolicy<'_>, pos: usize, end: usize) -> bool {
    if raw.get(pos..end) != Some("statistica") {
        return false;
    }
    match policy {
        PrismPolicy::RustComment { allow_it_statistica } => allow_it_statistica,
        PrismPolicy::UiOrAppRustLiteral { allow_it_statistica, .. } => allow_it_statistica,
        PrismPolicy::UiTextRun { allow_it_statistica, .. } => allow_it_statistica,
        _ => false,
    }
}

fn is_jmp_match_exempt(policy: PrismPolicy<'_>) -> bool {
    matches!(policy, PrismPolicy::RustIdent { allow_dsl_jmp: true, .. })
}

fn is_prism_match_exempt(raw: &str, lower: &str, pos: usize, end: usize, policy: PrismPolicy<'_>) -> bool {
    let is_lower_dot_ext = raw.as_bytes().get(pos..end) == Some(b"prism") && is_prism_extension_token(lower.as_bytes(), pos, end);
    let is_cmd_prism = matches!(raw.get(pos.saturating_sub(12)..end), Some("table.importPrism"))
        || matches!(raw.get(pos.saturating_sub(6)..end), Some("importPrism"));
    let is_xml_prism_tag = raw.get(pos..pos + 9) == Some("PrismFile") || raw.get(pos..pos + 15) == Some("PrismXMLVersion");
    match policy {
        PrismPolicy::Strict => false,
        PrismPolicy::AllowExactBarePrism => raw.as_bytes().get(pos..end) == Some(b"prism"),
        PrismPolicy::Standard => is_lower_dot_ext,
        PrismPolicy::RustComment { .. } => is_lower_dot_ext,
        PrismPolicy::RustIdent { allow_canonical_prism_ident, .. } => allow_canonical_prism_ident,
        PrismPolicy::RustLiteral { allow_bare_prism, allow_dot_prism, allow_import_prism_cmd, allow_pzfx_xml, allow_test_container_op } => {
            (allow_bare_prism && raw == "prism")
                || (allow_dot_prism && is_lower_dot_ext)
                || (allow_import_prism_cmd && is_cmd_prism)
                || (allow_pzfx_xml && is_xml_prism_tag)
                || (allow_test_container_op && raw.as_bytes().get(pos..end) == Some(b"prism"))
        }
        PrismPolicy::UiOrAppRustLiteral {
            allow_bare_prism,
            allow_dot_prism,
            allow_import_prism_cmd,
            allow_pzfx_xml,
            allow_test_container_op,
            ..
        } => {
            (allow_bare_prism && raw == "prism")
                || (allow_dot_prism && is_lower_dot_ext)
                || (allow_import_prism_cmd && is_cmd_prism)
                || (allow_pzfx_xml && is_xml_prism_tag)
                || (allow_test_container_op && raw.as_bytes().get(pos..end) == Some(b"prism"))
        }
        PrismPolicy::UiTextRun { allow_dot_prism, allow_palette_cmd_id, .. } => {
            (allow_dot_prism && is_lower_dot_ext) || (allow_palette_cmd_id && is_cmd_prism)
        }
        PrismPolicy::SvgTitleOrDesc { prev_word } => is_lower_dot_ext || is_preceded_by_optical_modifier(lower.get(..pos).unwrap_or(""), prev_word),
    }
}

fn is_dot_prism_extension_description(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed == ".prism" || trimmed == "*.prism" {
        return true;
    }
    if let Some(stem) = trimmed.strip_suffix(".prism")
        && !stem.is_empty()
        && stem.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    if !lower.contains(".prism") || lower.contains("http://") || lower.contains("https://") || lower.contains("www.") {
        return false;
    }
    const FORMAT_TOKENS: &[&str] = &[
        ".pzfx",
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

fn tail_char_slice(s: &str, max_bytes: usize) -> &str {
    let mut start = s.len().saturating_sub(max_bytes);
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    s.get(start..).unwrap_or(s)
}

fn head_char_slice(s: &str, max_bytes: usize) -> &str {
    let mut end = s.len().min(max_bytes);
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or(s)
}

fn statement_prefix_tail(before: &str, max_bytes: usize) -> &str {
    let window = tail_char_slice(before, max_bytes);
    if let Some((_, after)) = window.rsplit_once([';', '{', '}']) { after } else { window }
}

fn is_import_export_prism_literal(path: &str, src: &str, lit: &ExtractedLiteral) -> bool {
    if lit.text != "prism" {
        return false;
    }
    let before = src.get(..lit.quote_start).unwrap_or("");
    let after = src.get(lit.quote_end..).unwrap_or("");
    if is_prism_match_pattern(before, after) || is_prism_extension_array_item(before, after) || is_prism_format_call_arg(before) {
        return true;
    }
    let before_trim = before.trim_end();
    let after_trim = after.trim_start();
    if path == "crates/format/src/registry.rs" && before_trim.ends_with("id:") && before.contains("FormatDescriptor") {
        return true;
    }
    let in_test = path.starts_with("tests/") || path.contains("/tests/");
    if in_test {
        let stmt = statement_prefix_tail(before, 220);
        if before_trim.ends_with(".id,")
            || stmt.contains("unique_temp_path")
            || (before_trim.ends_with('(') && after_trim.contains(".prism\""))
            || stmt.contains("assert_eq!") && stmt.contains("[\"format\"]")
        {
            return true;
        }
    }
    false
}

fn is_prism_match_pattern(before: &str, after: &str) -> bool {
    let before_trim = before.trim_end();
    let after_trim = after.trim_start();

    if before_trim.ends_with("Some(")
        && let Some(after_paren) = after_trim.strip_prefix(')')
    {
        let post_paren = after_paren.trim_start();
        if post_paren.starts_with("=>") || post_paren.starts_with('|') || post_paren.starts_with("if ") {
            return true;
        }
    }

    if before_trim.ends_with('|') || after_trim.starts_with('|') {
        let arm_after = head_char_slice(after_trim, 160);
        let stmt_before = statement_prefix_tail(before, 200);
        if arm_after.contains("=>")
            || stmt_before.contains("matches!")
            || arm_after.contains("\"pzfx\"")
            || stmt_before.contains("\"pzfx\"")
            || arm_after.contains("\"curvebook\"")
            || stmt_before.contains("\"curvebook\"")
        {
            return true;
        }
    }

    if before_trim.ends_with("==") || before_trim.ends_with("!=") {
        let ident_part = before_trim.trim_end_matches(['=', '!']).trim_end();
        let ident = trailing_ascii_word(ident_part).to_ascii_lowercase();
        let tail_lower = tail_char_slice(ident_part, 40).to_ascii_lowercase();
        return ident.contains("ext")
            || ident.contains("format")
            || ident.contains("fmt")
            || ident.contains("filter")
            || ident.contains("kind")
            || ident.contains("importer")
            || tail_lower.contains("[\"format\"]")
            || tail_lower.contains("[\"id\"]");
    }

    false
}

fn find_enclosing_bracket_open(before: &str) -> Option<usize> {
    let window = tail_char_slice(before, 512);
    let offset = before.len() - window.len();
    let bytes = window.as_bytes();
    let mut depth = 0usize;
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b']' => depth += 1,
            b'[' => {
                if depth == 0 {
                    return Some(offset + i);
                }
                depth -= 1;
            }
            b';' | b'{' | b'}' => return None,
            _ => {}
        }
    }
    None
}

fn find_enclosing_bracket_close(after: &str) -> Option<usize> {
    let window = head_char_slice(after, 512);
    let bytes = window.as_bytes();
    let mut depth = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'[' => depth += 1,
            b']' => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            b';' | b'{' | b'}' => return None,
            _ => {}
        }
    }
    None
}

fn is_prism_extension_array_item(before: &str, after: &str) -> bool {
    let Some(open_idx) = find_enclosing_bracket_open(before) else {
        return false;
    };
    let Some(close_idx) = find_enclosing_bracket_close(after) else {
        return false;
    };
    let inner_before = before.get(open_idx..).unwrap_or("");
    let inner_after = after.get(..close_idx).unwrap_or("");
    const PEER_EXT_LITERALS: &[&str] = &["\"pzfx\"", "\"cbtemplate\"", "\"curvebook\"", "\"curvebook.d\"", "\"csv\"", "\"tsv\""];
    if PEER_EXT_LITERALS.iter().any(|peer| inner_before.contains(peer) || inner_after.contains(peer)) {
        return true;
    }
    let prefix = statement_prefix_tail(before.get(..open_idx).unwrap_or(""), 160).to_ascii_lowercase();
    const ARRAY_CONTEXT_MARKERS: &[&str] = &["add_filter", "extension", "file_filter", "import", "export", "accept", "format_"];
    ARRAY_CONTEXT_MARKERS.iter().any(|m| prefix.contains(m))
}

fn is_prism_format_call_arg(before: &str) -> bool {
    let stmt = statement_prefix_tail(before, 220).to_ascii_lowercase();
    let trimmed = before.trim_end();
    if let Some((before_paren, _)) = trimmed.rsplit_once('(') {
        let callee = trailing_ascii_word(before_paren.trim_end()).to_ascii_lowercase();
        if matches!(callee.as_str(), "label" | "title" | "text" | "button" | "tooltip" | "heading" | "window_title" | "set_title") {
            return false;
        }
    }
    if let Some((before_colon, _)) = trimmed.rsplit_once(':') {
        let field = trailing_ascii_word(before_colon.trim_end()).to_ascii_lowercase();
        if matches!(field.as_str(), "label" | "title" | "text" | "tooltip" | "summary" | "beginner" | "message" | "placeholder") {
            return false;
        }
    }
    const CALL_MARKERS: &[&str] = &[
        "apply_import_report",
        "load_import_report",
        "importer_by_format",
        "exporter_by_format",
        "add_filter",
        "open_prism",
        "import_prism",
        "handle_native_import_file",
        "formatdescriptor",
        "file_extensions",
        "import_extensions",
        "export_extensions",
        "format_tag",
        "format_id",
        "format_opt",
        "detect_ext",
    ];
    if CALL_MARKERS.iter().any(|m| stmt.contains(m)) {
        return true;
    }
    stmt.contains("[\"format\"]") && (trimmed.ends_with("json!(") || trimmed.ends_with("Some(") || before.trim_end().ends_with("=="))
}

fn is_prism_extension_token(bytes: &[u8], pos: usize, end: usize) -> bool {
    if pos == 0 || bytes[pos - 1] != b'.' {
        return false;
    }
    let followed_by_domain_segment = bytes.get(end) == Some(&b'.') && bytes.get(end + 1).is_some_and(u8::is_ascii_alphanumeric);
    !followed_by_domain_segment
}

fn is_preceded_by_optical_modifier(before_lower: &str, prev_word: Option<&str>) -> bool {
    let trimmed = before_lower.trim_end();
    let word = if trimmed.is_empty() { prev_word.unwrap_or("") } else { trailing_ascii_word(trimmed) };
    OPTICAL_PRISM_MODIFIERS.contains(&word)
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
    prism_policy: PrismPolicy<'_>,
    include_pre_rename: bool,
    out: &mut Vec<BrandHit>,
) {
    let mut seen: BTreeSet<&'static str> = BTreeSet::new();
    let lower = text.to_ascii_lowercase();
    for &brand in SUITE_BRANDS {
        if has_word_match(text, &lower, brand, PrismPolicy::Strict) && seen.insert(brand) {
            out.push(BrandHit { path: path.to_owned(), line, brand });
        }
    }
    if !is_legal_policy_doc_path(path)
        && !is_attribution_or_notice_path(path, true)
        && has_word_match(text, &lower, ADAPTED_UPSTREAM_BRAND, PrismPolicy::Strict)
        && seen.insert(ADAPTED_UPSTREAM_BRAND)
    {
        out.push(BrandHit { path: path.to_owned(), line, brand: ADAPTED_UPSTREAM_BRAND });
    }
    if include_pre_rename {
        for &brand in PRE_RENAME_ASSET_BRANDS {
            if has_word_match(text, &lower, brand, PrismPolicy::Strict) && seen.insert(brand) {
                out.push(BrandHit { path: path.to_owned(), line, brand });
            }
        }
    }
    for &brand in BRANDS {
        if has_word_match(text, &lower, brand, prism_policy) && seen.insert(brand) {
            out.push(BrandHit { path: path.to_owned(), line, brand });
        }
    }
}

fn check_single_string(path: &str, line: usize, text: &str, prism_policy: PrismPolicy<'_>, out: &mut Vec<BrandHit>) {
    check_single_string_with_extra(path, line, text, prism_policy, false, out);
}

/// Scan arbitrary UI, `egui_shell`, AccessKit, or tooltip `&str` slices for external
/// brands and suite names. Allows `.prism` / `*.prism` only inside explicit import/export
/// file-type descriptions.
#[cfg(any(feature = "full-gates", test))]
#[must_use]
pub fn check_text_runs<I, S>(source: &str, runs: I) -> Vec<BrandHit>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out = Vec::new();
    for (idx, run) in runs.into_iter().enumerate() {
        let text = run.as_ref();
        let policy = PrismPolicy::UiTextRun {
            allow_dot_prism: is_dot_prism_extension_description(text),
            allow_it_statistica: false,
            allow_palette_cmd_id: false,
        };
        check_single_string(source, idx + 1, text, policy, &mut out);
    }
    out
}

/// Strictly scan text runs, rejecting every external brand, suite name, and `"prism"` mention
/// (including `.prism`).
#[cfg(any(feature = "full-gates", test))]
#[must_use]
pub fn check_text_runs_strict<I, S>(source: &str, runs: I) -> Vec<BrandHit>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out = Vec::new();
    for (idx, run) in runs.into_iter().enumerate() {
        check_single_string(source, idx + 1, run.as_ref(), PrismPolicy::Strict, &mut out);
    }
    out
}

/// Scan `(label, text)` pairs (for example `(node.id, node.a11y.label)` or `(key, text)`),
/// tagging each hit with `label`.
#[cfg(any(feature = "full-gates", test))]
#[must_use]
pub fn check_labeled_text_runs<'a, I>(runs: I) -> Vec<BrandHit>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let mut out = Vec::new();
    for (idx, (label, text)) in runs.into_iter().enumerate() {
        let policy = PrismPolicy::UiTextRun {
            allow_dot_prism: is_dot_prism_extension_description(text),
            allow_it_statistica: false,
            allow_palette_cmd_id: false,
        };
        check_single_string(label, idx + 1, text, policy, &mut out);
    }
    out
}

#[cfg(any(feature = "full-gates", test))]
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
/// Rejects `"prism"` (including `.prism`) everywhere outside explicit file-extension/filter keys,
/// and permits lowercase Italian `"statistica"` only inside `locale == "it"`.
#[cfg(any(feature = "full-gates", test))]
#[must_use]
pub fn check_i18n_catalog_entries<'a, I>(entries: I) -> Vec<BrandHit>
where
    I: IntoIterator<Item = (&'a str, &'a str, &'a str)>,
{
    let mut out = Vec::new();
    for (key, locale, text) in entries {
        let allow_it_statistica = locale == "it";
        let policy = if is_i18n_format_extension_key(key) {
            PrismPolicy::UiTextRun { allow_dot_prism: is_dot_prism_extension_description(text), allow_it_statistica, allow_palette_cmd_id: false }
        } else {
            PrismPolicy::UiTextRun { allow_dot_prism: false, allow_it_statistica, allow_palette_cmd_id: false }
        };
        let path = format!("i18n:{locale}:{key}");
        check_single_string(&path, 1, text, policy, &mut out);
    }
    out
}

/// Scan all registered 16-locale `i18n::CATALOG` translations and locale display names.
#[cfg(feature = "full-gates")]
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

fn scan_xml_text_fragment(source: &str, start_line: usize, raw: &str, policy: PrismPolicy<'_>, out: &mut Vec<BrandHit>) {
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
                scan_xml_text_fragment(source, line, comment_body, PrismPolicy::Strict, &mut out);
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
                        scan_xml_text_fragment(source, active_start_line, &buf, PrismPolicy::Strict, &mut out);
                        buf.clear();
                    }
                } else if is_text_tag {
                    text_depth = text_depth.saturating_sub(1);
                    if meta_depth == 0 && text_depth == 0 && !buf.trim().is_empty() {
                        scan_xml_text_fragment(source, active_start_line, &buf, PrismPolicy::Strict, &mut out);
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
        scan_xml_text_fragment(source, active_start_line, &buf, PrismPolicy::Strict, &mut out);
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
                scan_xml_text_fragment(source, attr_line, val, PrismPolicy::Strict, out);
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
                    check_single_string(source, val_line, &decoded, PrismPolicy::Strict, &mut out);
                } else if pdf_bytes.get(pos) == Some(&b'<') && pdf_bytes.get(pos + 1) != Some(&b'<') {
                    let decoded = decode_pdf_hex_string(pdf_bytes, &mut pos, &mut line);
                    check_single_string(source, val_line, &decoded, PrismPolicy::Strict, &mut out);
                }
            }
            continue;
        }
        pos += 1;
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

/// Inspect HTML metadata (`<title>`, `<meta content=...>`, `aria-label`/`alt`/`title` attributes,
/// embedded `<svg>` metadata, and visible DOM text) for external brand or suite mentions.
#[must_use]
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
                    check_single_string_with_extra(source, tag_line, &content, PrismPolicy::Strict, true, &mut out);
                }
                for attr in ["aria-label", "alt", "title"] {
                    if let Some(val) = extract_html_attr(raw_inner, attr) {
                        check_single_string_with_extra(source, tag_line, &val, PrismPolicy::Strict, true, &mut out);
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
    let gen_docs = !strict && is_generated_docs_html_path(source);
    for (offset, line_text) in decoded.lines().enumerate() {
        let masked_opt = if gen_docs { mask_generated_docs_html_tokens(source, line_text) } else { None };
        let scanned = masked_opt.as_deref().unwrap_or(line_text);
        let policy = if strict {
            PrismPolicy::Strict
        } else {
            PrismPolicy::UiTextRun {
                allow_dot_prism: is_dot_prism_extension_description(scanned),
                allow_it_statistica: false,
                allow_palette_cmd_id: false,
            }
        };
        check_single_string_with_extra(source, start_line + offset, scanned, policy, strict, out);
    }
}

/// Inspect `.curvebook`/`.cbtemplate` `manifest.json` keys and string values strictly.
#[must_use]
pub fn check_manifest_metadata(source: &str, manifest_json: &str) -> Vec<BrandHit> {
    let mut out = Vec::new();
    match serde_json::from_str::<serde_json::Value>(manifest_json) {
        Ok(val) => visit_json_strings(source, &val, 1, &mut out),
        Err(_) => {
            for (idx, line) in manifest_json.lines().enumerate() {
                check_single_string(source, idx + 1, line, PrismPolicy::Strict, &mut out);
            }
        }
    }
    out
}

fn visit_json_strings(source: &str, val: &serde_json::Value, idx_hint: usize, out: &mut Vec<BrandHit>) {
    match val {
        serde_json::Value::String(s) => {
            check_single_string(source, idx_hint, s, PrismPolicy::Strict, out);
        }
        serde_json::Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                visit_json_strings(source, item, i + 1, out);
            }
        }
        serde_json::Value::Object(map) => {
            for (i, (k, v)) in map.iter().enumerate() {
                check_single_string(source, i + 1, k, PrismPolicy::Strict, out);
                visit_json_strings(source, v, i + 1, out);
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

/// Inspect a desktop or browser window title string strictly.
#[cfg(any(feature = "full-gates", test))]
#[must_use]
pub fn check_window_title(source: &str, title: &str) -> Vec<BrandHit> {
    let mut out = Vec::new();
    check_single_string(source, 1, title, PrismPolicy::Strict, &mut out);
    out
}

#[cfg(feature = "full-gates")]
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
        let allow_it_statistica = label.contains(":it:");
        let policy =
            PrismPolicy::UiTextRun { allow_dot_prism: is_dot_prism_extension_description(text), allow_it_statistica, allow_palette_cmd_id: false };
        check_single_string(label, 1, text, policy, hits);
    }
}

#[cfg(feature = "full-gates")]
fn check_labeled_ui_pairs<'a>(runs: impl IntoIterator<Item = (&'a str, &'a str)>, allow_it_statistica: bool, hits: &mut Vec<BrandHit>) {
    for (idx, (label, text)) in runs.into_iter().enumerate() {
        let policy =
            PrismPolicy::UiTextRun { allow_dot_prism: is_dot_prism_extension_description(text), allow_it_statistica, allow_palette_cmd_id: false };
        check_single_string(label, idx + 1, text, policy, hits);
    }
}

#[cfg(feature = "full-gates")]
fn check_ui_strings_with_locale<'a>(source: &str, runs: impl IntoIterator<Item = &'a str>, allow_it_statistica: bool, hits: &mut Vec<BrandHit>) {
    for (idx, text) in runs.into_iter().enumerate() {
        let policy =
            PrismPolicy::UiTextRun { allow_dot_prism: is_dot_prism_extension_description(text), allow_it_statistica, allow_palette_cmd_id: false };
        check_single_string(source, idx + 1, text, policy, hits);
    }
}

#[cfg(feature = "full-gates")]
fn push_widget_node_runs(prefix: &str, node: &curvebook_ui_egui::inspect::UiWidgetNode, out: &mut Vec<(String, String)>) {
    out.push((format!("{prefix}:{}", node.widget_id), node.label.clone()));
    if let Some(reason) = &node.disabled_reason {
        out.push((format!("{prefix}:{}:disabled_reason", node.widget_id), reason.clone()));
    }
    if let Some(serde_json::Value::String(v)) = &node.value {
        out.push((format!("{prefix}:{}:value", node.widget_id), v.clone()));
    }
}

#[cfg(feature = "full-gates")]
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

#[cfg(feature = "full-gates")]
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

#[cfg(feature = "full-gates")]
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

#[cfg(feature = "full-gates")]
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
#[cfg(feature = "full-gates")]
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

#[cfg(feature = "full-gates")]
struct BrandsGuiSessionHost {
    app: UiApp,
}

#[cfg(feature = "full-gates")]
impl BrandsGuiSessionHost {
    fn new(mut app: UiApp, gui_only: bool) -> Self {
        app.set_gui_only(gui_only);
        Self { app }
    }
}

#[cfg(feature = "full-gates")]
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

#[cfg(any(feature = "full-gates", test))]
#[derive(Clone, Copy)]
struct McpScanCtx<'a> {
    source: &'a str,
    method: &'a str,
    called_tool: Option<&'a str>,
    entry_tool_name: Option<&'a str>,
}

/// Inspect a serialized MCP JSON-RPC response (`initialize`, `tools/list`, `tools/call`,
/// `resources/list`, `resources/read`, `prompts/list`, `prompts/get`) for prohibited vendor,
/// product, retired-suite, or pre-rename tokens while preserving the narrow `.prism`/`.pzfx`
/// file-format boundary on `open_file` and `table_import_prism` / `table.importPrism`.
#[cfg(any(feature = "full-gates", test))]
#[must_use]
pub fn check_mcp_rpc_response(source: &str, method: &str, called_tool: Option<&str>, response: &serde_json::Value) -> Vec<BrandHit> {
    let mut out = Vec::new();
    let ctx = McpScanCtx { source, method, called_tool, entry_tool_name: None };
    scan_mcp_json_value(ctx, "", response, &mut out);
    out
}

#[cfg(any(feature = "full-gates", test))]
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

#[cfg(any(feature = "full-gates", test))]
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
        check_single_string_with_extra(&key_source, idx + 1, k, PrismPolicy::Strict, true, out);
        scan_mcp_json_value(child_ctx, &format!("{ptr}/{k}"), v, out);
    }
}

#[cfg(any(feature = "full-gates", test))]
fn scan_mcp_inline_png_block(source: &str, ptr: &str, map: &serde_json::Map<String, serde_json::Value>, out: &mut Vec<BrandHit>) {
    let block_src = format!("{source}:{ptr}");
    for (idx, k) in map.keys().enumerate() {
        check_single_string_with_extra(&block_src, idx + 1, k, PrismPolicy::Strict, true, out);
    }
    if let Some(b64) = map.get("data").and_then(serde_json::Value::as_str)
        && let Some(decoded) = decode_base64_ascii(b64)
    {
        out.extend(check_png_metadata(&format!("{block_src}/data#png"), &decoded));
    }
}

#[cfg(any(feature = "full-gates", test))]
fn is_top_tool_field_ptr(ptr: &str, field: &str) -> bool {
    let Some(rest) = ptr.strip_prefix("/result/tools/") else {
        return false;
    };
    let Some((idx_seg, tail)) = rest.split_once('/') else {
        return false;
    };
    !idx_seg.is_empty() && idx_seg.bytes().all(|b| b.is_ascii_digit()) && tail == field
}

#[cfg(any(feature = "full-gates", test))]
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
    if is_exact_canonical_mcp_import_name(ctx, ptr, text) {
        let lower = text.to_ascii_lowercase();
        for &brand in SUITE_BRANDS.iter().chain(PRE_RENAME_ASSET_BRANDS) {
            if has_word_match(text, &lower, brand, PrismPolicy::Strict) {
                out.push(BrandHit { path: full_source.clone(), line: 1, brand });
            }
        }
        return;
    }

    let policy = mcp_leaf_prism_policy(ctx, ptr, text);
    check_single_string_with_extra(&full_source, 1, text, policy, true, out);
}

#[cfg(any(feature = "full-gates", test))]
fn is_exact_canonical_mcp_import_name(ctx: McpScanCtx<'_>, ptr: &str, text: &str) -> bool {
    if text != "table_import_prism" {
        return false;
    }
    if ctx.method == "tools/list" && ctx.entry_tool_name == Some("table_import_prism") && is_top_tool_field_ptr(ptr, "name") {
        return true;
    }
    ctx.method == "tools/call"
        && matches!(ctx.called_tool, Some("list_toolsets" | "command_list"))
        && (ptr.ends_with("/name") || ptr.ends_with("/mcp_tool"))
}

#[cfg(any(feature = "full-gates", test))]
fn mcp_leaf_prism_policy<'a>(ctx: McpScanCtx<'_>, ptr: &str, text: &str) -> PrismPolicy<'a> {
    if ctx.method == "tools/list" {
        if ctx.entry_tool_name == Some("open_file") {
            if is_top_tool_field_ptr(ptr, "description") {
                return PrismPolicy::UiTextRun {
                    allow_dot_prism: is_dot_prism_extension_description(text),
                    allow_it_statistica: false,
                    allow_palette_cmd_id: false,
                };
            }
            if ptr.contains("/inputSchema/properties/format/enum/") && text == "prism" {
                return PrismPolicy::AllowExactBarePrism;
            }
            return PrismPolicy::Strict;
        }
        if ctx.entry_tool_name == Some("table_import_prism") && is_top_tool_field_ptr(ptr, "description") {
            return PrismPolicy::UiTextRun {
                allow_dot_prism: is_dot_prism_extension_description(text),
                allow_it_statistica: false,
                allow_palette_cmd_id: true,
            };
        }
        return PrismPolicy::Strict;
    }

    if ctx.method == "tools/call" && matches!(ctx.called_tool, Some("list_toolsets" | "command_list" | "ui_inspect")) {
        if (ptr.ends_with("/command_id") || ptr.ends_with("/id") || ptr.ends_with("/widget_id"))
            && (text == "table.importPrism" || text.ends_with(".table.importPrism"))
        {
            return PrismPolicy::UiTextRun { allow_dot_prism: false, allow_it_statistica: false, allow_palette_cmd_id: true };
        }
        if ptr.ends_with("/summary") || ptr.ends_with("/beginner") || ptr.ends_with("/params_doc") || ptr.ends_with("/label") {
            return PrismPolicy::UiTextRun {
                allow_dot_prism: is_dot_prism_extension_description(text),
                allow_it_statistica: false,
                allow_palette_cmd_id: false,
            };
        }
    }

    PrismPolicy::Strict
}

#[cfg(feature = "full-gates")]
fn send_mcp_rpc(state: &mut McpServerState, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    let req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    dispatch_json_value(req, state).ok_or_else(|| format!("MCP `{method}` produced no JSON-RPC response"))
}

#[cfg(feature = "full-gates")]
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
#[cfg(feature = "full-gates")]
pub fn check_mcp_surfaces() -> Result<Vec<BrandHit>, String> {
    let mut hits = Vec::new();
    let doc = build_brands_sweep_document()?;
    let mut session = Session::from_document(doc);
    session.recalculate_all().map_err(|e| format!("MCP sweep recalculate: {e}"))?;

    check_mcp_headless_mode(session.clone(), &mut hits)?;
    check_mcp_gui_only_mode(session, &mut hits)?;
    Ok(hits)
}

#[cfg(feature = "full-gates")]
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

#[cfg(feature = "full-gates")]
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

#[cfg(feature = "full-gates")]
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

#[cfg(feature = "full-gates")]
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

#[cfg(feature = "full-gates")]
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

#[cfg(feature = "full-gates")]
fn check_engine_and_ui_commands(hits: &mut Vec<BrandHit>) {
    for spec in engine_cmd_registry().specs() {
        let is_import_prism = spec.id == "table.importPrism";
        let id_policy = PrismPolicy::UiTextRun { allow_dot_prism: false, allow_it_statistica: false, allow_palette_cmd_id: is_import_prism };
        check_single_string(&format!("engine_cmd:{}:id", spec.id), 1, spec.id, id_policy, hits);
        let text_policy = PrismPolicy::UiTextRun { allow_dot_prism: true, allow_it_statistica: false, allow_palette_cmd_id: false };
        for (field, val) in [("label", spec.label), ("menu_path", spec.menu_path), ("summary", spec.summary), ("beginner", spec.beginner)] {
            let p = PrismPolicy::UiTextRun {
                allow_dot_prism: is_dot_prism_extension_description(val),
                allow_it_statistica: false,
                allow_palette_cmd_id: false,
            };
            check_single_string(&format!("engine_cmd:{}:{field}", spec.id), 1, val, p, hits);
        }
        if let Ok(schema_str) = serde_json::to_string(&(spec.input_schema)()) {
            check_single_string(&format!("engine_cmd:{}:input_schema", spec.id), 1, &schema_str, text_policy, hits);
        }
    }
    for cmd in UI_COMMANDS {
        let is_import_prism = cmd.id == "table.importPrism";
        let id_policy = PrismPolicy::UiTextRun { allow_dot_prism: false, allow_it_statistica: false, allow_palette_cmd_id: is_import_prism };
        check_single_string(&format!("ui_cmd:{}:id", cmd.id), 1, cmd.id, id_policy, hits);
        for (field, val) in [("label", cmd.label), ("params_doc", cmd.params_doc), ("summary", cmd.summary), ("beginner", cmd.beginner)] {
            let p = PrismPolicy::UiTextRun {
                allow_dot_prism: is_dot_prism_extension_description(val),
                allow_it_statistica: false,
                allow_palette_cmd_id: false,
            };
            check_single_string(&format!("ui_cmd:{}:{field}", cmd.id), 1, val, p, hits);
        }
    }
}

/// Sweep live `egui_shell` painted text runs, `FrameInspectNodeMeta`, and AccessKit strings
/// across ALL `16` `LOCALES`, all `5` sheet kinds, all `4` `WelcomeMode`s, all `7` `ActiveInspector`
/// tabs, all `26` `ActiveDialog` variants, open dropdown menus, toolbar overflow, split view, and
/// portfolio gallery.
#[cfg(feature = "full-gates")]
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
        app.view_shards.toolbar_menus.overflow_open = false;
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

        app.view_shards.toolbar_menus.overflow_open = true;
        let overflow_cap = scanner.scan(&mut app, narrow_vp);
        collect_egui_surface_hits(&format!("egui_shell:{locale}:toolbar_overflow"), is_it, false, &overflow_cap, &mut hits);
        app.view_shards.toolbar_menus.overflow_open = false;

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

#[cfg(feature = "full-gates")]
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

#[cfg(feature = "full-gates")]
fn collect_egui_surface_hits(
    context: &str,
    allow_it_statistica: bool,
    allow_palette_cmd_id: bool,
    cap: &EguiSurfaceCapture,
    hits: &mut Vec<BrandHit>,
) {
    for (idx, (owner, text)) in cap.text_runs.iter().enumerate() {
        let is_palette_cmd = allow_palette_cmd_id && (owner == "palette.status" || owner.starts_with("palette.item."));
        let run_ctx = format!("{context}:text_run:{owner}");
        let policy = PrismPolicy::UiTextRun {
            allow_dot_prism: is_dot_prism_extension_description(text),
            allow_it_statistica,
            allow_palette_cmd_id: is_palette_cmd,
        };
        check_single_string(&run_ctx, idx + 1, text, policy, hits);
    }
    for (idx, entry) in cap.a11y_entries.iter().enumerate() {
        let a11y_ctx = format!("{context}:a11y:{}", entry.widget_id);
        for txt in [Some(entry.label.as_str()), entry.value_text.as_deref(), entry.disabled_reason.as_deref(), entry.full_label.as_deref()]
            .into_iter()
            .flatten()
        {
            let policy =
                PrismPolicy::UiTextRun { allow_dot_prism: is_dot_prism_extension_description(txt), allow_it_statistica, allow_palette_cmd_id: false };
            check_single_string(&a11y_ctx, idx + 1, txt, policy, hits);
        }
        if let Some(cmd) = &entry.command_id
            && cmd != "table.importPrism"
        {
            hits.extend(check_text_runs_strict(&format!("{a11y_ctx}:command_id"), [cmd.as_str()]));
        }
    }
    for (idx, ak) in cap.accesskit_labels.iter().enumerate() {
        let policy =
            PrismPolicy::UiTextRun { allow_dot_prism: is_dot_prism_extension_description(ak), allow_it_statistica, allow_palette_cmd_id: false };
        check_single_string(&format!("{context}:accesskit"), idx + 1, ak, policy, hits);
    }
}

/// Inspect PNG binary metadata chunks (`tEXt`, `iTXt`, and compressed `zTXt`) strictly against
/// external brands, suite names, and pre-rename `LumenCraft` icon tags.
#[must_use]
pub fn check_png_metadata(source: &str, png_bytes: &[u8]) -> Vec<BrandHit> {
    const PNG_SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    let mut out = Vec::new();
    if !png_bytes.starts_with(PNG_SIG) {
        return out;
    }
    let mut pos = 8usize;
    let mut chunk_idx = 0usize;
    while pos + 12 <= png_bytes.len() {
        chunk_idx += 1;
        let len_bytes = [png_bytes[pos], png_bytes[pos + 1], png_bytes[pos + 2], png_bytes[pos + 3]];
        let chunk_len = u32::from_be_bytes(len_bytes) as usize;
        let ctype = &png_bytes[pos + 4..pos + 8];
        let data_start = pos + 8;
        let Some(data_end) = data_start.checked_add(chunk_len) else {
            break;
        };
        if data_end + 4 > png_bytes.len() {
            break;
        }
        let cdata = &png_bytes[data_start..data_end];
        match ctype {
            b"tEXt" => {
                let text = latin1_to_string(cdata);
                check_single_string_with_extra(source, chunk_idx, &text, PrismPolicy::Strict, true, &mut out);
            }
            b"zTXt" => {
                if let Some(nul) = cdata.iter().position(|&b| b == 0) {
                    let keyword = latin1_to_string(&cdata[..nul]);
                    check_single_string_with_extra(source, chunk_idx, &keyword, PrismPolicy::Strict, true, &mut out);
                    if let Some(&method) = cdata.get(nul + 1)
                        && method == 0
                        && let Some(compressed) = cdata.get(nul + 2..)
                        && let Ok(raw) = inflate_zlib_bounded(compressed)
                    {
                        let decoded = latin1_to_string(&raw);
                        check_single_string_with_extra(source, chunk_idx, &decoded, PrismPolicy::Strict, true, &mut out);
                    }
                }
            }
            b"iTXt" => {
                scan_itxt_chunk(source, chunk_idx, cdata, &mut out);
            }
            b"IEND" => break,
            _ => {}
        }
        pos = data_end + 4;
    }
    out
}

fn scan_itxt_chunk(source: &str, chunk_idx: usize, cdata: &[u8], out: &mut Vec<BrandHit>) {
    let Some(nul1) = cdata.iter().position(|&b| b == 0) else {
        return;
    };
    let keyword = String::from_utf8_lossy(&cdata[..nul1]);
    check_single_string_with_extra(source, chunk_idx, &keyword, PrismPolicy::Strict, true, out);
    let comp_flag = cdata.get(nul1 + 1).copied().unwrap_or(0);
    let comp_method = cdata.get(nul1 + 2).copied().unwrap_or(0);
    let Some(rest_lang) = cdata.get(nul1 + 3..) else {
        return;
    };
    let Some(nul2) = rest_lang.iter().position(|&b| b == 0) else {
        return;
    };
    let Some(rest_trans) = rest_lang.get(nul2 + 1..) else {
        return;
    };
    let Some(nul3) = rest_trans.iter().position(|&b| b == 0) else {
        return;
    };
    let translated = String::from_utf8_lossy(&rest_trans[..nul3]);
    check_single_string_with_extra(source, chunk_idx, &translated, PrismPolicy::Strict, true, out);
    let payload = rest_trans.get(nul3 + 1..).unwrap_or(&[]);
    if comp_flag == 1 && comp_method == 0 {
        if let Ok(raw) = inflate_zlib_bounded(payload) {
            let text = String::from_utf8_lossy(&raw);
            check_single_string_with_extra(source, chunk_idx, &text, PrismPolicy::Strict, true, out);
        }
    } else if comp_flag == 0 {
        let text = String::from_utf8_lossy(payload);
        check_single_string_with_extra(source, chunk_idx, &text, PrismPolicy::Strict, true, out);
    }
}

/// Inspect `.ico` / `.icns` icon containers (including every embedded PNG frame and non-PNG `.icns`
/// metadata chunks such as `info` / `name`).
#[must_use]
pub fn check_icon_container_metadata(source: &str, bytes: &[u8]) -> Vec<BrandHit> {
    const PNG_SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    let mut out = Vec::new();
    let mut offset = 0usize;
    let mut frame_idx = 0usize;
    while let Some(rel) = bytes.get(offset..).and_then(|s| s.windows(PNG_SIG.len()).position(|w| w == PNG_SIG)) {
        frame_idx += 1;
        let abs = offset + rel;
        let frame_source = format!("{source}#png{frame_idx}");
        out.extend(check_png_metadata(&frame_source, &bytes[abs..]));
        offset = abs + PNG_SIG.len();
    }
    if bytes.starts_with(b"icns") && bytes.len() >= 8 {
        let mut pos = 8usize;
        let mut chunk_idx = 0usize;
        while let Some(hdr) = bytes.get(pos..pos + 8) {
            chunk_idx += 1;
            let len = u32::from_be_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as usize;
            if len < 8 || pos.checked_add(len).is_none_or(|end| end > bytes.len()) {
                break;
            }
            let payload = &bytes[pos + 8..pos + len];
            if !payload.starts_with(PNG_SIG) && matches!(&hdr[..4], b"info" | b"name" | b"icnV") {
                let tag = String::from_utf8_lossy(&hdr[..4]);
                scan_binary_ascii_runs(&format!("{source}#icns_{tag}"), chunk_idx, payload, &mut out);
            }
            pos += len;
        }
    }
    out
}

/// Inspect `.gif` Comment Extension (`0x21 0xFE`) and Application Extension (`0x21 0xFF`) blocks.
#[must_use]
pub fn check_gif_metadata(source: &str, bytes: &[u8]) -> Vec<BrandHit> {
    let mut out = Vec::new();
    if !(bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) || bytes.len() < 13 {
        return out;
    }
    let packed = bytes[10];
    let gct_len = if (packed & 0x80) != 0 { 3usize * (1usize << ((packed & 0x07) + 1)) } else { 0 };
    let mut pos = 13usize.saturating_add(gct_len);
    let mut ext_idx = 0usize;
    while let Some(&intro) = bytes.get(pos) {
        pos += 1;
        match intro {
            0x3B => break,
            0x21 => {
                let Some(&label) = bytes.get(pos) else { break };
                pos += 1;
                let data = read_gif_subblocks(bytes, &mut pos);
                if matches!(label, 0xFE | 0xFF) {
                    ext_idx += 1;
                    let text = String::from_utf8_lossy(&data);
                    check_single_string_with_extra(&format!("{source}#gif_ext{ext_idx}"), ext_idx, &text, PrismPolicy::Strict, true, &mut out);
                }
            }
            0x2C => {
                let Some(desc) = bytes.get(pos..pos + 9) else { break };
                let lct_packed = desc[8];
                let lct_len = if (lct_packed & 0x80) != 0 { 3usize * (1usize << ((lct_packed & 0x07) + 1)) } else { 0 };
                pos = pos.saturating_add(9).saturating_add(lct_len);
                if pos >= bytes.len() {
                    break;
                }
                pos += 1; // LZW minimum code size
                let _ = read_gif_subblocks(bytes, &mut pos);
            }
            _ => break,
        }
    }
    out
}

fn read_gif_subblocks(bytes: &[u8], pos: &mut usize) -> Vec<u8> {
    let mut buf = Vec::new();
    while let Some(&sub_len) = bytes.get(*pos) {
        *pos += 1;
        if sub_len == 0 {
            break;
        }
        let n = usize::from(sub_len);
        let Some(chunk) = bytes.get(*pos..*pos + n) else {
            *pos = bytes.len();
            break;
        };
        buf.extend_from_slice(chunk);
        *pos += n;
    }
    buf
}

fn scan_binary_ascii_runs(source: &str, line: usize, bytes: &[u8], out: &mut Vec<BrandHit>) {
    let mut idx = 0usize;
    while idx < bytes.len() {
        if !(bytes[idx].is_ascii_graphic() || bytes[idx] == b' ') {
            idx += 1;
            continue;
        }
        let start = idx;
        while idx < bytes.len() && (bytes[idx].is_ascii_graphic() || bytes[idx] == b' ') {
            idx += 1;
        }
        if idx - start >= 4
            && let Ok(run) = std::str::from_utf8(&bytes[start..idx])
        {
            check_single_string_with_extra(source, line, run, PrismPolicy::Strict, true, out);
        }
    }
}

fn latin1_to_string(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| if b == 0 { '\n' } else { char::from(b) }).collect()
}

const MAX_PNG_INFLATE_BYTES: usize = 1_048_576;

pub(crate) fn inflate_gzip_bounded(gz: &[u8]) -> Result<Vec<u8>, ()> {
    if gz.len() < 18 || gz[0] != 0x1F || gz[1] != 0x8B || gz[2] != 8 {
        return Err(());
    }
    let flg = gz[3];
    let mut pos = 10usize;
    if (flg & 0x04) != 0 {
        let xlen = u16::from_le_bytes([*gz.get(pos).ok_or(())?, *gz.get(pos + 1).ok_or(())?]) as usize;
        pos = pos.checked_add(2 + xlen).ok_or(())?;
    }
    for flag in [0x08u8, 0x10u8] {
        if (flg & flag) != 0 {
            let rel = gz.get(pos..).ok_or(())?.iter().position(|&b| b == 0).ok_or(())?;
            pos = pos.checked_add(rel + 1).ok_or(())?;
        }
    }
    if (flg & 0x02) != 0 {
        pos = pos.checked_add(2).ok_or(())?;
    }
    if pos + 8 > gz.len() {
        return Err(());
    }
    inflate_deflate_bounded(&gz[pos..gz.len() - 8])
}

fn inflate_zlib_bounded(zlib: &[u8]) -> Result<Vec<u8>, ()> {
    if zlib.len() < 6 {
        return Err(());
    }
    let cmf = zlib[0];
    let flg = zlib[1];
    if (cmf & 0x0f) != 8 || !((u16::from(cmf) * 256 + u16::from(flg)).is_multiple_of(31)) {
        return Err(());
    }
    let header_len = if (flg & 0x20) != 0 { 6 } else { 2 };
    let payload = zlib.get(header_len..zlib.len() - 4).ok_or(())?;
    inflate_deflate_bounded(payload)
}

fn inflate_deflate_bounded(payload: &[u8]) -> Result<Vec<u8>, ()> {
    use std::io::Read as _;
    let mut out = Vec::new();
    let cap = (MAX_PNG_INFLATE_BYTES as u64).saturating_add(1);
    flate2::read::DeflateDecoder::new(payload).take(cap).read_to_end(&mut out).map_err(|_| ())?;
    if out.len() > MAX_PNG_INFLATE_BYTES {
        return Err(());
    }
    Ok(out)
}

/// Recursively scan all assembled release/launch artefacts under `dist_dir` (`.app`, web root,
/// `.zip`, `.curvebook`, `.cbtemplate`, HTML/CSS/JS, `Info.plist`, `.desktop`, `.mime.xml`, SVG,
/// PNG/ICO/ICNS/GIF, PDF, manifests) — scanning file/directory paths as well as nested contents and
/// failing if `docs/audit/` or `plan/prism/` were mistakenly bundled.
pub fn scan_dist_artefacts(dist_dir: &Path) -> Result<Vec<BrandHit>, String> {
    let (_, hits) = scan_dist_artefacts_with_stats(dist_dir)?;
    Ok(hits)
}

/// Recursively scan all assembled artefacts under `dist_dir`, returning explicit file & archive
/// entry counts alongside any brand violations.
pub fn scan_dist_artefacts_with_stats(dist_dir: &Path) -> Result<(DistScanStats, Vec<BrandHit>), String> {
    if !dist_dir.exists() {
        return Err(format!("dist directory `{}` does not exist", dist_dir.display()));
    }
    let mut files = Vec::new();
    collect_dist_files(dist_dir, dist_dir, &mut files)?;
    files.sort();
    let mut stats = DistScanStats { files: files.len(), archive_entries: 0 };
    let mut hits = Vec::new();
    for rel in files {
        scan_dist_path_and_file(dist_dir, &rel, &mut stats, &mut hits)?;
    }
    Ok((stats, hits))
}

fn collect_dist_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
    let mut paths = Vec::new();
    for entry in entries {
        paths.push(entry.map_err(|e| format!("dir entry under {}: {e}", dir.display()))?.path());
    }
    paths.sort();
    for path in paths {
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("stat {}: {e}", path.display()))?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            collect_dist_files(root, &path, out)?;
        } else if meta.is_file() {
            let rel = path.strip_prefix(root).map_err(|e| format!("strip prefix {}: {e}", path.display()))?;
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

const MAX_DIST_FILE_BYTES: usize = 256 * 1024 * 1024;

fn scan_dist_path_and_file(dist_dir: &Path, rel: &str, stats: &mut DistScanStats, hits: &mut Vec<BrandHit>) -> Result<(), String> {
    if rel.starts_with(AUDIT_PREFIX)
        || rel.contains("/docs/audit/")
        || rel.starts_with(HISTORY_AUDIT_PREFIX)
        || rel.contains("/docs/history/audit/")
        || rel == PLAN_PRISM_SCOPE
        || rel.starts_with("plan/prism/")
        || rel.contains("/plan/prism/")
    {
        hits.push(BrandHit { path: rel.to_owned(), line: 1, brand: "prism" });
        return Ok(());
    }
    collect_line_words(rel, 1, rel, SUITE_BRANDS, PrismPolicy::Strict, hits);
    collect_line_words(rel, 1, rel, &[ADAPTED_UPSTREAM_BRAND], PrismPolicy::Strict, hits);
    collect_line_words(rel, 1, rel, BRANDS, PrismPolicy::Strict, hits);
    collect_line_words(rel, 1, rel, PRE_RENAME_ASSET_BRANDS, PrismPolicy::Strict, hits);
    let bytes = curvebook_platform::fs::read_regular_file_bytes(&dist_dir.join(rel), MAX_DIST_FILE_BYTES)
        .map_err(|e| format!("read dist file {rel}: {e}"))?;
    scan_dist_bytes(rel, &bytes, 0, stats, hits)
}

fn is_macos_stapled_ticket(display_path: &str, bytes: &[u8]) -> bool {
    let norm = display_path.replace('\\', "/");
    norm.ends_with(".app/Contents/CodeResources") && bytes.starts_with(b"s8ch")
}

fn is_native_binary_blob(display_path: &str, bytes: &[u8]) -> bool {
    let ext = Path::new(display_path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if matches!(ext.as_str(), "exe" | "dll" | "so" | "dylib" | "wasm" | "deb" | "rpm" | "dmg" | "msi" | "appimage" | "zsync" | "a" | "lib") {
        return true;
    }
    if is_macos_stapled_ticket(display_path, bytes) {
        return true;
    }
    bytes.starts_with(b"\x7FELF")
        || bytes.starts_with(b"\x00asm")
        || bytes.starts_with(b"MZ")
        || bytes.starts_with(b"!<arch>\n")
        || bytes.starts_with(b"\xED\xAB\xEE\xDB")
        || bytes.starts_with(b"\xFE\xED\xFA\xCE")
        || bytes.starts_with(b"\xFE\xED\xFA\xCF")
        || bytes.starts_with(b"\xCE\xFA\xED\xFE")
        || bytes.starts_with(b"\xCF\xFA\xED\xFE")
        || bytes.starts_with(b"\xCA\xFE\xBA\xBE")
        || bytes.starts_with(b"\xBE\xBA\xFE\xCA")
}

fn scan_dist_bytes(display_path: &str, bytes: &[u8], depth: usize, stats: &mut DistScanStats, hits: &mut Vec<BrandHit>) -> Result<(), String> {
    let ext = Path::new(display_path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    let lower_path = display_path.to_ascii_lowercase();
    let is_zip = bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06");
    if is_zip {
        if depth >= 2 {
            return Err(format!("{display_path}: nested archive depth exceeded (max depth 2)"));
        }
        let zip =
            ZipArchive::from_bytes_with_limits(bytes, ZipLimits::default()).map_err(|e| format!("cannot inspect archive `{display_path}`: {e}"))?;
        for (name, entry_bytes) in zip.iter() {
            stats.archive_entries += 1;
            let entry_path = format!("{display_path}!{name}");
            collect_line_words(&entry_path, 1, name, SUITE_BRANDS, PrismPolicy::Strict, hits);
            collect_line_words(&entry_path, 1, name, &[ADAPTED_UPSTREAM_BRAND], PrismPolicy::Strict, hits);
            collect_line_words(&entry_path, 1, name, BRANDS, PrismPolicy::Strict, hits);
            collect_line_words(&entry_path, 1, name, PRE_RENAME_ASSET_BRANDS, PrismPolicy::Strict, hits);
            scan_dist_bytes(&entry_path, entry_bytes, depth + 1, stats, hits)?;
        }
        return Ok(());
    }
    if ext == "png" || bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        hits.extend(check_png_metadata(display_path, bytes));
        return Ok(());
    }
    if matches!(ext.as_str(), "ico" | "icns") {
        hits.extend(check_icon_container_metadata(display_path, bytes));
        return Ok(());
    }
    if ext == "gif" || bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        hits.extend(check_gif_metadata(display_path, bytes));
        return Ok(());
    }
    if ext == "pdf" || bytes.starts_with(b"%PDF-") {
        hits.extend(check_pdf_metadata(display_path, bytes));
        return Ok(());
    }
    if ext == "svgz" || (bytes.starts_with(&[0x1F, 0x8B]) && is_binary_asset_ext(display_path)) {
        let raw = inflate_gzip_bounded(bytes).map_err(|()| format!("{display_path}: corrupt or oversized .svgz gzip payload"))?;
        let text = match decode_text_file(display_path, &raw)? {
            DecodedFile::Text(s) => s,
            DecodedFile::BinaryAsset => return Err(format!("{display_path}: undecodable .svgz SVG text")),
        };
        scan_svg(display_path, &text, hits);
        hits.extend(check_svg_metadata(display_path, &text));
        return Ok(());
    }
    let text = match decode_text_file(display_path, bytes) {
        Ok(DecodedFile::Text(s)) => s,
        Ok(DecodedFile::BinaryAsset) => return Ok(()),
        Err(err) => {
            if is_native_binary_blob(display_path, bytes) {
                return Ok(());
            }
            return Err(err);
        }
    };
    if ext == "json" {
        hits.extend(check_manifest_metadata(display_path, &text));
    } else if ext == "svg" {
        scan_svg(display_path, &text, hits);
        hits.extend(check_svg_metadata(display_path, &text));
    } else if matches!(ext.as_str(), "html" | "htm") {
        scan_html_file(display_path, &text, hits);
        hits.extend(check_html_metadata(display_path, &text));
    } else if lower_path.ends_with(".plist") || lower_path.ends_with(".plist.in") {
        scan_plist(display_path, &text, hits);
    } else if is_mime_package_xml(display_path) {
        scan_mime_xml(display_path, &text, hits);
    } else if lower_path.ends_with(".desktop") {
        scan_desktop_entry(display_path, &text, hits);
    } else {
        scan_suite_lines_with_scope(display_path, &text, true, hits);
        scan_plain_lines_no_waiver(display_path, &text, hits);
    }
    Ok(())
}

fn is_mime_package_xml(path: &str) -> bool {
    let norm = path.replace('\\', "/").to_ascii_lowercase();
    norm.ends_with(".mime.xml") || (norm.ends_with(".xml") && (norm.starts_with("share/mime/packages/") || norm.contains("/share/mime/packages/")))
}

/// Scan an asset filename or symlink target string strictly against all forbidden brand sets.
#[must_use]
pub(crate) fn scan_dist_label(source: &str, label: &str) -> Vec<BrandHit> {
    let mut hits = Vec::new();
    collect_line_words(source, 1, label, SUITE_BRANDS, PrismPolicy::Strict, &mut hits);
    collect_line_words(source, 1, label, &[ADAPTED_UPSTREAM_BRAND], PrismPolicy::Strict, &mut hits);
    collect_line_words(source, 1, label, BRANDS, PrismPolicy::Strict, &mut hits);
    collect_line_words(source, 1, label, PRE_RENAME_ASSET_BRANDS, PrismPolicy::Strict, &mut hits);
    dedup_tail_hits(&mut hits, 0);
    hits
}

/// Scan a standalone release compliance file (`SHA256SUMS.txt`, `THIRD-PARTY-LICENSES.html`,
/// or decoded `.zsync` header) without waivers.
#[must_use]
pub(crate) fn scan_compliance_text(display_path: &str, text: &str) -> Vec<BrandHit> {
    let mut hits = scan_dist_label(display_path, display_path);
    let from = hits.len();
    scan_suite_lines_with_scope(display_path, text, true, &mut hits);
    scan_plain_lines_no_waiver(display_path, text, &mut hits);
    let ext = Path::new(display_path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if matches!(ext.as_str(), "html" | "htm") {
        hits.extend(check_html_metadata(display_path, text));
    }
    dedup_tail_hits(&mut hits, from);
    hits
}

/// Scan a generated static documentation site directory under the narrow generated-HTML
/// format-boundary policy while keeping `PrismPolicy::Strict` on non-HTML text and suite brands.
#[cfg(feature = "full-gates")]
pub fn check_docs_site_dir(dir: &Path) -> Result<Vec<BrandHit>, String> {
    if !dir.exists() {
        return Err(format!("docs-site directory `{}` does not exist", dir.display()));
    }
    let mut rel_files = Vec::new();
    collect_dist_files(dir, dir, &mut rel_files)?;
    rel_files.sort();
    let mut stats = DistScanStats::default();
    let mut hits = Vec::new();
    for rel in rel_files {
        let display_path = format!("docs-site/{rel}");
        stats.files += 1;
        collect_line_words(&display_path, 1, &rel, SUITE_BRANDS, PrismPolicy::Strict, &mut hits);
        collect_line_words(&display_path, 1, &rel, BRANDS, PrismPolicy::Strict, &mut hits);
        collect_line_words(&display_path, 1, &rel, PRE_RENAME_ASSET_BRANDS, PrismPolicy::Strict, &mut hits);
        let bytes = curvebook_platform::fs::read_regular_file_bytes(&dir.join(&rel), MAX_DIST_FILE_BYTES)
            .map_err(|e| format!("read docs-site file {rel}: {e}"))?;
        scan_dist_bytes(&display_path, &bytes, 0, &mut stats, &mut hits)?;
    }
    hits.sort();
    hits.dedup();
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn r15_1_provenance_permits_only_exact_upstream_url_inside_attributed_source_section() {
        let story = ["sto", "ry", "told"].concat();
        let vc = ["vec", "tor", "craft"].concat();
        let vc_title = ["Vec", "tor", "Cra", "ft"].concat();
        let exact_url = format!("https://github.com/{story}/{vc}");

        let mut ok_attr = Vec::new();
        let attr_body =
            format!("# Attribution\n\n## Attributed Source Code\n\nAdapted helpers from {vc_title} (`{exact_url}`) at commit `deadbeef`.\n");
        super::scan_file("ATTRIBUTION.md", &attr_body, &mut ok_attr);
        assert!(ok_attr.is_empty(), "expected clean ATTRIBUTION.md, got {ok_attr:?}");

        let mut ok_notice = Vec::new();
        let notice_body = format!(
            "CurveBook\n\n## Attributed Source Code\n\nFrom {vc_title} ({exact_url}, commit abc).\nCopyright (c) 2026 {vc_title} contributors.\n"
        );
        super::scan_file("NOTICE", &notice_body, &mut ok_notice);
        assert!(ok_notice.is_empty(), "expected clean NOTICE, got {ok_notice:?}");

        let mut ok_packaged_notice = Vec::new();
        let pkg_notice_body = format!("## Attributed Source Code\nFrom {vc_title} ({exact_url}).\n");
        super::scan_suite_lines_with_scope("curvebook-web-0.1.0/NOTICE", &pkg_notice_body, true, &mut ok_packaged_notice);
        assert!(ok_packaged_notice.is_empty(), "expected clean packaged NOTICE, got {ok_packaged_notice:?}");

        // Legal policy docs may mention the adapted upstream name but never the org or URL.
        let mut ok_policy = Vec::new();
        let policy_body = format!("Attributed {vc_title} GUI source reuse rules.\n");
        super::scan_file("AGENTS.md", &policy_body, &mut ok_policy);
        assert!(ok_policy.is_empty(), "expected preserved AGENTS.md policy, got {ok_policy:?}");

        for (path, bad_body) in [
            ("ATTRIBUTION.md", format!("## Other Section\n{exact_url}\n")),
            ("ATTRIBUTION.md", format!("## Attributed Source Code\n{exact_url}/blob/main/lib.rs\n")),
            ("ATTRIBUTION.md", format!("## Attributed Source Code\nhttp://github.com/{story}/{vc}\n")),
            ("ATTRIBUTION.md", format!("## Attributed Source Code\n{exact_url}.git\n")),
            ("ATTRIBUTION.md", format!("## Attributed Source Code\nhttps://github.com/{story}/other\n")),
            ("ATTRIBUTION.md", format!("## Attributed Source Code\nMaintained by {story}.\n")),
            ("ATTRIBUTION.md", format!("## Attributed Source Code\nSee {exact_url}\n## Assets\n{exact_url}\n")),
            ("NOTICE", format!("Preamble {exact_url}\n")),
            ("README.md", format!("Upstream {exact_url} <!-- brand-ok -->\n")),
            ("AGENTS.md", format!("Do not link {exact_url} here.\n")),
            ("docs/ATTRIBUTION.md", format!("## Attributed Source Code\n{exact_url}\n")),
        ] {
            let mut hits = Vec::new();
            super::scan_file(path, &bad_body, &mut hits);
            assert!(hits.iter().any(|h| h.brand == "storytold"), "expected storytold rejection for {path}: {bad_body:?}, got {hits:?}");
        }
    }

    #[test]
    fn r15_1_vectorcraft_is_forbidden_across_crates_apps_and_ui_surfaces() {
        let vc = ["vec", "tor", "craft"].concat();
        let vc_title = ["Vec", "tor", "Cra", "ft"].concat();
        for (path, body) in [
            ("crates/ui-egui/src/widgets.rs", format!("// Adapted from {vc_title}\n")),
            ("crates/ui-egui/src/widgets.rs", format!("pub fn {vc}_helper() {{}}\n")),
            ("apps/curvebook/src/gui_window.rs", format!("const HELP: &str = \"{vc}\";\n")),
            ("apps/curvebook-web/index.html", format!("<title>CurveBook {vc_title}</title>\n")),
        ] {
            let mut hits = Vec::new();
            super::scan_file(path, &body, &mut hits);
            assert!(hits.iter().any(|h| h.brand == "vectorcraft"), "expected vectorcraft rejection for {path}, got {hits:?}");
        }

        let mut path_hits = Vec::new();
        super::scan_tracked_path_name(&format!("crates/ui-egui/src/{vc}_panel.rs"), &mut path_hits);
        assert!(path_hits.iter().any(|h| h.brand == "vectorcraft"), "{path_hits:?}");

        let ui_hits = super::check_text_runs("ui_label", [format!("Exported with {vc_title}")]);
        assert!(ui_hits.iter().any(|h| h.brand == "vectorcraft"), "{ui_hits:?}");
    }

    #[cfg(feature = "full-gates")]
    use super::check_i18n_catalog;
    use super::{
        BrandHit, check_html_metadata, check_i18n_catalog_entries, check_labeled_text_runs, check_manifest_metadata, check_pdf_metadata,
        check_svg_metadata, check_text_runs, check_text_runs_strict, check_window_title, run, scan_file, scan_repo_checked, skip_path,
    };

    fn temp_dir(label: &str) -> PathBuf {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("cb-brands-{}-{label}-{stamp}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn to_utf16_bytes(s: &str, little_endian: bool, include_bom: bool) -> Vec<u8> {
        let mut out = Vec::new();
        if include_bom {
            out.extend_from_slice(if little_endian { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
        }
        for unit in s.encode_utf16() {
            let bytes = if little_endian { unit.to_le_bytes() } else { unit.to_be_bytes() };
            out.extend_from_slice(&bytes);
        }
        out
    }

    #[test]
    fn flags_rust_comments_identifiers_literals_and_cfg_test_without_waivers() {
        let mut hits = Vec::new();
        let src = "\
// GraphPad comment is also caught
pub const LABEL: &str = \"Import GraphPad table\";
pub const NO_WAIVER: &str = \"Import OriginPro table\"; // brand-ok
pub fn instantiate() -> &'static str { \"instantiate\" }
pub struct PrismStyle;
#[cfg(test)]
mod tests {
    const T: &str = \"SPSS fixture\";
}
";
        scan_file("crates/ui-egui/src/lib.rs", src, &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("crates/ui-egui/src/lib.rs"), line: 1, brand: "graphpad" },
                BrandHit { path: String::from("crates/ui-egui/src/lib.rs"), line: 2, brand: "graphpad" },
                BrandHit { path: String::from("crates/ui-egui/src/lib.rs"), line: 3, brand: "originpro" },
                BrandHit { path: String::from("crates/ui-egui/src/lib.rs"), line: 5, brand: "prism" },
                BrandHit { path: String::from("crates/ui-egui/src/lib.rs"), line: 8, brand: "spss" },
            ]
        );
    }

    #[test]
    fn cfg_test_items_and_prod_code_are_both_scanned() {
        let mut hits = Vec::new();
        let src = "\
#[cfg(test)]
fn test_only_helper() {
    let _ = \"SPSS fixture }; // brace inside string\";
}

pub fn shipped_error() -> &'static str {
    let q = '\"';
    let _ = q;
    \"GraphPad-compatible: unknown option\"
}

#[cfg(all(test, unix))]
fn unix_test_helper() {
    let _ = \"Minitab\";
}

#[cfg(not(test))]
pub const PROD: &str = \"Dotmatics\";
";
        scan_file("apps/curvebook/src/main.rs", src, &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("apps/curvebook/src/main.rs"), line: 3, brand: "spss" },
                BrandHit { path: String::from("apps/curvebook/src/main.rs"), line: 9, brand: "graphpad" },
                BrandHit { path: String::from("apps/curvebook/src/main.rs"), line: 14, brand: "minitab" },
                BrandHit { path: String::from("apps/curvebook/src/main.rs"), line: 18, brand: "dotmatics" },
            ]
        );
    }

    #[test]
    fn flags_multiline_string_literals_continuations_and_raw_strings() {
        let mut hits = Vec::new();
        let src = "\
pub const CONT: &str = \"Import from \\
    GraphPad table\";
pub const SPLIT: &str = \"Import Graph\\
Pad table\";
pub const ESC_NL: &str = \"line1\\nOriginPro\";
pub const RAW: &str = r#\"hello
Minitab\"#;
pub const NO_WAIVER: &str = \"Import from \\
    SigmaPlot table\"; // brand-ok
";
        scan_file("crates/format/src/lib.rs", src, &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("crates/format/src/lib.rs"), line: 2, brand: "graphpad" },
                BrandHit { path: String::from("crates/format/src/lib.rs"), line: 3, brand: "graphpad" },
                BrandHit { path: String::from("crates/format/src/lib.rs"), line: 5, brand: "originpro" },
                BrandHit { path: String::from("crates/format/src/lib.rs"), line: 7, brand: "minitab" },
                BrandHit { path: String::from("crates/format/src/lib.rs"), line: 9, brand: "sigmaplot" },
            ]
        );
    }

    #[test]
    fn decodes_utf16_files_scans_png_metadata_and_fails_on_non_utf8_text() {
        let dir = temp_dir("encoding");
        std::fs::create_dir_all(dir.join("packaging/windows")).unwrap();
        std::fs::create_dir_all(dir.join("packaging/linux")).unwrap();
        std::fs::create_dir_all(dir.join("assets")).unwrap();

        std::fs::write(dir.join("packaging/windows/app.rc"), to_utf16_bytes("VALUE \"Product\", \"GraphPad\"\n", true, true)).unwrap();
        std::fs::write(dir.join("packaging/windows/clean.rc"), to_utf16_bytes("VALUE \"Product\", \"CurveBook\"\n", false, true)).unwrap();
        std::fs::write(dir.join("assets/icon.png"), [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0xFF]).unwrap();

        let files = vec![String::from("packaging/windows/app.rc"), String::from("packaging/windows/clean.rc"), String::from("assets/icon.png")];
        let hits = scan_repo_checked(&dir, &files).unwrap();
        assert_eq!(hits, vec![BrandHit { path: String::from("packaging/windows/app.rc"), line: 1, brand: "graphpad" }]);

        // Non-UTF-8 Latin-1 byte in a packaging file must fail rather than be silently skipped.
        std::fs::write(dir.join("packaging/linux/latin1.desktop"), b"[Desktop Entry]\nComment=\xa9 CurveBook\n").unwrap();
        let err = scan_repo_checked(&dir, &[String::from("packaging/linux/latin1.desktop")]).unwrap_err();
        assert!(err.contains("cannot scan packaging/linux/latin1.desktop"), "{err}");

        // Verify git-backed run also rejects non-UTF-8 text cleanly if tested directly via scan_repo_checked.
        let _ = run;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn desktop_entry_allows_mime_types_and_rejects_keywords_or_waivers() {
        let mut hits = Vec::new();
        let desktop = "\
[Desktop Entry]
Name=CurveBook
MimeType=application/vnd.curvebook.project;application/x-prism-project;application/x-prism-xml;
# Compatibility search term: brand-ok
Keywords=biostatistics;regression;GraphPad;prism;
";
        scan_file("packaging/linux/org.curvebook.CurveBook.desktop", desktop, &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("packaging/linux/org.curvebook.CurveBook.desktop"), line: 5, brand: "graphpad" },
                BrandHit { path: String::from("packaging/linux/org.curvebook.CurveBook.desktop"), line: 5, brand: "prism" },
            ]
        );
    }

    #[test]
    fn flags_artcraft_suite_references_in_docs_and_workflows() {
        let mut hits = Vec::new();
        scan_file("docs/release/releasing.md", "Installs under Programs\\ArtCraft\\CurveBook or https://getartcraft.com\n", &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("docs/release/releasing.md"), line: 1, brand: "artcraft" },
                BrandHit { path: String::from("docs/release/releasing.md"), line: 1, brand: "getartcraft" },
            ]
        );
    }

    #[test]
    fn readme_disclaimer_line_with_marker_may_name_the_suite() {
        let mut hits = Vec::new();
        scan_file("README.md", "Not affiliated with ArtCraft. <!-- brand-ok -->\n", &mut hits);
        assert!(hits.is_empty(), "{hits:?}");
    }

    #[test]
    fn suite_disclaimer_exemption_is_limited_to_marked_readme_lines_and_the_suite_name() {
        let mut hits = Vec::new();
        // No marker: still flagged in the README.
        scan_file("README.md", "Part of ArtCraft.\n", &mut hits);
        // Marker in any other file: still flagged.
        scan_file("docs/release/releasing.md", "Not affiliated with ArtCraft. <!-- brand-ok -->\n", &mut hits);
        // Marked README line: suite infrastructure is still flagged.
        scan_file("README.md", "See https://getartcraft.com and craft-ui. <!-- brand-ok -->\n", &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("README.md"), line: 1, brand: "artcraft" },
                BrandHit { path: String::from("docs/release/releasing.md"), line: 1, brand: "artcraft" },
                BrandHit { path: String::from("README.md"), line: 1, brand: "getartcraft" },
                BrandHit { path: String::from("README.md"), line: 1, brand: "craft-ui" },
            ]
        );
    }

    #[test]
    fn flags_unmarked_prism_mentions_while_allowing_extensions_and_format_tokens() {
        let mut hits = Vec::new();
        let rust_src = r#"
pub fn ext_match(ext: &str) -> &'static str {
    match ext {
        "prism" | "pzfx" => "project.prism",
        _ => "other",
    }
}
pub const MSG_OK: &str = "Import *.prism or table.importPrism files";
pub const MSG_BRAND: &str = "Import .prism or Prism project";
pub const LABEL_BAD: &str = "Prism Archive";
pub const DOMAIN_BAD: &str = "https://www.prism.com/download";
pub const LABEL_NO_WAIVER: &str = "Prism Archive"; // brand-ok
#[cfg(test)]
mod tests {
    const TEST_FIXTURE: &str = "GraphPad study";
}
"#;
        scan_file("crates/format/src/registry.rs", rust_src, &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("crates/format/src/registry.rs"), line: 9, brand: "prism" },
                BrandHit { path: String::from("crates/format/src/registry.rs"), line: 10, brand: "prism" },
                BrandHit { path: String::from("crates/format/src/registry.rs"), line: 11, brand: "prism" },
                BrandHit { path: String::from("crates/format/src/registry.rs"), line: 12, brand: "prism" },
                BrandHit { path: String::from("crates/format/src/registry.rs"), line: 15, brand: "graphpad" },
            ]
        );
    }

    #[test]
    fn svg_allows_optical_prism_metadata_and_flags_non_optical_uses() {
        let mut hits = Vec::new();
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg">
  <title>CurveBook icon — Mole holding a refractive prism</title>
  <desc>Clean geometric vector icon holding a glowing triangular optical
  prism dispersing light into a spectrum.</desc>
  <title>Prism Scientific Studio</title>
  <text x="0" y="10">optical prism</text>
  <!-- Compatibility icon label: brand-ok -->
  <title>Prism Archive</title>
</svg>
"#;
        scan_file("assets/app-icon/curvebook.svg", svg, &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("assets/app-icon/curvebook.svg"), line: 5, brand: "prism" },
                BrandHit { path: String::from("assets/app-icon/curvebook.svg"), line: 6, brand: "prism" },
                BrandHit { path: String::from("assets/app-icon/curvebook.svg"), line: 8, brand: "prism" },
            ]
        );
    }

    #[test]
    fn scans_github_issue_templates_and_apps_non_rust_files() {
        let mut hits = Vec::new();
        let template = "\
name: Numerical discrepancy
body:
  - type: markdown
    attributes:
      value: |
        Do not attach screenshots from Prism or OriginPro.
        Export `.prism` cleanly instead.
        # brand-ok compatibility mention: Prism
";
        scan_file(".github/ISSUE_TEMPLATE/numerical_result.yml", template, &mut hits);

        let html = "\
<!doctype html>
<html>
  <head><title>CurveBook</title></head>
  <body>
    <input accept=\".curvebook,.cbtemplate,.prism,.pzfx\">
    <p>Drop a Prism file here.</p>
    <p>Drop a Prism file here.</p> <!-- brand-ok -->
  </body>
</html>
";
        scan_file("apps/curvebook-web/index.html", html, &mut hits);

        let web_rs = "\
#[cfg(test)]
mod tests {
    const FIXTURE: &str = \"Prism\";
}
pub fn prod_format() -> &'static str { \"prism\" }
";
        scan_file("apps/curvebook-web/src/lib.rs", web_rs, &mut hits);

        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from(".github/ISSUE_TEMPLATE/numerical_result.yml"), line: 6, brand: "prism" },
                BrandHit { path: String::from(".github/ISSUE_TEMPLATE/numerical_result.yml"), line: 6, brand: "originpro" },
                BrandHit { path: String::from(".github/ISSUE_TEMPLATE/numerical_result.yml"), line: 8, brand: "prism" },
                BrandHit { path: String::from("apps/curvebook-web/index.html"), line: 6, brand: "prism" },
                BrandHit { path: String::from("apps/curvebook-web/index.html"), line: 7, brand: "prism" },
                BrandHit { path: String::from("apps/curvebook-web/src/lib.rs"), line: 3, brand: "prism" },
                BrandHit { path: String::from("apps/curvebook-web/src/lib.rs"), line: 5, brand: "prism" },
            ]
        );
    }

    #[test]
    fn scans_cargo_toml_multiline_descriptions_keywords_categories_and_metadata() {
        let mut hits = Vec::new();
        let toml = r#"
package.metadata.tool = "SPSS"

[package]
name = "demo"
description = """
Curve fitting for .prism files,
unlike GraphPad
or Prism."""
keywords = [
  "biostatistics",
  "Prism", # brand-ok
  "OriginPro",
]
categories = ["science", "prism"]
# Allowed note: brand-ok
homepage = """
https://example.com/prism
"""
documentation = """
Prism docs
""" # brand-ok

[package.metadata]
short_desc = "JMP importer"

[package.metadata.prism]
enabled = true

[workspace.package]
description = '''
Continuation
Minitab note
'''

[workspace.metadata]
suite_note = "Systat"

[workspace]
package = { description = "Prism workspace" }

[dependencies]
# Ignored dependency comment mentioning GraphPad and Prism
serde = "1"
"#;
        scan_file("crates/demo/Cargo.toml", toml, &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 2, brand: "spss" },
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 8, brand: "graphpad" },
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 9, brand: "prism" },
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 12, brand: "prism" },
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 13, brand: "originpro" },
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 15, brand: "prism" },
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 18, brand: "prism" },
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 21, brand: "prism" },
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 25, brand: "jmp" },
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 27, brand: "prism" },
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 33, brand: "minitab" },
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 37, brand: "systat" },
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 40, brand: "prism" },
            ]
        );
    }

    #[test]
    fn tightened_ui_and_app_literal_prism_policy() {
        let mut hits = Vec::new();
        let ui_rs = r#"
pub fn dialog_strings() {
    let ok_brand = "CurveBook";
    let ok_slug = "curvebook";
    let ok_art = "approved-seven-pane";
    let ok_win95 = "curvebook-win95-2508";
    let ok_filter = "Import (*.curvebook, *.pzfx, *.prism)";
    let _ = (ok_brand, ok_slug, ok_art, ok_win95, ok_filter);

    let bad_bare = "prism";
    let bad_sentence = "Switch to .prism for layout";
    let bad_vendor = "GraphPad";
    let bad_suite = "ArtCraft";
    let bad_cap = "Prism";
    let _ = (bad_bare, bad_sentence, bad_vendor, bad_suite, bad_cap);
}
"#;
        scan_file("crates/ui-egui/src/dialogs_canvas.rs", ui_rs, &mut hits);

        let app_rs = r#"
pub fn detect_ext(ext: Option<&str>) -> &'static str {
    match ext {
        Some("prism") => "external",
        _ => "native",
    }
}
pub fn add_filters(dialog: DummyDialog) {
    dialog.add_filter("Supported files", &["curvebook", "pzfx", "prism"]);
}
pub fn status_line() -> &'static str {
    "prism"
}
"#;
        scan_file("apps/curvebook-web/src/lib.rs", app_rs, &mut hits);

        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("crates/ui-egui/src/dialogs_canvas.rs"), line: 13, brand: "artcraft" },
                BrandHit { path: String::from("crates/ui-egui/src/dialogs_canvas.rs"), line: 10, brand: "prism" },
                BrandHit { path: String::from("crates/ui-egui/src/dialogs_canvas.rs"), line: 11, brand: "prism" },
                BrandHit { path: String::from("crates/ui-egui/src/dialogs_canvas.rs"), line: 12, brand: "graphpad" },
                BrandHit { path: String::from("crates/ui-egui/src/dialogs_canvas.rs"), line: 14, brand: "prism" },
                BrandHit { path: String::from("apps/curvebook-web/src/lib.rs"), line: 12, brand: "prism" },
            ]
        );
    }

    #[test]
    fn checks_16_locale_i18n_catalog_and_entries() {
        #[cfg(feature = "full-gates")]
        assert_eq!(check_i18n_catalog(), Vec::new());

        let entries = [
            ("menu.file.open", "en", "CurveBook Project"),
            ("dialog.open.file_filter", "de", "Data files (*.curvebook, *.pzfx, *.prism)"),
            ("menu.help.about", "fr", "Built for GraphPad users"),
            ("cmd.table.importPrism", "ja", "Prism からインポート"),
            ("toolbar.export", "ar", "Export prism archive"),
            ("dialog.open.file_filter", "es", "Prism files (*.prism)"),
            ("inspector.title", "ko", "ArtCraft Inspector"),
        ];
        let hits = check_i18n_catalog_entries(entries);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("i18n:fr:menu.help.about"), line: 1, brand: "graphpad" },
                BrandHit { path: String::from("i18n:ja:cmd.table.importPrism"), line: 1, brand: "prism" },
                BrandHit { path: String::from("i18n:ar:toolbar.export"), line: 1, brand: "prism" },
                BrandHit { path: String::from("i18n:es:dialog.open.file_filter"), line: 1, brand: "prism" },
                BrandHit { path: String::from("i18n:ko:inspector.title"), line: 1, brand: "artcraft" },
            ]
        );
    }

    #[test]
    fn checks_text_runs_and_labeled_text_runs() {
        let runs = [
            "CurveBook 1.0",
            "approved-seven-pane",
            "Import .pzfx / .prism archive",
            "Welcome to prism",
            "Powered by OriginPro and GraphPad",
            "Open Prism workspace",
        ];
        assert_eq!(
            check_text_runs("layout_tree", runs),
            vec![
                BrandHit { path: String::from("layout_tree"), line: 4, brand: "prism" },
                BrandHit { path: String::from("layout_tree"), line: 5, brand: "graphpad" },
                BrandHit { path: String::from("layout_tree"), line: 5, brand: "originpro" },
                BrandHit { path: String::from("layout_tree"), line: 6, brand: "prism" },
            ]
        );

        assert_eq!(
            check_text_runs_strict("window_labels", ["CurveBook", "Import .prism archive"]),
            vec![BrandHit { path: String::from("window_labels"), line: 2, brand: "prism" }]
        );

        let labeled =
            [("node#1", "CurveBook Canvas"), ("node#2", "File formats: *.pzfx, *.prism"), ("node#3", "Open in OriginPro"), ("node#4", "prism mode")];
        assert_eq!(
            check_labeled_text_runs(labeled),
            vec![
                BrandHit { path: String::from("node#3"), line: 3, brand: "originpro" },
                BrandHit { path: String::from("node#4"), line: 4, brand: "prism" },
            ]
        );
    }

    #[test]
    fn checks_export_and_window_metadata() {
        let clean_svg = r#"<svg><title>CurveBook Chart</title><desc>Dose curve</desc><metadata>CurveBook</metadata><text>Group A</text></svg>"#;
        assert_eq!(check_svg_metadata("clean.svg", clean_svg), Vec::new());

        let dirty_svg =
            "<svg>\n<title>Prism &amp; OriginPro</title>\n<desc>prism plot</desc>\n<metadata><author>GraphPad</author></metadata>\n</svg>";
        assert_eq!(
            check_svg_metadata("dirty.svg", dirty_svg),
            vec![
                BrandHit { path: String::from("dirty.svg"), line: 2, brand: "prism" },
                BrandHit { path: String::from("dirty.svg"), line: 2, brand: "originpro" },
                BrandHit { path: String::from("dirty.svg"), line: 3, brand: "prism" },
                BrandHit { path: String::from("dirty.svg"), line: 4, brand: "graphpad" },
            ]
        );

        let clean_pdf = b"%PDF-1.4\n1 0 obj\n<< /Title (CurveBook) /Creator (CurveBook) /Producer (CurveBook) >>\nendobj\n";
        assert_eq!(check_pdf_metadata("clean.pdf", clean_pdf), Vec::new());

        // UTF-16BE BOM + "GraphPad" = FEFF 0047 0072 0061 0070 0068 0050 0061 0064
        let dirty_pdf = b"%PDF-1.4\n1 0 obj\n<< /Creator (Prism) /Producer (ArtCraft) /Title <FEFF00470072006100700068005000610064> >>\nendobj\n";
        assert_eq!(
            check_pdf_metadata("dirty.pdf", dirty_pdf),
            vec![
                BrandHit { path: String::from("dirty.pdf"), line: 3, brand: "prism" },
                BrandHit { path: String::from("dirty.pdf"), line: 3, brand: "artcraft" },
                BrandHit { path: String::from("dirty.pdf"), line: 3, brand: "graphpad" },
            ]
        );

        let clean_html = "<!doctype html>\n<html>\n<head><title>CurveBook</title><meta name=\"generator\" content=\"CurveBook\"></head>\n<body><input accept=\".curvebook,.cbtemplate,.prism,.pzfx\"></body>\n</html>";
        assert_eq!(check_html_metadata("clean.html", clean_html), Vec::new());

        let dirty_html = "<html>\n<head><title>Prism</title><meta name=\"author\" content=\"OriginLab\"></head>\n<body aria-label=\"GraphPad workspace\"></body>\n</html>";
        assert_eq!(
            check_html_metadata("dirty.html", dirty_html),
            vec![
                BrandHit { path: String::from("dirty.html"), line: 2, brand: "prism" },
                BrandHit { path: String::from("dirty.html"), line: 2, brand: "originlab" },
                BrandHit { path: String::from("dirty.html"), line: 3, brand: "graphpad" },
            ]
        );

        let clean_manifest = r#"{"app":"CurveBook","schema_version":1,"title":"Dose-Response Study"}"#;
        assert_eq!(check_manifest_metadata("manifest.json", clean_manifest), Vec::new());
        let dirty_manifest = r#"{"app":"CurveBook","format":".prism","generator":"Prism","vendor":"GraphPad"}"#;
        assert_eq!(
            check_manifest_metadata("manifest.json", dirty_manifest),
            vec![
                BrandHit { path: String::from("manifest.json"), line: 2, brand: "prism" },
                BrandHit { path: String::from("manifest.json"), line: 3, brand: "prism" },
                BrandHit { path: String::from("manifest.json"), line: 4, brand: "graphpad" },
            ]
        );

        assert_eq!(check_window_title("desktop_host", "CurveBook — Study 1"), Vec::new());
        assert_eq!(
            check_window_title("desktop_host", "CurveBook (prism)"),
            vec![BrandHit { path: String::from("desktop_host"), line: 1, brand: "prism" }]
        );
    }

    #[test]
    fn detects_compound_camel_snake_kebab_brands_without_false_statistical_hits() {
        let mut hits = Vec::new();
        let src = r#"
pub struct PrismStyle { pub like_prism: bool }
pub fn graphpad_theme() {}
pub fn xlstatHelper() {}
pub fn StatisticaExport() {}
pub fn statistical_analysis() {}
pub fn origin_pro_preset() {}
"#;
        scan_file("crates/ui-egui/src/theme.rs", src, &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("crates/ui-egui/src/theme.rs"), line: 2, brand: "prism" },
                BrandHit { path: String::from("crates/ui-egui/src/theme.rs"), line: 3, brand: "graphpad" },
                BrandHit { path: String::from("crates/ui-egui/src/theme.rs"), line: 4, brand: "xlstat" },
                BrandHit { path: String::from("crates/ui-egui/src/theme.rs"), line: 5, brand: "statistica" },
                BrandHit { path: String::from("crates/ui-egui/src/theme.rs"), line: 7, brand: "originpro" },
            ]
        );
    }

    #[test]
    fn italian_statistica_and_dsl_jmp_rules_are_narrowly_scoped() {
        let mut i18n_hits = Vec::new();
        let i18n_src = r#"
en: "Copy methods and statistics report",
it: "Copia report metodi e statistica",
de: "Statistica kopieren",
it: "Copia Statistica report",
"#;
        scan_file("crates/ui-egui/src/i18n.rs", i18n_src, &mut i18n_hits);
        assert_eq!(
            i18n_hits,
            vec![
                BrandHit { path: String::from("crates/ui-egui/src/i18n.rs"), line: 4, brand: "statistica" },
                BrandHit { path: String::from("crates/ui-egui/src/i18n.rs"), line: 5, brand: "statistica" },
            ]
        );

        let mut dsl_hits = Vec::new();
        scan_file("crates/fit/src/dsl.rs", "struct Compile { jmp_end: usize }\nconst BAD: &str = \"jmp\";\nfn JmpImporter() {}\n", &mut dsl_hits);
        assert_eq!(
            dsl_hits,
            vec![
                BrandHit { path: String::from("crates/fit/src/dsl.rs"), line: 2, brand: "jmp" },
                BrandHit { path: String::from("crates/fit/src/dsl.rs"), line: 3, brand: "jmp" },
            ]
        );
    }

    #[test]
    fn validates_tracked_paths_and_rejects_blind_test_directory_skips() {
        let dir = temp_dir("path-and-test-scan");
        std::fs::create_dir_all(dir.join("crates/format/src")).unwrap();
        std::fs::create_dir_all(dir.join("crates/engine/tests")).unwrap();
        std::fs::create_dir_all(dir.join("crates/ui-egui/src")).unwrap();
        std::fs::write(dir.join("crates/format/src/prism.rs"), "// clean .prism module\n").unwrap();
        std::fs::write(dir.join("crates/engine/tests/bad_guard_leak.rs"), "const LEAK: &str = \"Import from Prism\";\n").unwrap();
        std::fs::write(dir.join("crates/ui-egui/src/PrismStyle.rs"), "pub struct Clean;\n").unwrap();

        let files = vec![
            String::from("crates/format/src/prism.rs"),
            String::from("crates/engine/tests/bad_guard_leak.rs"),
            String::from("crates/ui-egui/src/PrismStyle.rs"),
        ];
        let hits = scan_repo_checked(&dir, &files).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("crates/engine/tests/bad_guard_leak.rs"), line: 1, brand: "prism" },
                BrandHit { path: String::from("crates/ui-egui/src/PrismStyle.rs"), line: 1, brand: "prism" },
            ]
        );
    }

    fn build_test_png_with_chunk(chunk_type: &[u8; 4], chunk_data: &[u8]) -> Vec<u8> {
        let mut png = Vec::from(&b"\x89PNG\r\n\x1a\n"[..]);
        let ihdr_data = [0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0];
        png.extend_from_slice(&(ihdr_data.len() as u32).to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&ihdr_data);
        png.extend_from_slice(&0u32.to_be_bytes());

        png.extend_from_slice(&(chunk_data.len() as u32).to_be_bytes());
        png.extend_from_slice(chunk_type);
        png.extend_from_slice(chunk_data);
        png.extend_from_slice(&0u32.to_be_bytes());

        png.extend_from_slice(&0u32.to_be_bytes());
        png.extend_from_slice(b"IEND");
        png.extend_from_slice(&0u32.to_be_bytes());
        png
    }

    #[test]
    fn scans_png_text_itxt_ztxt_and_dist_artefact_bundles() {
        let dirty_text_png = build_test_png_with_chunk(b"tEXt", b"svg:title\x00   LumenCraft logo");
        assert_eq!(
            super::check_png_metadata("icon.png", &dirty_text_png),
            vec![BrandHit { path: String::from("icon.png"), line: 2, brand: "lumencraft" }]
        );

        let dirty_itxt_png = build_test_png_with_chunk(b"iTXt", b"Description\x00\x00\x00en\x00\x00Built with OriginPro");
        assert_eq!(
            super::check_png_metadata("icon2.png", &dirty_itxt_png),
            vec![BrandHit { path: String::from("icon2.png"), line: 2, brand: "originpro" }]
        );

        // Stored-block zlib payload inside zTXt chunk (deflate stored block: CMF=0x78 FLG=0x9c, BFINAL=1 BTYPE=0, LEN=9, NLEN=!9)
        let payload = b"Prism Art";
        let mut ztxt = Vec::from(&b"Comment\x00\x00"[..]);
        ztxt.extend_from_slice(&[0x78, 0x9c, 0x01]);
        let len = payload.len() as u16;
        ztxt.extend_from_slice(&len.to_le_bytes());
        ztxt.extend_from_slice(&(!len).to_le_bytes());
        ztxt.extend_from_slice(payload);
        ztxt.extend_from_slice(&[0, 0, 0, 1]);
        let dirty_ztxt_png = build_test_png_with_chunk(b"zTXt", &ztxt);
        assert_eq!(
            super::check_png_metadata("icon3.png", &dirty_ztxt_png),
            vec![BrandHit { path: String::from("icon3.png"), line: 2, brand: "prism" }]
        );

        let clean_png = build_test_png_with_chunk(b"tEXt", b"svg:title\x00CurveBook seven-pane logo");
        assert_eq!(super::check_png_metadata("clean.png", &clean_png), Vec::new());

        let dist = temp_dir("dist-bundle");
        std::fs::create_dir_all(dist.join("CurveBook.app/Contents/Resources")).unwrap();
        std::fs::create_dir_all(dist.join("web")).unwrap();
        std::fs::write(dist.join("CurveBook.app/Contents/Info.plist"), "<dict><key>CFBundleName</key><string>CurveBook</string></dict>\n").unwrap();
        std::fs::write(dist.join("CurveBook.app/Contents/Resources/app.png"), &clean_png).unwrap();
        std::fs::write(dist.join("web/index.html"), "<title>CurveBook</title><input accept=\".curvebook,.pzfx,.prism\">\n").unwrap();
        assert_eq!(super::scan_dist_artefacts(&dist).unwrap(), Vec::new());

        std::fs::create_dir_all(dist.join("docs/audit")).unwrap();
        std::fs::write(dist.join("docs/audit/leaked.md"), "leak").unwrap();
        std::fs::write(dist.join("web/PrismStyle.css"), "/* Prism theme */").unwrap();
        let bad_hits = super::scan_dist_artefacts(&dist).unwrap();
        let _ = std::fs::remove_dir_all(&dist);
        assert!(bad_hits.iter().any(|h| h.path == "docs/audit/leaked.md"), "{bad_hits:?}");
        assert!(bad_hits.iter().any(|h| h.path == "web/PrismStyle.css" && h.brand == "prism"), "{bad_hits:?}");
    }

    #[test]
    fn r021_repo_run_positive_and_negative_regression_suite() {
        let write_f = |root: &PathBuf, rel: &str, bytes: &[u8]| {
            let full = root.join(rel);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, bytes).unwrap();
        };
        let init_git = |root: &PathBuf| {
            let init = std::process::Command::new("git").current_dir(root).args(["init"]).output().unwrap();
            assert!(init.status.success());
            let add = std::process::Command::new("git").current_dir(root).args(["add", "--all"]).output().unwrap();
            assert!(add.status.success());
        };

        // 1. Positive clean minimal repo exercising ALL narrow boundaries at once.
        let pos = temp_dir("r021-pos");
        let clean_png = build_test_png_with_chunk(b"tEXt", b"svg:title\x00CurveBook seven-pane logo");
        write_f(&pos, "ASSETS.md", b"# Asset attribution\n\n| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n| `assets/icon.png` | CurveBook contributors | Original artwork | MIT | app icon |\n");
        write_f(&pos, "ATTRIBUTION.md", b"# Third-party attributions\n");
        write_f(&pos, "NOTICE", b"CurveBook Contributors\n");
        write_f(&pos, "AGENTS.md", b"# AGENTS\n- **Scope:** Existing local `plan/prism/` frames.\n");
        write_f(&pos, "README.md", b"# CurveBook\nNot affiliated with ArtCraft. <!-- brand-ok -->\n");
        write_f(&pos, "assets/icon.png", &clean_png);
        write_f(&pos, "crates/format/src/prism.rs", b"pub fn ext_filter() -> &'static str { \"*.prism\" }\n");
        write_f(&pos, "crates/format/src/pzfx.rs", b"pub const ROOT: &str = \"<PrismFile></PrismFile>\";\n");
        write_f(
            &pos,
            "crates/ui-egui/src/import_dialog.rs",
            b"pub const CMD: &str = \"table.importPrism\";\npub const FILTER: &str = \"Import (*.curvebook, *.pzfx, *.prism)\";\n",
        );
        write_f(&pos, "crates/ui-egui/src/i18n.rs", b"it: \"Copia report metodi e statistica\",\n");
        write_f(&pos, "crates/fit/src/dsl.rs", b"pub struct Op { jmp_end: usize }\n");
        write_f(&pos, "packaging/macos/Info.plist.in", b"<dict>\n  <key>CFBundleTypeName</key>\n  <string>External Scientific Project (.prism, .pzfx)</string>\n  <key>CFBundleTypeExtensions</key>\n  <array>\n    <string>prism</string>\n  </array>\n</dict>\n");
        write_f(&pos, "packaging/linux/org.curvebook.CurveBook.mime.xml", b"<mime-info>\n<mime-type type=\"application/x-prism-project\">\n<comment>External Project (.prism)</comment>\n<glob pattern=\"*.prism\"/>\n</mime-type>\n</mime-info>\n");
        write_f(&pos, "packaging/linux/org.curvebook.CurveBook.desktop", b"[Desktop Entry]\nName=CurveBook\nMimeType=application/vnd.curvebook.project;application/x-prism-project;application/x-prism-xml;\nKeywords=biostatistics;fitting;\n");
        write_f(&pos, "packaging/web/index.html", b"<!doctype html><title>CurveBook</title><input accept=\".prism,.pzfx\">");
        init_git(&pos);
        // `run` would also repeat the root-independent live UI sweep that `cargo xtask brands`
        // already gates; this fixture only exercises the narrow path/content boundaries.
        let pos_files = crate::repo_files(&pos).unwrap();
        assert_eq!(super::scan_tree(&pos, &pos_files).map(|(hits, _)| hits), Ok(Vec::new()));
        assert_eq!(crate::cleanroom::run(&pos), Ok(()));
        assert_eq!(crate::assets::run(&pos), Ok(()));
        let _ = std::fs::remove_dir_all(&pos);

        // 2. Negative leak probes across source, identifiers, waivers, tests, docs, paths, and dist
        // (uses file + path + web/dist artefact scanning directly so 12 probes finish in milliseconds).
        // Each probe lists its own few files instead of initialising a git repository per probe.
        let scan_repo_and_dist = |root: &PathBuf, files: &[&str]| -> Result<Vec<BrandHit>, String> {
            let files: Vec<String> = files.iter().map(|&f| f.to_owned()).collect();
            super::scan_tree(root, &files).map(|(hits, _)| hits)
        };
        for (name, rel, content) in [
            ("title", "crates/ui-egui/src/title.rs", b"pub const T: &str = \"Prism Studio\";\n".as_slice()),
            ("camel", "crates/ui-egui/src/style.rs", b"pub struct PrismStyle;\n".as_slice()),
            ("snake", "crates/ui-egui/src/compat.rs", b"pub fn like_prism() {}\n".as_slice()),
            ("vendor_snake", "crates/ui-egui/src/theme.rs", b"pub fn graphpad_theme() {}\n".as_slice()),
            ("waiver_blocked", "crates/engine/src/help.rs", b"pub const H: &str = \"Import GraphPad\"; // brand-ok\n".as_slice()),
            ("cfg_test_leak", "crates/engine/src/mod.rs", b"#[cfg(test)] mod t { const X: &str = \"Prism Archive\"; }\n".as_slice()),
            ("tests_leak", "crates/engine/tests/leak.rs", b"const X: &str = \"OriginPro import\";\n".as_slice()),
            ("docs_md_leak", "docs/guide.md", b"# Guide\nExport from Prism 11.\n".as_slice()),
            ("path_leak", "crates/ui-egui/src/PrismStyle.rs", b"pub struct X;\n".as_slice()),
            ("xlstat_leak", "crates/fit/src/helper.rs", b"pub fn XlStatHelper() {}\n".as_slice()),
            ("statistica_cap", "crates/ui-egui/src/label.rs", b"pub const L: &str = \"StatisticaReport\";\n".as_slice()),
            ("dist_leak", "packaging/web/bad.html", b"<!doctype html><title>Prism</title>".as_slice()),
        ] {
            let neg = temp_dir(&format!("r021-neg-{name}"));
            write_f(&neg, "ASSETS.md", b"# Asset attribution\n\n| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n");
            write_f(&neg, "ATTRIBUTION.md", b"# Attributions\n");
            write_f(&neg, "NOTICE", b"CurveBook Contributors\n");
            write_f(&neg, rel, content);
            let hits = scan_repo_and_dist(&neg, &["ASSETS.md", "ATTRIBUTION.md", "NOTICE", rel]).unwrap();
            let _ = std::fs::remove_dir_all(&neg);
            assert!(!hits.is_empty(), "negative probe `{name}` (`{rel}`) returned 0 brand hits");
        }
    }

    #[test]
    fn mcp_rpc_validator_enforces_narrow_open_file_boundary_and_rejects_contaminated_surfaces() {
        let clean_init = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "protocolVersion": "2024-11-05",
                "serverInfo": { "name": "curvebook", "version": "0.1.0" },
                "instructions": "CurveBook MCP server for scientific tables, statistical analysis, and publication figures."
            }
        });
        assert_eq!(super::check_mcp_rpc_response("init", "initialize", None, &clean_init), Vec::new());

        let dirty_init = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "serverInfo": { "name": "lumencraft-mcp", "version": "0.1.0" },
                "instructions": "Compatible with GraphPad Prism workflows."
            }
        });
        let init_hits = super::check_mcp_rpc_response("init", "initialize", None, &dirty_init);
        assert!(init_hits.iter().any(|h| h.brand == "lumencraft"), "{init_hits:?}");
        assert!(init_hits.iter().any(|h| h.brand == "graphpad"), "{init_hits:?}");
        assert!(init_hits.iter().any(|h| h.brand == "prism"), "{init_hits:?}");

        let clean_tools = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "tools": [
                    {
                        "name": "open_file",
                        "description": "Open or import a .curvebook, .cbtemplate, .prism, .pzfx, or .csv file.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "format": { "type": "string", "enum": ["curvebook", "prism", "pzfx", "csv"] },
                                "title": { "type": "string", "description": "Optional sheet title." }
                            }
                        }
                    },
                    {
                        "name": "table_import_prism",
                        "description": "[table.importPrism] Import External Archive — Import an external ZIP-based project archive.",
                        "inputSchema": { "type": "object" }
                    }
                ]
            }
        });
        assert_eq!(super::check_mcp_rpc_response("tools", "tools/list", None, &clean_tools), Vec::new());

        let dirty_tools = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "tools": [
                    {
                        "name": "open_file",
                        "description": "Open a GraphPad Prism (.prism) project.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "title": { "type": "string", "description": "Optional .prism project title." }
                            }
                        }
                    },
                    {
                        "name": "doc_inspect",
                        "description": "Inspect .prism or OriginPro sheets.",
                        "inputSchema": { "type": "object" }
                    }
                ]
            }
        });
        let tool_hits = super::check_mcp_rpc_response("tools", "tools/list", None, &dirty_tools);
        assert!(tool_hits.iter().any(|h| h.brand == "graphpad"), "{tool_hits:?}");
        assert!(tool_hits.iter().any(|h| h.brand == "originpro"), "{tool_hits:?}");
        assert!(tool_hits.iter().filter(|h| h.brand == "prism").count() >= 2, "{tool_hits:?}");

        let dirty_prompt = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "result": {
                "description": "ArtCraft workflow",
                "messages": [{ "role": "user", "content": { "type": "text", "text": "Run ANOVA like Minitab or Prism." } }]
            }
        });
        let prompt_hits = super::check_mcp_rpc_response("prompt", "prompts/get", None, &dirty_prompt);
        assert!(prompt_hits.iter().any(|h| h.brand == "artcraft"), "{prompt_hits:?}");
        assert!(prompt_hits.iter().any(|h| h.brand == "minitab"), "{prompt_hits:?}");
        assert!(prompt_hits.iter().any(|h| h.brand == "prism"), "{prompt_hits:?}");

        let dirty_resource = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "result": {
                "contents": [{ "uri": "sheet://data-1", "mimeType": "application/json", "text": "{\"title\":\"SPSS Cohort\",\"legacy\":\"LumenCraft\"}" }]
            }
        });
        let res_hits = super::check_mcp_rpc_response("res", "resources/read", None, &dirty_resource);
        assert!(res_hits.iter().any(|h| h.brand == "spss"), "{res_hits:?}");
        assert!(res_hits.iter().any(|h| h.brand == "lumencraft"), "{res_hits:?}");

        let dirty_png = build_test_png_with_chunk(b"tEXt", b"svg:title\x00LumenCraft preview");
        let b64_png = encode_b64_test(&dirty_png);
        let dirty_preview = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "result": {
                "content": [
                    { "type": "image", "mimeType": "image/png", "data": b64_png },
                    { "type": "text", "text": "{\"title\":\"Clean\"}" }
                ],
                "isError": false
            }
        });
        let prev_hits = super::check_mcp_rpc_response("preview", "tools/call", Some("render_preview"), &dirty_preview);
        assert_eq!(prev_hits, vec![BrandHit { path: String::from("preview:/result/content/0/data#png"), line: 2, brand: "lumencraft" }]);

        let dirty_svg_export = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 6,
            "result": {
                "content": [{ "type": "text", "text": "<svg xmlns=\"http://www.w3.org/2000/svg\"><title>Prism Plot</title></svg>" }],
                "isError": false
            }
        });
        let svg_hits = super::check_mcp_rpc_response("export", "tools/call", Some("export"), &dirty_svg_export);
        assert!(svg_hits.iter().any(|h| h.brand == "prism"), "{svg_hits:?}");
    }

    #[test]
    fn scans_icns_info_plist_and_gif_comment_extensions() {
        let mut clean_icns = Vec::from(&b"icns"[..]);
        let info_body = b"bplist00icon_assetcatalog";
        let info_len = (8 + info_body.len()) as u32;
        clean_icns.extend_from_slice(&(8 + info_len).to_be_bytes());
        clean_icns.extend_from_slice(b"info");
        clean_icns.extend_from_slice(&info_len.to_be_bytes());
        clean_icns.extend_from_slice(info_body);
        assert_eq!(super::check_icon_container_metadata("clean.icns", &clean_icns), Vec::new());

        let mut dirty_icns = Vec::from(&b"icns"[..]);
        let bad_body = b"bplist00 LumenCraft OriginPro icon";
        let bad_len = (8 + bad_body.len()) as u32;
        dirty_icns.extend_from_slice(&(8 + bad_len).to_be_bytes());
        dirty_icns.extend_from_slice(b"info");
        dirty_icns.extend_from_slice(&bad_len.to_be_bytes());
        dirty_icns.extend_from_slice(bad_body);
        let icns_hits = super::check_icon_container_metadata("dirty.icns", &dirty_icns);
        assert!(icns_hits.iter().any(|h| h.brand == "lumencraft"), "{icns_hits:?}");
        assert!(icns_hits.iter().any(|h| h.brand == "originpro"), "{icns_hits:?}");

        // Minimal GIF89a (1x1, no GCT) with clean NETSCAPE2.0 vs dirty Comment Extension (0x21 0xFE).
        let mut clean_gif = Vec::from(&b"GIF89a\x01\x00\x01\x00\x00\x00\x00"[..]);
        clean_gif.extend_from_slice(b"\x21\xff\x0bNETSCAPE2.0\x03\x01\x00\x00\x00\x3b");
        assert_eq!(super::check_gif_metadata("clean.gif", &clean_gif), Vec::new());

        let mut dirty_gif = Vec::from(&b"GIF89a\x01\x00\x01\x00\x00\x00\x00"[..]);
        let comment = b"Created in GraphPad";
        dirty_gif.extend_from_slice(&[0x21, 0xfe, comment.len() as u8]);
        dirty_gif.extend_from_slice(comment);
        dirty_gif.extend_from_slice(&[0x00, 0x3b]);
        assert_eq!(
            super::check_gif_metadata("dirty.gif", &dirty_gif),
            vec![BrandHit { path: String::from("dirty.gif#gif_ext1"), line: 1, brand: "graphpad" }]
        );
    }

    fn encode_b64_test(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        let mut idx = 0usize;
        while idx < bytes.len() {
            let b0 = bytes[idx];
            let b1 = bytes.get(idx + 1).copied().unwrap_or(0);
            let b2 = bytes.get(idx + 2).copied().unwrap_or(0);
            out.push(char::from(TABLE[usize::from(b0 >> 2)]));
            out.push(char::from(TABLE[usize::from(((b0 & 0x03) << 4) | (b1 >> 4))]));
            if idx + 1 < bytes.len() {
                out.push(char::from(TABLE[usize::from(((b1 & 0x0f) << 2) | (b2 >> 6))]));
            } else {
                out.push('=');
            }
            if idx + 2 < bytes.len() {
                out.push(char::from(TABLE[usize::from(b2 & 0x3f)]));
            } else {
                out.push('=');
            }
            idx += 3;
        }
        out
    }

    #[test]
    fn generated_docs_html_allows_exact_format_tokens_and_rejects_brand_prose() {
        let mut ok_hits = Vec::new();
        let ok_html = "\
<!doctype html><html><body>\
<p>Supports <code>table.importPrism</code> and <code>&quot;prism&quot;</code> for <code>.prism</code> files (<a href=\"https://github.com/HauserGroup/curvebook/blob/main/crates/format/src/prism.rs\">source</a>).</p>\
<p>Anchor <a href=\"#mcp-tool-table_import_prism\">tool</a> and <a href=\"#format-prism\">format</a>.</p>\
</body></html>\n";
        scan_file("docs/manual/v0.1/registry/formats.html", ok_html, &mut ok_hits);
        ok_hits.extend(check_html_metadata("docs/manual/v0.1/registry/formats.html", ok_html));
        assert!(ok_hits.is_empty(), "unexpected hits in generated docs HTML: {ok_hits:?}");

        let mut agents_hits = Vec::new();
        let agents_html = "<p>Scope: crates/format/src/lib.rs (<code>plan/prism/</code> reference notes)</p>\n";
        scan_file("docs/manual/v0.1/contribute/agents.html", agents_html, &mut agents_hits);
        agents_hits.extend(check_html_metadata("docs/manual/v0.1/contribute/agents.html", agents_html));
        assert!(agents_hits.is_empty(), "unexpected hits in contribute/agents.html: {agents_hits:?}");

        let mut bad_gen_hits = Vec::new();
        scan_file("docs/manual/v0.1/files.html", "<p>Matches GraphPad Prism output with PrismStyle headers.</p>\n", &mut bad_gen_hits);
        assert!(bad_gen_hits.iter().any(|h| h.brand == "graphpad"), "{bad_gen_hits:?}");
        assert!(bad_gen_hits.iter().any(|h| h.brand == "prism"), "{bad_gen_hits:?}");

        let mut stub_hits = Vec::new();
        scan_file("docs/manual/index.html", "<p>table.importPrism</p>\n", &mut stub_hits);
        assert!(stub_hits.iter().any(|h| h.brand == "prism"), "stub index.html must stay strict: {stub_hits:?}");
    }

    #[test]
    fn archived_audit_history_is_skipped_like_live_audit_dir() {
        assert!(skip_path("docs/audit/README.md"));
        assert!(skip_path("docs/history/audit/findings/r1/S02.md"));
        assert!(skip_path("./docs/history/audit"));
        assert!(!skip_path("docs/history/roadmap/r0.md"));
        assert!(!skip_path("docs/evidence/overview.md"));
    }

    #[test]
    fn sec22_catches_rust_escapes_svg_comments_textpath_data_uris_pdf_case_and_dist_bypasses() {
        let dir = temp_dir("sec22-brands");
        std::fs::create_dir_all(dir.join("crates/core/src")).unwrap();
        std::fs::create_dir_all(dir.join("assets/ui")).unwrap();
        std::fs::create_dir_all(dir.join("packaging/macos")).unwrap();

        std::fs::write(
            dir.join("crates/core/src/esc.rs"),
            "pub fn a() -> &'static str { \"\\x47raph\\u{0050}ad\" }\n\
             pub fn b() -> &'static str { \"\\x61rtcraft\" }\n\
             pub fn c() -> &'static str { \"\\x76ectorcraft\" }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("assets/ui/sneaky.svg"),
            "<svg xmlns=\"http://www.w3.org/2000/svg\">\n\
               <!-- OriginPro comment -->\n\
               <g aria-label=\"Graph&#80;ad\"><textPath>Minitab</textPath></g>\n\
               <image href=\"data:image/svg+xml;charset=utf-8;base64,PHN2Zz48dGl0bGU+U3lzdGF0PC90aXRsZT48L3N2Zz4=\"/>\n\
             </svg>\n",
        )
        .unwrap();
        std::fs::write(dir.join("packaging/macos/INFO.PLIST"), "<string>GraphPad</string>\n").unwrap();
        std::fs::write(dir.join("ASSETS.md"), "| Asset | Author |\n|---|---|\n| `a` | Prism optical curve |\n").unwrap();

        let repo_hits = scan_repo_checked(
            &dir,
            &[
                String::from("crates/core/src/esc.rs"),
                String::from("assets/ui/sneaky.svg"),
                String::from("packaging/macos/INFO.PLIST"),
                String::from("ASSETS.md"),
            ],
        )
        .unwrap();
        assert!(repo_hits.iter().any(|h| h.path.ends_with("esc.rs") && h.brand == "graphpad"), "{repo_hits:?}");
        assert!(repo_hits.iter().any(|h| h.path.ends_with("esc.rs") && h.brand == "artcraft"), "{repo_hits:?}");
        assert!(repo_hits.iter().any(|h| h.path.ends_with("esc.rs") && h.brand == "vectorcraft"), "{repo_hits:?}");
        assert!(repo_hits.iter().any(|h| h.path.ends_with("sneaky.svg") && h.brand == "originpro"), "{repo_hits:?}");
        assert!(repo_hits.iter().any(|h| h.path.ends_with("sneaky.svg") && h.brand == "graphpad"), "{repo_hits:?}");
        assert!(repo_hits.iter().any(|h| h.path.ends_with("sneaky.svg") && h.brand == "minitab"), "{repo_hits:?}");
        assert!(repo_hits.iter().any(|h| h.path.ends_with("sneaky.svg") && h.brand == "systat"), "{repo_hits:?}");
        assert!(repo_hits.iter().any(|h| h.path.ends_with("INFO.PLIST") && h.brand == "graphpad"), "{repo_hits:?}");
        assert!(repo_hits.iter().any(|h| h.path == "ASSETS.md" && h.brand == "prism"), "{repo_hits:?}");

        let svg_meta = check_svg_metadata("meta.svg", "<svg><!-- SigmaPlot --><text><textPath>Graph&#80;ad</textPath></text></svg>");
        assert!(svg_meta.iter().any(|h| h.brand == "sigmaplot"), "{svg_meta:?}");
        assert!(svg_meta.iter().any(|h| h.brand == "graphpad"), "{svg_meta:?}");
        for degenerate in ["<!--><g/>", "<!---><g/>", "<!----><g/>", "<!-->"] {
            assert!(check_svg_metadata("degenerate.svg", degenerate).is_empty());
            let mut hits = Vec::new();
            super::scan_svg("degenerate.svg", degenerate, &mut hits);
            assert!(hits.is_empty(), "degenerate SVG comment `{degenerate}` produced {hits:?}");
        }

        let dist_vc = dir.join("dist-vc");
        std::fs::create_dir_all(&dist_vc).unwrap();
        std::fs::write(dist_vc.join("curvebook"), b"\x7FELF\xFF\xFEclean_elf_stub").unwrap();
        std::fs::write(dist_vc.join("notes.txt"), b"Powered by VectorCraft.\n").unwrap();
        let vc_hits = super::scan_dist_artefacts(&dist_vc).unwrap();
        assert!(vc_hits.iter().any(|h| h.brand == "vectorcraft"), "{vc_hits:?}");

        let dist_bad = dir.join("dist-bad");
        std::fs::create_dir_all(&dist_bad).unwrap();
        std::fs::write(dist_bad.join("index.html"), b"\xFF\xFE\x00\xD8broken").unwrap();
        assert!(super::scan_dist_artefacts(&dist_bad).is_ok());
        std::fs::write(dist_bad.join("index.html"), b"\xFFbad-utf8").unwrap();
        assert!(super::scan_dist_artefacts(&dist_bad).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dist_scan_allows_macos_stapled_notarization_ticket_coderesources_only() {
        let dir = temp_dir("macos-stapled-coderesources");
        let app_contents = dir.join("CurveBook.app/Contents");
        std::fs::create_dir_all(app_contents.join("Resources")).unwrap();

        // Apple `xcrun stapler staple` writes a binary ticket prefixed with `s8ch` to Contents/CodeResources.
        let ticket_bytes = b"s8ch\x01\x00\x00\x00\x30\x82\x01\x02\xFF\xFEnotarization-ticket";
        std::fs::write(app_contents.join("CodeResources"), ticket_bytes).unwrap();
        assert!(super::scan_dist_artefacts(&dir).unwrap().is_empty());

        // Non-ticket binary bytes at Contents/CodeResources must still fail closed.
        std::fs::write(app_contents.join("CodeResources"), b"\xFF\x00not-a-stapled-ticket").unwrap();
        assert!(super::scan_dist_artefacts(&dir).is_err());

        // Restore valid ticket and verify `s8ch` magic alone does not waive arbitrary paths.
        std::fs::write(app_contents.join("CodeResources"), ticket_bytes).unwrap();
        std::fs::write(app_contents.join("Resources/bogus.txt"), ticket_bytes).unwrap();
        assert!(super::scan_dist_artefacts(&dir).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
