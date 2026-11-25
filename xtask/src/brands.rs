//! `cargo xtask brands`: keep competitor vendor/product trademarks and retired suite branding out
//! of user-facing strings, package descriptions, workflows, and packaging metadata (`AGENTS.md`).

use std::collections::BTreeSet;
use std::path::Path;

use crate::rust_lex::{ExtractedLiteral, RustLexer, count_newlines, push_char_with_line};
use crate::scan_util::{collect_dist_files, is_zip_magic, lower_ext};
use crate::text_decode::{decode_utf8_or_utf16, decode_utf16, decode_xml_html_entities};
use crate::zip_read::{ZipArchive, ZipLimits};

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

/// Legacy audit-directory prefixes kept strictly to reject accidental inclusion in `dist/` bundles
/// (`scan_dist_path_and_file`). Historical audit transcripts were retired from the working tree in `R21.4`.
const AUDIT_PREFIX: &str = "docs/audit/";

/// Archived audit-history prefix kept strictly to reject accidental inclusion in `dist/` bundles
/// (`scan_dist_path_and_file`).
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

/// External vendor names the README's non-affiliation disclaimer may cite on a line carrying [`ALLOW_MARKER`].
const VENDOR_DISCLAIMER_NAMES: &[&str] = &["dotmatics", "graphpad"];

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
    let (hits, dist_stats) = scan_tree(root, &files)?;
    if hits.is_empty() {
        if let Some(stats) = dist_stats {
            println!(
                "brands: 0 unapproved brand or suite mentions across {} tracked files and {} dist/ files ({} archive entries)",
                files.len(),
                stats.files,
                stats.archive_entries
            );
        } else {
            println!("brands: 0 unapproved brand or suite mentions across {} tracked files", files.len());
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

pub(crate) fn is_plan_prism_path(path: &str) -> bool {
    let norm = path.replace('\\', "/");
    let trimmed = norm.strip_prefix("./").unwrap_or(&norm);
    trimmed == PLAN_PRISM_SCOPE || trimmed.starts_with("plan/prism/") || trimmed.contains("/plan/prism/") || trimmed.ends_with("/plan/prism")
}

pub(crate) fn is_forbidden_bundled_dir(rel: &str) -> bool {
    let norm = rel.replace('\\', "/");
    let trimmed = norm.strip_prefix("./").unwrap_or(&norm);
    trimmed == "docs/audit"
        || trimmed.starts_with(AUDIT_PREFIX)
        || trimmed.contains("/docs/audit/")
        || trimmed.ends_with("/docs/audit")
        || trimmed == "docs/history/audit"
        || trimmed.starts_with(HISTORY_AUDIT_PREFIX)
        || trimmed.contains("/docs/history/audit/")
        || trimmed.ends_with("/docs/history/audit")
        || is_plan_prism_path(trimmed)
}

fn try_scan_binary_metadata(path: &str, ext: &str, bytes: &[u8], sniff_magic: bool, hits: &mut Vec<BrandHit>) -> Result<bool, String> {
    if ext == "png" || (sniff_magic && bytes.starts_with(b"\x89PNG\r\n\x1a\n")) {
        hits.extend(check_png_metadata(path, bytes));
        return Ok(true);
    }
    if matches!(ext, "ico" | "icns") {
        hits.extend(check_icon_container_metadata(path, bytes));
        return Ok(true);
    }
    if ext == "gif" || (sniff_magic && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"))) {
        hits.extend(check_gif_metadata(path, bytes));
        return Ok(true);
    }
    if ext == "pdf" || bytes.starts_with(b"%PDF-") {
        hits.extend(check_pdf_metadata(path, bytes));
        return Ok(true);
    }
    if ext == "svgz" || (bytes.starts_with(&[0x1F, 0x8B]) && is_binary_asset_ext(path)) {
        let raw = inflate_gzip_bounded(bytes).map_err(|()| format!("cannot scan {path}: corrupt or oversized .svgz gzip payload"))?;
        let DecodedFile::Text(svg) = decode_text_file(path, &raw)? else {
            return Err(format!("cannot scan {path}: .svgz payload is not valid UTF-8/UTF-16 SVG text"));
        };
        scan_svg(path, &svg, hits);
        hits.extend(check_svg_metadata(path, &svg));
        return Ok(true);
    }
    Ok(false)
}

fn scan_repo_file_bytes(path: &str, bytes: &[u8], hits: &mut Vec<BrandHit>, scan_errors: &mut Vec<String>) {
    let ext_lower = lower_ext(path);
    match try_scan_binary_metadata(path, &ext_lower, bytes, false, hits) {
        Ok(true) => return,
        Ok(false) => {}
        Err(err) => {
            scan_errors.push(err);
            return;
        }
    }
    if is_zip_magic(bytes) || matches!(ext_lower.as_str(), "zip" | "curvebook" | "cbtemplate") {
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
    let path_policy = if norm == "crates/format/src/prism.rs" { PrismPolicy::AllowExactBarePrism } else { PrismPolicy::Strict };
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

fn skip_path(path: &str) -> bool {
    let norm = path.replace('\\', "/");
    let trimmed = norm.strip_prefix("./").unwrap_or(&norm);
    SELF_SCANNER_PATHS.contains(&trimmed)
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

fn is_readme_disclaimer_path(path: &str, allow_packaged: bool) -> bool {
    let norm = path.replace('\\', "/");
    let trimmed = norm.strip_prefix("./").unwrap_or(&norm);
    if trimmed == SUITE_DISCLAIMER_PATH {
        return true;
    }
    if !allow_packaged || is_generated_docs_html_path(trimmed) || trimmed.split(['/', '!', ':']).any(|seg| matches!(seg, "crates" | "apps")) {
        return false;
    }
    trimmed.rsplit(['/', ':', '!']).next() == Some(SUITE_DISCLAIMER_PATH)
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
    (trimmed.starts_with("docs-site/") && trimmed != "docs-site/manual/index.html")
        || trimmed.starts_with("target/docs-site/")
        || (trimmed.starts_with("target/docs-manual/") && trimmed != "target/docs-manual/index.html")
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
            collect_line_words(path, line_no, &masked, BRANDS, PrismPolicy::Strict, out);
        } else {
            collect_line_words(path, line_no, line, BRANDS, PrismPolicy::Strict, out);
        }
    }
}

fn scan_plain_lines_no_waiver(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    scan_plain_lines_with_scope(path, text, false, out);
}

fn scan_plain_lines_with_scope(path: &str, text: &str, allow_packaged: bool, out: &mut Vec<BrandHit>) {
    let is_readme_disclaimer = is_readme_disclaimer_path(path, allow_packaged);
    let is_gen_docs = is_generated_docs_html_path(path);
    let strict_disclaimer_brands: Vec<&'static str> =
        if is_readme_disclaimer { BRANDS.iter().copied().filter(|brand| !VENDOR_DISCLAIMER_NAMES.contains(brand)).collect() } else { Vec::new() };
    for (idx, line) in text.lines().enumerate() {
        let brands = if is_readme_disclaimer && line.contains(ALLOW_MARKER) { &strict_disclaimer_brands[..] } else { BRANDS };
        if is_gen_docs && let Some(masked) = mask_generated_docs_html_tokens(path, line) {
            collect_line_words(path, idx + 1, &masked, brands, PrismPolicy::Strict, out);
        } else {
            collect_line_words(path, idx + 1, line, brands, PrismPolicy::Strict, out);
        }
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
        collect_line_words(path, line_no, line, BRANDS, PrismPolicy::Strict, out);
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
        collect_line_words(path, line_no, line, BRANDS, PrismPolicy::Strict, out);
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
    let is_readme_disclaimer = is_readme_disclaimer_path(path, false);
    let strict_disclaimer_brands: Vec<&'static str> =
        if is_readme_disclaimer { BRANDS.iter().copied().filter(|brand| !VENDOR_DISCLAIMER_NAMES.contains(brand)).collect() } else { Vec::new() };
    for (idx, line) in text.lines().enumerate() {
        let line_no = idx + 1;
        let brands = if is_readme_disclaimer && line.contains(ALLOW_MARKER) { &strict_disclaimer_brands[..] } else { BRANDS };
        if let Some(masked) = mask_markdown_format_tokens(line, is_agents, is_dev_or_cmd) {
            collect_line_words(path, line_no, &masked, brands, PrismPolicy::Strict, out);
        } else {
            collect_line_words(path, line_no, line, brands, PrismPolicy::Strict, out);
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
                        text: decode_xml_html_entities(line.get(span_start..i).unwrap_or("")),
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
                    chunks.push(SvgChunk { text: decode_xml_html_entities(tag_chunk), is_meta_text: false, prev_word: None });
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
                chunks.push(SvgChunk { text: decode_xml_html_entities(tail), is_meta_text: false, prev_word: None });
            }
        }
        chunks
    }

    fn push_text_chunk(&mut self, line: &str, start: usize, end: usize, chunks: &mut Vec<SvgChunk>) {
        if start >= end {
            return;
        }
        let decoded = decode_xml_html_entities(line.get(start..end).unwrap_or(""));
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

    fn skip_to_eol(&mut self, consume_nl: bool) {
        while let Some(b) = self.peek() {
            if b == b'\n' {
                if consume_nl {
                    self.advance_one();
                }
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
                b'#' => self.skip_to_eol(false),
                _ => break,
            }
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
        self.skip_to_eol(true);
        self.in_workspace_section = segments.len() == 1 && segments[0] == "workspace";
        self.in_metadata_section = is_metadata_table_segments(&segments);
        if self.in_metadata_section {
            for lit in &key_lits {
                self.check_literal(lit);
            }
        }
    }

    fn read_statement(&mut self) {
        let (segments, key_lits) = self.read_dotted_key(false);
        if self.peek() != Some(b'=') {
            self.skip_to_eol(true);
            return;
        }
        self.pos += 1;
        let is_meta = self.in_metadata_section || is_metadata_key_segments(&segments, self.in_workspace_section);
        if is_meta {
            for lit in &key_lits {
                self.check_literal(lit);
            }
        }
        self.scan_value(is_meta);
    }

    fn read_dotted_key(&mut self, in_header: bool) -> (Vec<String>, Vec<ExtractedLiteral>) {
        let mut segments = Vec::new();
        let mut lits = Vec::new();
        while let Some(b) = self.peek() {
            let lit = match b {
                b'\n' | b'#' => break,
                b']' if in_header => break,
                b'=' if !in_header => break,
                b'.' | b' ' | b'\t' | b'\r' => {
                    self.pos += 1;
                    continue;
                }
                b'"' | b'\'' => self.read_string(b, false),
                _ => {
                    let tok = self.read_bare_token();
                    if tok.text.is_empty() {
                        continue;
                    }
                    tok
                }
            };
            segments.push(lit.text.clone());
            lits.push(lit);
        }
        (segments, lits)
    }

    fn scan_value(&mut self, is_meta: bool) {
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
                b'#' => self.skip_to_eol(false),
                b'[' | b'{' => {
                    depth += 1;
                    self.pos += 1;
                }
                b']' | b'}' => {
                    depth = depth.saturating_sub(1);
                    self.pos += 1;
                }
                b'"' | b'\'' => {
                    let multiline = self.bytes.get(self.pos..self.pos + 3) == Some(&[b, b, b]);
                    let lit = self.read_string(b, multiline);
                    if is_meta {
                        self.check_literal(&lit);
                    }
                }
                _ => {
                    let lit = self.read_bare_token();
                    if is_meta && !lit.text.is_empty() {
                        self.check_literal(&lit);
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

    fn read_string(&mut self, quote: u8, multiline: bool) -> ExtractedLiteral {
        let start_line = self.line;
        let is_basic = quote == b'"';
        let delim = [quote, quote, quote];
        self.pos += if multiline { 3 } else { 1 };
        if multiline {
            self.trim_opening_newline();
        }
        let mut text = String::new();
        let mut byte_lines = Vec::new();
        while let Some(b) = self.peek() {
            if multiline {
                if self.bytes.get(self.pos..self.pos + 3) == Some(&delim) {
                    self.consume_closing_triple(quote, &mut text, &mut byte_lines);
                    break;
                }
            } else if b == quote {
                self.pos += 1;
                break;
            } else if b == b'\n' {
                break;
            }
            if is_basic && b == b'\\' {
                self.read_toml_escape(&mut text, &mut byte_lines, multiline);
                continue;
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
        let line = self.line;
        for _ in 0..run.saturating_sub(3) {
            push_char_with_line(text, byte_lines, char::from(quote), line);
        }
        self.pos += run;
    }

    fn read_toml_escape(&mut self, text: &mut String, byte_lines: &mut Vec<usize>, multiline: bool) {
        let esc_line = self.line;
        self.pos += 1;
        let Some(next) = self.peek() else {
            return;
        };
        if multiline && is_line_ending_backslash(self.bytes, self.pos) {
            while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
                self.advance_one();
            }
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

    fn push_unicode_escape(&mut self, text: &mut String, byte_lines: &mut Vec<usize>, digits: usize, line: usize) {
        if let Some(hex) = self.src.get(self.pos..self.pos + digits)
            && let Ok(code) = u32::from_str_radix(hex, 16)
            && let Some(ch) = char::from_u32(code)
        {
            self.pos += digits;
            push_char_with_line(text, byte_lines, ch, line);
        }
    }

    fn check_literal(&mut self, lit: &ExtractedLiteral) {
        collect_literal_words(self.path, lit, BRANDS, PrismPolicy::Strict, &mut self.seen, self.out);
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
            let allow_import_prism_cmd = is_import_prism_cmd_literal(&lit.text);
            let allow_pzfx_xml = is_pzfx_xml_literal(&lit.text);
            let allow_it_statistica = is_italian_i18n_line(path, &lines, &lit);
            let allow_test_container_op = is_format_test_container_literal(path, &lit.text);
            let policy = if strict_ui_or_app {
                PrismPolicy::UiOrAppRustLiteral {
                    allow_bare_prism,
                    allow_import_prism_cmd,
                    allow_pzfx_xml,
                    allow_it_statistica,
                    allow_test_container_op,
                }
            } else {
                PrismPolicy::RustLiteral { allow_bare_prism, allow_import_prism_cmd, allow_pzfx_xml, allow_test_container_op }
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
    RustComment {
        allow_it_statistica: bool,
    },
    RustIdent {
        allow_dsl_jmp: bool,
        allow_canonical_prism_ident: bool,
    },
    RustLiteral {
        allow_bare_prism: bool,
        allow_import_prism_cmd: bool,
        allow_pzfx_xml: bool,
        allow_test_container_op: bool,
    },
    UiOrAppRustLiteral {
        allow_bare_prism: bool,
        allow_import_prism_cmd: bool,
        allow_pzfx_xml: bool,
        allow_it_statistica: bool,
        allow_test_container_op: bool,
    },
    UiTextRun {
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

/// Whether one `prism` match is allowed under `policy`.
///
/// The lowercase `.prism` file extension (`.prism`, `*.prism`, `experiment.prism`) names a file
/// format, not a product, so it is allowed under every policy, including [`PrismPolicy::Strict`]
/// surfaces such as page titles, meta descriptions, and UI text (maintainer decision 2026-11-24,
/// `R12.8`). Bare `prism`/`Prism`, capitalised `.Prism`, and domain-like `.prism.<tld>` stay flagged.
fn is_prism_match_exempt(raw: &str, lower: &str, pos: usize, end: usize, policy: PrismPolicy<'_>) -> bool {
    if raw.as_bytes().get(pos..end) == Some(b"prism") && is_prism_extension_token(lower.as_bytes(), pos, end) {
        return true;
    }
    let is_cmd_prism = matches!(raw.get(pos.saturating_sub(12)..end), Some("table.importPrism"))
        || matches!(raw.get(pos.saturating_sub(6)..end), Some("importPrism"));
    let is_xml_prism_tag = raw.get(pos..pos + 9) == Some("PrismFile") || raw.get(pos..pos + 15) == Some("PrismXMLVersion");
    match policy {
        PrismPolicy::Strict | PrismPolicy::RustComment { .. } => false,
        PrismPolicy::AllowExactBarePrism => raw.as_bytes().get(pos..end) == Some(b"prism"),
        PrismPolicy::RustIdent { allow_canonical_prism_ident, .. } => allow_canonical_prism_ident,
        PrismPolicy::RustLiteral { allow_bare_prism, allow_import_prism_cmd, allow_pzfx_xml, allow_test_container_op }
        | PrismPolicy::UiOrAppRustLiteral { allow_bare_prism, allow_import_prism_cmd, allow_pzfx_xml, allow_test_container_op, .. } => {
            (allow_bare_prism && raw == "prism")
                || (allow_import_prism_cmd && is_cmd_prism)
                || (allow_pzfx_xml && is_xml_prism_tag)
                || (allow_test_container_op && raw.as_bytes().get(pos..end) == Some(b"prism"))
        }
        PrismPolicy::UiTextRun { allow_palette_cmd_id, .. } => allow_palette_cmd_id && is_cmd_prism,
        PrismPolicy::SvgTitleOrDesc { prev_word } => is_preceded_by_optical_modifier(lower.get(..pos).unwrap_or(""), prev_word),
    }
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
    !followed_by_domain_segment && !is_url_host_segment(bytes, pos - 1)
}

fn is_url_host_segment(bytes: &[u8], dot_pos: usize) -> bool {
    let mut i = dot_pos;
    while i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || matches!(bytes[i - 1], b'-' | b'.')) {
        i -= 1;
    }
    i >= 3 && bytes.get(i - 3..i) == Some(b"://")
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

/// Scan a single UI/egui/AccessKit text run under `PrismPolicy::UiTextRun`.
pub fn check_ui_text_run(path: &str, line: usize, text: &str, allow_it_statistica: bool, allow_palette_cmd_id: bool, out: &mut Vec<BrandHit>) {
    let policy = PrismPolicy::UiTextRun { allow_it_statistica, allow_palette_cmd_id };
    check_single_string(path, line, text, policy, out);
}

/// Scan arbitrary UI, `egui_shell`, AccessKit, or tooltip `&str` slices for external
/// brands and suite names. Allows `.prism` / `*.prism` only inside explicit import/export
/// file-type descriptions.
#[must_use]
pub fn check_text_runs<I, S>(source: &str, runs: I) -> Vec<BrandHit>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out = Vec::new();
    for (idx, run) in runs.into_iter().enumerate() {
        let policy = PrismPolicy::UiTextRun { allow_it_statistica: false, allow_palette_cmd_id: false };
        check_single_string(source, idx + 1, run.as_ref(), policy, &mut out);
    }
    out
}

/// Strictly scan text runs, rejecting every external brand, suite name, and non-extension `"prism"` mention.
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
#[must_use]
pub fn check_labeled_text_runs<'a, I>(runs: I) -> Vec<BrandHit>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let mut out = Vec::new();
    for (idx, (label, text)) in runs.into_iter().enumerate() {
        let policy = PrismPolicy::UiTextRun { allow_it_statistica: false, allow_palette_cmd_id: false };
        check_single_string(label, idx + 1, text, policy, &mut out);
    }
    out
}

/// Scan `(key, locale, text)` i18n catalog entries across locales.
///
/// Rejects non-extension `"prism"` mentions everywhere, and permits lowercase Italian
/// `"statistica"` only inside `locale == "it"`.
#[must_use]
pub fn check_i18n_catalog_entries<'a, I>(entries: I) -> Vec<BrandHit>
where
    I: IntoIterator<Item = (&'a str, &'a str, &'a str)>,
{
    let mut out = Vec::new();
    for (key, locale, text) in entries {
        let policy = PrismPolicy::UiTextRun { allow_it_statistica: locale == "it", allow_palette_cmd_id: false };
        let path = format!("i18n:{locale}:{key}");
        check_single_string(&path, 1, text, policy, &mut out);
    }
    out
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

fn scan_xml_text_fragment(source: &str, start_line: usize, raw: &str, policy: PrismPolicy<'_>, out: &mut Vec<BrandHit>) {
    let decoded = decode_xml_html_entities(raw);
    for (offset, line_text) in decoded.lines().enumerate() {
        check_single_string_with_extra(source, start_line + offset, line_text, policy, true, out);
    }
}

fn consume_delimited_span<'a>(doc: &'a str, pos: usize, open: &str, close: &str) -> Option<(&'a str, usize)> {
    let rest = doc.get(pos..)?;
    let after = rest.strip_prefix(open)?;
    let body_start = pos + open.len();
    let (body, end) = match rest.find(close) {
        Some(rel) => (doc.get(body_start..(pos + rel).max(body_start)).unwrap_or(""), pos + rel + close.len()),
        None => (after, doc.len()),
    };
    Some((body, end))
}

fn next_markup_text_chunk<'a>(doc: &'a str, pos: usize, rest: &'a str) -> (&'a str, usize) {
    let next_lt = rest.find('<').map_or(doc.len(), |rel| pos + if rel == 0 { rest.chars().next().map_or(1, char::len_utf8) } else { rel });
    (doc.get(pos..next_lt).unwrap_or(""), next_lt)
}

/// Inspect rendered SVG export metadata (`<title>`, `<desc>`, `<metadata>`) and text elements
/// strictly against external brands and suite names.
#[must_use]
pub fn check_svg_metadata(source: &str, svg: &str) -> Vec<BrandHit> {
    let (mut out, mut line, mut pos, mut active_depth, mut skip_depth, mut active_start_line) = (Vec::new(), 1usize, 0usize, 0usize, 0usize, 1usize);
    let mut buf = String::new();

    while let Some(rest) = svg.get(pos..) {
        if rest.is_empty() {
            break;
        }
        if let Some((body, end)) = consume_delimited_span(svg, pos, "<!--", "-->") {
            if !body.trim().is_empty() {
                scan_xml_text_fragment(source, line, body, PrismPolicy::Strict, &mut out);
            }
            line += count_newlines(svg.get(pos..end).unwrap_or(rest).as_bytes());
            pos = end;
            continue;
        }
        if let Some((body, end)) = consume_delimited_span(svg, pos, "<![CDATA[", "]]>") {
            if skip_depth == 0 && active_depth > 0 {
                buf.push_str(body);
            }
            line += count_newlines(svg.get(pos..end).unwrap_or(rest).as_bytes());
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
            let is_active = matches!(lower.as_str(), "title" | "desc" | "metadata" | "text" | "tspan" | "textpath");
            let is_skip = matches!(lower.as_str(), "style" | "image");

            if !is_close && !is_skip {
                scan_svg_tag_attributes(source, line, raw_inner, &mut out);
            }
            if is_close {
                if is_skip {
                    skip_depth = skip_depth.saturating_sub(1);
                } else if is_active {
                    active_depth = active_depth.saturating_sub(1);
                    if active_depth == 0 && !buf.trim().is_empty() {
                        scan_xml_text_fragment(source, active_start_line, &buf, PrismPolicy::Strict, &mut out);
                        buf.clear();
                    }
                }
            } else if !self_closing {
                if is_skip {
                    skip_depth += 1;
                } else if is_active {
                    if active_depth == 0 {
                        active_start_line = line;
                        buf.clear();
                    }
                    active_depth += 1;
                }
            }
            line += count_newlines(svg.get(pos..tag_end).unwrap_or("").as_bytes());
            pos = tag_end;
            continue;
        }
        let (chunk, next_lt) = next_markup_text_chunk(svg, pos, rest);
        if skip_depth == 0 && active_depth > 0 {
            if buf.is_empty() {
                active_start_line = line;
            }
            buf.push_str(chunk);
        }
        line += count_newlines(chunk.as_bytes());
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
                return Some(decode_xml_html_entities(val));
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
    let (mut out, mut line, mut pos, mut strict_depth, mut in_script_or_style, mut text_start_line) =
        (Vec::new(), 1usize, 0usize, 0usize, false, 1usize);
    let mut buf = String::new();

    while let Some(rest) = html.get(pos..) {
        if rest.is_empty() {
            break;
        }
        if let Some((comment_inner, end)) = consume_delimited_span(html, pos, "<!--", "-->") {
            flush_html_text(source, line, comment_inner, true, &mut out);
            line += count_newlines(html.get(pos..end).unwrap_or(rest).as_bytes());
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
                line += count_newlines(chunk.as_bytes());
            }
            if let Some(rel) = close_pos {
                let tag_start = pos + rel;
                let gt_end = html.get(tag_start..).and_then(|s| s.find('>')).map_or(html.len(), |g| tag_start + g + 1);
                line += count_newlines(html.get(tag_start..gt_end).unwrap_or("").as_bytes());
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

            if matches!(lower.as_str(), "script" | "style") {
                if !is_close && !self_closing {
                    in_script_or_style = true;
                }
            } else if !is_close {
                let attrs: &[&str] = if lower == "meta" { &["content", "aria-label", "alt", "title"] } else { &["aria-label", "alt", "title"] };
                for attr in attrs {
                    if let Some(val) = extract_html_attr(raw_inner, attr) {
                        check_single_string_with_extra(source, line, &val, PrismPolicy::Strict, true, &mut out);
                    }
                }
                if matches!(lower.as_str(), "title" | "desc" | "metadata") {
                    strict_depth += 1;
                }
            } else if matches!(lower.as_str(), "title" | "desc" | "metadata") {
                strict_depth = strict_depth.saturating_sub(1);
            }
            line += count_newlines(html.get(pos..tag_end).unwrap_or("").as_bytes());
            pos = tag_end;
            continue;
        }
        let (text_chunk, next_lt) = next_markup_text_chunk(html, pos, rest);
        if buf.is_empty() {
            text_start_line = line;
        }
        buf.push_str(text_chunk);
        line += count_newlines(text_chunk.as_bytes());
        pos = next_lt;
    }

    if !buf.trim().is_empty() {
        flush_html_text(source, text_start_line, &buf, strict_depth > 0, &mut out);
    }
    out
}

fn flush_html_text(source: &str, start_line: usize, raw: &str, strict: bool, out: &mut Vec<BrandHit>) {
    let decoded = decode_xml_html_entities(raw);
    let gen_docs = !strict && is_generated_docs_html_path(source);
    for (offset, line_text) in decoded.lines().enumerate() {
        let masked_opt = if gen_docs { mask_generated_docs_html_tokens(source, line_text) } else { None };
        let scanned = masked_opt.as_deref().unwrap_or(line_text);
        check_single_string_with_extra(source, start_line + offset, scanned, PrismPolicy::Strict, strict, out);
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
#[must_use]
pub fn check_window_title(source: &str, title: &str) -> Vec<BrandHit> {
    let mut out = Vec::new();
    check_single_string(source, 1, title, PrismPolicy::Strict, &mut out);
    out
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
/// product, retired-suite, or pre-rename tokens while preserving the narrow `.prism`/`.pzfx`
/// file-format boundary on `open_file` and `table_import_prism` / `table.importPrism`.
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
        check_single_string_with_extra(&key_source, idx + 1, k, PrismPolicy::Strict, true, out);
        scan_mcp_json_value(child_ctx, &format!("{ptr}/{k}"), v, out);
    }
}

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

fn mcp_leaf_prism_policy<'a>(ctx: McpScanCtx<'_>, ptr: &str, text: &str) -> PrismPolicy<'a> {
    if ctx.method == "tools/list" {
        if ctx.entry_tool_name == Some("open_file") {
            if ptr.contains("/inputSchema/properties/format/enum/") && text == "prism" {
                return PrismPolicy::AllowExactBarePrism;
            }
            return PrismPolicy::Strict;
        }
        if ctx.entry_tool_name == Some("table_import_prism") && is_top_tool_field_ptr(ptr, "description") {
            return PrismPolicy::UiTextRun { allow_it_statistica: false, allow_palette_cmd_id: true };
        }
        return PrismPolicy::Strict;
    }

    if ctx.method == "tools/call"
        && matches!(ctx.called_tool, Some("list_toolsets" | "command_list" | "ui_inspect"))
        && (ptr.ends_with("/command_id") || ptr.ends_with("/id") || ptr.ends_with("/widget_id"))
        && (text == "table.importPrism" || text.ends_with(".table.importPrism"))
    {
        return PrismPolicy::UiTextRun { allow_it_statistica: false, allow_palette_cmd_id: true };
    }

    PrismPolicy::Strict
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

const MAX_DIST_FILE_BYTES: usize = 256 * 1024 * 1024;

fn scan_strict_label_words(source: &str, text: &str, include_adapted: bool, hits: &mut Vec<BrandHit>) {
    collect_line_words(source, 1, text, SUITE_BRANDS, PrismPolicy::Strict, hits);
    if include_adapted {
        collect_line_words(source, 1, text, &[ADAPTED_UPSTREAM_BRAND], PrismPolicy::Strict, hits);
    }
    collect_line_words(source, 1, text, BRANDS, PrismPolicy::Strict, hits);
    collect_line_words(source, 1, text, PRE_RENAME_ASSET_BRANDS, PrismPolicy::Strict, hits);
}

fn scan_dist_path_and_file(dist_dir: &Path, rel: &str, stats: &mut DistScanStats, hits: &mut Vec<BrandHit>) -> Result<(), String> {
    if is_forbidden_bundled_dir(rel) {
        hits.push(BrandHit { path: rel.to_owned(), line: 1, brand: "prism" });
        return Ok(());
    }
    scan_strict_label_words(rel, rel, true, hits);
    let bytes = curvebook_platform::fs::read_regular_file_bytes(&dist_dir.join(rel), MAX_DIST_FILE_BYTES)
        .map_err(|e| format!("read dist file {rel}: {e}"))?;
    scan_dist_bytes(rel, &bytes, 0, stats, hits)
}

const MAX_MACOS_STAPLED_TICKET_BYTES: usize = 1024 * 1024;

fn is_macos_stapled_ticket(display_path: &str, bytes: &[u8]) -> bool {
    if bytes.len() < 12 || bytes.len() > MAX_MACOS_STAPLED_TICKET_BYTES || !bytes.starts_with(b"s8ch") {
        return false;
    }
    let header_window = &bytes[8..bytes.len().min(64)];
    if !header_window.windows(4).any(|w| w[0] == 0x30 && w[1] == 0x82 && (w[2] != 0 || w[3] != 0)) {
        return false;
    }
    let norm = display_path.replace('\\', "/");
    let inner_path = if let Some((outer, inner)) = norm.split_once('!') {
        let outer_lower = outer.to_ascii_lowercase();
        if inner.contains('!') || !outer_lower.ends_with(".dmg") || outer_lower == ".dmg" || outer_lower.ends_with("/.dmg") {
            return false;
        }
        inner
    } else {
        norm.as_str()
    };
    let segs: Vec<&str> = inner_path.split('/').filter(|s| !s.is_empty()).collect();
    let n = segs.len();
    n >= 3
        && !segs.contains(&"..")
        && segs[n - 1] == "CodeResources"
        && segs[n - 2] == "Contents"
        && segs[n - 3].strip_suffix(".app").is_some_and(|stem| !stem.is_empty() && stem != "." && stem != "..")
}

fn is_native_binary_blob(display_path: &str, bytes: &[u8]) -> bool {
    let ext = lower_ext(display_path);
    matches!(ext.as_str(), "exe" | "dll" | "so" | "dylib" | "wasm" | "deb" | "rpm" | "dmg" | "msi" | "appimage" | "zsync" | "a" | "lib")
        || is_macos_stapled_ticket(display_path, bytes)
        || [
            &b"\x7FELF"[..],
            b"\x00asm",
            b"MZ",
            b"!<arch>\n",
            b"\xED\xAB\xEE\xDB",
            b"\xFE\xED\xFA\xCE",
            b"\xFE\xED\xFA\xCF",
            b"\xCE\xFA\xED\xFE",
            b"\xCF\xFA\xED\xFE",
            b"\xCA\xFE\xBA\xBE",
            b"\xBE\xBA\xFE\xCA",
        ]
        .iter()
        .any(|sig| bytes.starts_with(sig))
}

fn scan_dist_bytes(display_path: &str, bytes: &[u8], depth: usize, stats: &mut DistScanStats, hits: &mut Vec<BrandHit>) -> Result<(), String> {
    let ext = lower_ext(display_path);
    let lower_path = display_path.to_ascii_lowercase();
    if is_zip_magic(bytes) {
        if depth >= 2 {
            return Err(format!("{display_path}: nested archive depth exceeded (max depth 2)"));
        }
        let zip =
            ZipArchive::from_bytes_with_limits(bytes, ZipLimits::default()).map_err(|e| format!("cannot inspect archive `{display_path}`: {e}"))?;
        for (name, entry_bytes) in zip.iter() {
            stats.archive_entries += 1;
            let entry_path = format!("{display_path}!{name}");
            scan_strict_label_words(&entry_path, name, true, hits);
            scan_dist_bytes(&entry_path, entry_bytes, depth + 1, stats, hits)?;
        }
        return Ok(());
    }
    if try_scan_binary_metadata(display_path, &ext, bytes, true, hits)? {
        return Ok(());
    }
    let text = match decode_text_file(display_path, bytes) {
        Ok(DecodedFile::Text(s)) => s,
        Ok(DecodedFile::BinaryAsset) => return Ok(()),
        Err(err) => return if is_native_binary_blob(display_path, bytes) { Ok(()) } else { Err(err) },
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
        scan_plain_lines_with_scope(display_path, &text, true, hits);
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
    scan_strict_label_words(source, label, true, &mut hits);
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
    if matches!(lower_ext(display_path).as_str(), "html" | "htm") {
        hits.extend(check_html_metadata(display_path, text));
    }
    dedup_tail_hits(&mut hits, from);
    hits
}

/// Scan a generated static documentation site directory under the narrow generated-HTML
/// format-boundary policy while keeping `PrismPolicy::Strict` on non-HTML text and suite brands.
#[cfg(feature = "docs-site")]
pub fn check_docs_site_dir(dir: &Path) -> Result<Vec<BrandHit>, String> {
    check_generated_docs_dir(dir, "docs-site")
}

/// Scan a generated manual build directory (`index.html`, `latest/`, `v<major>.<minor>/`) under
/// `docs-site/manual/...` so redirect `index.html` stays strict while manual pages get the narrow
/// generated-HTML format-boundary policy (`R12.10`).
#[cfg(feature = "docs-site")]
pub fn check_manual_build_dir(dir: &Path) -> Result<Vec<BrandHit>, String> {
    check_generated_docs_dir(dir, "docs-site/manual")
}

#[cfg(feature = "docs-site")]
fn check_generated_docs_dir(dir: &Path, display_prefix: &str) -> Result<Vec<BrandHit>, String> {
    if !dir.exists() {
        return Err(format!("docs-site directory `{}` does not exist", dir.display()));
    }
    let mut rel_files = Vec::new();
    collect_dist_files(dir, dir, &mut rel_files)?;
    rel_files.sort();
    let mut stats = DistScanStats::default();
    let mut hits = Vec::new();
    for rel in rel_files {
        let display_path = format!("{display_prefix}/{rel}");
        stats.files += 1;
        scan_strict_label_words(&display_path, &rel, false, &mut hits);
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
    use super::{
        BrandHit, check_html_metadata, check_i18n_catalog_entries, check_labeled_text_runs, check_manifest_metadata, check_pdf_metadata,
        check_svg_metadata, check_text_runs, check_text_runs_strict, check_window_title, run, scan_file, scan_repo_checked, skip_path,
    };
    use crate::scan_util::test_support::{TestDir, to_utf16_bytes};

    fn hit(path: &str, line: usize, brand: &'static str) -> BrandHit {
        BrandHit { path: path.to_owned(), line, brand }
    }

    fn scan(path: &str, text: &str) -> Vec<BrandHit> {
        let mut hits = Vec::new();
        scan_file(path, text, &mut hits);
        hits
    }

    fn hits_for(path: &str, expected: &[(usize, &'static str)]) -> Vec<BrandHit> {
        expected.iter().map(|&(line, brand)| hit(path, line, brand)).collect()
    }

    fn build_test_png_with_chunk(chunk_type: &[u8; 4], chunk_data: &[u8]) -> Vec<u8> {
        let mut png = Vec::from(&b"\x89PNG\r\n\x1a\n"[..]);
        for (ctype, data) in [(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0][..]), (chunk_type, chunk_data), (b"IEND", &[][..])] {
            png.extend_from_slice(&(data.len() as u32).to_be_bytes());
            png.extend_from_slice(ctype);
            png.extend_from_slice(data);
            png.extend_from_slice(&0u32.to_be_bytes());
        }
        png
    }

    #[test]
    fn r15_1_provenance_permits_only_exact_upstream_url_inside_attributed_source_section() {
        let story = ["sto", "ry", "told"].concat();
        let vc = ["vec", "tor", "craft"].concat();
        let vc_title = ["Vec", "tor", "Cra", "ft"].concat();
        let exact_url = format!("https://github.com/{story}/{vc}");

        for (path, body) in [
            (
                "ATTRIBUTION.md",
                format!("# Attribution\n\n## Attributed Source Code\n\nAdapted helpers from {vc_title} (`{exact_url}`) at commit `deadbeef`.\n"),
            ),
            (
                "NOTICE",
                format!(
                    "CurveBook\n\n## Attributed Source Code\n\nFrom {vc_title} ({exact_url}, commit abc).\nCopyright (c) 2026 {vc_title} contributors.\n"
                ),
            ),
            ("AGENTS.md", format!("Attributed {vc_title} GUI source reuse rules.\n")),
        ] {
            let hits = scan(path, &body);
            assert!(hits.is_empty(), "expected clean {path}, got {hits:?}");
        }
        let mut pkg_hits = Vec::new();
        super::scan_suite_lines_with_scope(
            "curvebook-web-0.1.0/NOTICE",
            &format!("## Attributed Source Code\nFrom {vc_title} ({exact_url}).\n"),
            true,
            &mut pkg_hits,
        );
        assert!(pkg_hits.is_empty(), "{pkg_hits:?}");

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
            let hits = scan(path, &bad_body);
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
            let hits = scan(path, &body);
            assert!(hits.iter().any(|h| h.brand == "vectorcraft"), "expected vectorcraft rejection for {path}, got {hits:?}");
        }
        let mut path_hits = Vec::new();
        super::scan_tracked_path_name(&format!("crates/ui-egui/src/{vc}_panel.rs"), &mut path_hits);
        assert!(path_hits.iter().any(|h| h.brand == "vectorcraft"), "{path_hits:?}");
        assert!(check_text_runs("ui_label", [format!("Exported with {vc_title}")]).iter().any(|h| h.brand == "vectorcraft"));
    }

    #[test]
    fn scans_rust_sources_comments_literals_cfg_test_escapes_and_prism_boundaries() {
        type ExpectedHit = (usize, &'static str);
        let cases: &[(&str, &str, &[ExpectedHit])] = &[
            (
                "crates/ui-egui/src/lib.rs",
                "// GraphPad comment is also caught\npub const LABEL: &str = \"Import GraphPad table\";\npub const NO_WAIVER: &str = \"Import OriginPro table\"; // brand-ok\npub fn instantiate() -> &'static str { \"instantiate\" }\npub struct PrismStyle;\n#[cfg(test)]\nmod tests {\n    const T: &str = \"SPSS fixture\";\n}\n",
                &[(1, "graphpad"), (2, "graphpad"), (3, "originpro"), (5, "prism"), (8, "spss")],
            ),
            (
                "apps/curvebook/src/main.rs",
                "#[cfg(test)]\nfn test_only_helper() {\n    let _ = \"SPSS fixture }; // brace inside string\";\n}\n\npub fn shipped_error() -> &'static str {\n    let q = '\"';\n    let _ = q;\n    \"GraphPad-compatible: unknown option\"\n}\n\n#[cfg(all(test, unix))]\nfn unix_test_helper() {\n    let _ = \"Minitab\";\n}\n\n#[cfg(not(test))]\npub const PROD: &str = \"Dotmatics\";\n",
                &[(3, "spss"), (9, "graphpad"), (14, "minitab"), (18, "dotmatics")],
            ),
            (
                "crates/format/src/lib.rs",
                "pub const CONT: &str = \"Import from \\\n    GraphPad table\";\npub const SPLIT: &str = \"Import Graph\\\nPad table\";\npub const ESC_NL: &str = \"line1\\nOriginPro\";\npub const RAW: &str = r#\"hello\nMinitab\"#;\npub const NO_WAIVER: &str = \"Import from \\\n    SigmaPlot table\"; // brand-ok\n",
                &[(2, "graphpad"), (3, "graphpad"), (5, "originpro"), (7, "minitab"), (9, "sigmaplot")],
            ),
            (
                "crates/format/src/registry.rs",
                "\npub fn ext_match(ext: &str) -> &'static str {\n    match ext {\n        \"prism\" | \"pzfx\" => \"project.prism\",\n        _ => \"other\",\n    }\n}\npub const MSG_OK: &str = \"Import *.prism or table.importPrism files\";\npub const MSG_BRAND: &str = \"Import .prism or Prism project\";\npub const LABEL_BAD: &str = \"Prism Archive\";\npub const DOMAIN_BAD: &str = \"https://www.prism.com/download\";\npub const LABEL_NO_WAIVER: &str = \"Prism Archive\"; // brand-ok\n#[cfg(test)]\nmod tests {\n    const TEST_FIXTURE: &str = \"GraphPad study\";\n}\n",
                &[(9, "prism"), (10, "prism"), (11, "prism"), (12, "prism"), (15, "graphpad")],
            ),
            (
                "crates/ui-egui/src/dialogs_canvas.rs",
                "\npub fn dialog_strings() {\n    let ok = (\"CurveBook\", \"curvebook\", \"approved-seven-pane\", \"curvebook-win95-2508\", \"Import (*.curvebook, *.pzfx, *.prism)\");\n    let bad_bare = \"prism\";\n    let ok_sentence = \"Switch to .prism for layout\";\n    let bad_vendor = \"GraphPad\";\n    let bad_suite = \"ArtCraft\";\n    let bad_cap = \"Prism\";\n    let _ = (ok, bad_bare, ok_sentence, bad_vendor, bad_suite, bad_cap);\n}\n",
                &[(7, "artcraft"), (4, "prism"), (6, "graphpad"), (8, "prism")],
            ),
            (
                "apps/curvebook-web/src/lib.rs",
                "\npub fn detect_ext(ext: Option<&str>) -> &'static str {\n    match ext { Some(\"prism\") => \"external\", _ => \"native\" }\n}\npub fn add_filters(d: Dummy) { d.add_filter(\"Supported\", &[\"curvebook\", \"pzfx\", \"prism\"]); }\npub fn status_line() -> &'static str { \"prism\" }\n",
                &[(6, "prism")],
            ),
            (
                "crates/ui-egui/src/theme.rs",
                "\npub struct PrismStyle { pub like_prism: bool }\npub fn graphpad_theme() {}\npub fn xlstatHelper() {}\npub fn StatisticaExport() {}\npub fn statistical_analysis() {}\npub fn origin_pro_preset() {}\n",
                &[(2, "prism"), (3, "graphpad"), (4, "xlstat"), (5, "statistica"), (7, "originpro")],
            ),
            (
                "crates/ui-egui/src/i18n.rs",
                "\nen: \"Copy methods and statistics report\",\nit: \"Copia report metodi e statistica\",\nde: \"Statistica kopieren\",\nit: \"Copia Statistica report\",\n",
                &[(4, "statistica"), (5, "statistica")],
            ),
            (
                "crates/fit/src/dsl.rs",
                "struct Compile { jmp_end: usize }\nconst BAD: &str = \"jmp\";\nfn JmpImporter() {}\n",
                &[(2, "jmp"), (3, "jmp")],
            ),
        ];
        for &(path, src, expected) in cases {
            assert_eq!(scan(path, src), hits_for(path, expected), "case {path}");
        }
    }

    #[test]
    fn scans_non_rust_files_disclaimers_svg_desktop_toml_and_issue_templates() {
        assert_eq!(
            scan(
                "packaging/linux/org.curvebook.CurveBook.desktop",
                "[Desktop Entry]\nName=CurveBook\nMimeType=application/vnd.curvebook.project;application/x-prism-project;application/x-prism-xml;\n# Compatibility: brand-ok\nKeywords=biostatistics;regression;GraphPad;prism;\n"
            ),
            hits_for("packaging/linux/org.curvebook.CurveBook.desktop", &[(5, "graphpad"), (5, "prism")])
        );
        assert_eq!(
            scan("docs/release/releasing.md", "Installs under Programs\\ArtCraft\\CurveBook or https://getartcraft.com\n"),
            hits_for("docs/release/releasing.md", &[(1, "artcraft"), (1, "getartcraft")])
        );

        // README disclaimers (allowed only with marker and allowed names).
        for ok in ["Not affiliated with ArtCraft. <!-- brand-ok -->\n", "Not affiliated with Dotmatics or GraphPad Software. <!-- brand-ok -->\n"] {
            assert!(scan("README.md", ok).is_empty(), "{ok}");
        }
        let mut dist_hits = Vec::new();
        super::scan_dist_bytes(
            "curvebook-0.1.0-linux-x86_64/README.md",
            b"Not affiliated with Dotmatics or GraphPad Software. <!-- brand-ok -->\n",
            0,
            &mut super::DistScanStats::default(),
            &mut dist_hits,
        )
        .unwrap();
        assert!(dist_hits.is_empty(), "{dist_hits:?}");

        for (path, line, expected) in [
            ("README.md", "Not affiliated with Dotmatics or GraphPad.\n", &[(1, "graphpad"), (1, "dotmatics")][..]),
            ("docs/release/releasing.md", "Not affiliated with Dotmatics or GraphPad. <!-- brand-ok -->\n", &[(1, "graphpad"), (1, "dotmatics")]),
            ("README.md", "Not affiliated with OriginPro or SPSS. <!-- brand-ok -->\n", &[(1, "spss"), (1, "originpro")]),
            ("README.md", "Part of ArtCraft.\n", &[(1, "artcraft")]),
            ("docs/release/releasing.md", "Not affiliated with ArtCraft. <!-- brand-ok -->\n", &[(1, "artcraft")]),
            ("README.md", "See https://getartcraft.com and craft-ui. <!-- brand-ok -->\n", &[(1, "getartcraft"), (1, "craft-ui")]),
        ] {
            assert_eq!(scan(path, line), hits_for(path, expected), "{path}: {line}");
        }

        assert_eq!(
            scan(
                "assets/app-icon/curvebook.svg",
                "<svg xmlns=\"http://www.w3.org/2000/svg\">\n  <title>CurveBook icon — Mole holding a refractive prism</title>\n  <desc>Clean geometric vector icon holding a glowing triangular optical\n  prism dispersing light into a spectrum.</desc>\n  <title>Prism Scientific Studio</title>\n  <text x=\"0\" y=\"10\">optical prism</text>\n  <!-- Compatibility icon label: brand-ok -->\n  <title>Prism Archive</title>\n</svg>\n"
            ),
            hits_for("assets/app-icon/curvebook.svg", &[(5, "prism"), (6, "prism"), (8, "prism")])
        );

        assert_eq!(
            scan(
                ".github/ISSUE_TEMPLATE/numerical_result.yml",
                "name: Numerical discrepancy\nbody:\n  - type: markdown\n    attributes:\n      value: |\n        Do not attach screenshots from Prism or OriginPro.\n        Export `.prism` cleanly instead.\n        # brand-ok compatibility mention: Prism\n"
            ),
            hits_for(".github/ISSUE_TEMPLATE/numerical_result.yml", &[(6, "prism"), (6, "originpro"), (8, "prism")])
        );
        assert_eq!(
            scan(
                "apps/curvebook-web/index.html",
                "<!doctype html>\n<html>\n  <head><title>CurveBook</title></head>\n  <body>\n    <input accept=\".curvebook,.cbtemplate,.prism,.pzfx\">\n    <p>Drop a Prism file here.</p>\n    <p>Drop a Prism file here.</p> <!-- brand-ok -->\n  </body>\n</html>\n"
            ),
            hits_for("apps/curvebook-web/index.html", &[(6, "prism"), (7, "prism")])
        );
        assert_eq!(
            scan(
                "apps/curvebook-web/src/lib.rs",
                "#[cfg(test)]\nmod tests {\n    const FIXTURE: &str = \"Prism\";\n}\npub fn prod_format() -> &'static str { \"prism\" }\n"
            ),
            hits_for("apps/curvebook-web/src/lib.rs", &[(3, "prism"), (5, "prism")])
        );

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
        assert_eq!(
            scan("crates/demo/Cargo.toml", toml),
            hits_for(
                "crates/demo/Cargo.toml",
                &[
                    (2, "spss"),
                    (8, "graphpad"),
                    (9, "prism"),
                    (12, "prism"),
                    (13, "originpro"),
                    (15, "prism"),
                    (18, "prism"),
                    (21, "prism"),
                    (25, "jmp"),
                    (27, "prism"),
                    (33, "minitab"),
                    (37, "systat"),
                    (40, "prism"),
                ]
            )
        );
    }

    #[test]
    fn decodes_utf16_files_validates_paths_and_fails_on_non_utf8_text() {
        let dir = TestDir::new("brands-encoding");
        dir.write_bytes("packaging/windows/app.rc", &to_utf16_bytes("VALUE \"Product\", \"GraphPad\"\n", true, true));
        dir.write_bytes("packaging/windows/clean.rc", &to_utf16_bytes("VALUE \"Product\", \"CurveBook\"\n", false, true));
        dir.write_bytes("assets/icon.png", &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0xFF]);
        dir.write("crates/format/src/prism.rs", "// clean .prism module\n");
        dir.write("crates/engine/tests/bad_guard_leak.rs", "const LEAK: &str = \"Import from Prism\";\n");
        dir.write("crates/ui-egui/src/PrismStyle.rs", "pub struct Clean;\n");

        let files: Vec<String> = [
            "packaging/windows/app.rc",
            "packaging/windows/clean.rc",
            "assets/icon.png",
            "crates/format/src/prism.rs",
            "crates/engine/tests/bad_guard_leak.rs",
            "crates/ui-egui/src/PrismStyle.rs",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        assert_eq!(
            scan_repo_checked(dir.path(), &files).unwrap(),
            vec![
                hit("packaging/windows/app.rc", 1, "graphpad"),
                hit("crates/engine/tests/bad_guard_leak.rs", 1, "prism"),
                hit("crates/ui-egui/src/PrismStyle.rs", 1, "prism"),
            ]
        );

        dir.write_bytes("packaging/linux/latin1.desktop", b"[Desktop Entry]\nComment=\xa9 CurveBook\n");
        let err = scan_repo_checked(dir.path(), &[String::from("packaging/linux/latin1.desktop")]).unwrap_err();
        assert!(err.contains("cannot scan packaging/linux/latin1.desktop"), "{err}");
        let _ = run;
    }

    #[test]
    fn checks_i18n_text_runs_exports_window_title_and_binary_metadata() {
        assert_eq!(
            check_i18n_catalog_entries([
                ("menu.file.open", "en", "CurveBook Project"),
                ("dialog.open.file_filter", "de", "Data files (*.curvebook, *.pzfx, *.prism)"),
                ("menu.help.about", "fr", "Built for GraphPad users"),
                ("cmd.table.importPrism", "ja", "Prism からインポート"),
                ("toolbar.export", "ar", "Export prism archive"),
                ("dialog.open.file_filter", "es", "Prism files (*.prism)"),
                ("inspector.title", "ko", "ArtCraft Inspector"),
            ]),
            vec![
                hit("i18n:fr:menu.help.about", 1, "graphpad"),
                hit("i18n:ja:cmd.table.importPrism", 1, "prism"),
                hit("i18n:ar:toolbar.export", 1, "prism"),
                hit("i18n:es:dialog.open.file_filter", 1, "prism"),
                hit("i18n:ko:inspector.title", 1, "artcraft"),
            ]
        );

        assert_eq!(
            check_text_runs(
                "layout_tree",
                [
                    "CurveBook 1.0",
                    "approved-seven-pane",
                    "Import .pzfx / .prism archive",
                    "Welcome to prism",
                    "Powered by OriginPro and GraphPad",
                    "Open Prism workspace"
                ]
            ),
            hits_for("layout_tree", &[(4, "prism"), (5, "graphpad"), (5, "originpro"), (6, "prism")])
        );
        assert_eq!(
            check_text_runs_strict("window_labels", ["CurveBook", "Import .prism archive", "Import Prism archive"]),
            vec![hit("window_labels", 3, "prism")]
        );
        assert_eq!(
            check_labeled_text_runs([
                ("node#1", "CurveBook Canvas"),
                ("node#2", "File formats: *.pzfx, *.prism"),
                ("node#3", "Open in OriginPro"),
                ("node#4", "prism mode"),
            ]),
            vec![hit("node#3", 3, "originpro"), hit("node#4", 4, "prism")]
        );

        assert!(
            check_svg_metadata(
                "clean.svg",
                r#"<svg><title>CurveBook Chart</title><desc>Dose curve</desc><metadata>CurveBook</metadata><text>Group A</text></svg>"#
            )
            .is_empty()
        );
        assert_eq!(
            check_svg_metadata(
                "dirty.svg",
                "<svg>\n<title>Prism &amp; OriginPro</title>\n<desc>prism plot</desc>\n<metadata><author>GraphPad</author></metadata>\n</svg>"
            ),
            hits_for("dirty.svg", &[(2, "prism"), (2, "originpro"), (3, "prism"), (4, "graphpad")])
        );

        assert!(
            check_pdf_metadata("clean.pdf", b"%PDF-1.4\n1 0 obj\n<< /Title (CurveBook) /Creator (CurveBook) /Producer (CurveBook) >>\nendobj\n")
                .is_empty()
        );
        assert_eq!(
            check_pdf_metadata(
                "dirty.pdf",
                b"%PDF-1.4\n1 0 obj\n<< /Creator (Prism) /Producer (ArtCraft) /Title <FEFF00470072006100700068005000610064> >>\nendobj\n"
            ),
            hits_for("dirty.pdf", &[(3, "prism"), (3, "artcraft"), (3, "graphpad")])
        );

        assert!(check_html_metadata("clean.html", "<!doctype html>\n<html>\n<head><title>CurveBook</title><meta name=\"generator\" content=\"CurveBook\"></head>\n<body><input accept=\".curvebook,.cbtemplate,.prism,.pzfx\"></body>\n</html>").is_empty());
        assert_eq!(
            check_html_metadata(
                "dirty.html",
                "<html>\n<head><title>Prism</title><meta name=\"author\" content=\"OriginLab\"></head>\n<body aria-label=\"GraphPad workspace\"></body>\n</html>"
            ),
            hits_for("dirty.html", &[(2, "prism"), (2, "originlab"), (3, "graphpad")])
        );

        assert!(check_manifest_metadata("manifest.json", r#"{"app":"CurveBook","schema_version":1,"title":"Dose-Response Study"}"#).is_empty());
        assert_eq!(
            check_manifest_metadata("manifest.json", r#"{"app":"CurveBook","format":".prism","generator":"Prism","vendor":"GraphPad"}"#),
            hits_for("manifest.json", &[(3, "prism"), (4, "graphpad")])
        );

        assert!(check_window_title("desktop_host", "CurveBook — Study 1").is_empty());
        assert_eq!(check_window_title("desktop_host", "CurveBook (prism)"), vec![hit("desktop_host", 1, "prism")]);

        // PNG tEXt / iTXt / zTXt + dist bundle
        assert_eq!(
            super::check_png_metadata("icon.png", &build_test_png_with_chunk(b"tEXt", b"svg:title\x00   LumenCraft logo")),
            vec![hit("icon.png", 2, "lumencraft")]
        );
        assert_eq!(
            super::check_png_metadata("icon2.png", &build_test_png_with_chunk(b"iTXt", b"Description\x00\x00\x00en\x00\x00Built with OriginPro")),
            vec![hit("icon2.png", 2, "originpro")]
        );
        let payload = b"Prism Art";
        let len = payload.len() as u16;
        let mut ztxt = Vec::from(&b"Comment\x00\x00\x78\x9c\x01"[..]);
        ztxt.extend_from_slice(&len.to_le_bytes());
        ztxt.extend_from_slice(&(!len).to_le_bytes());
        ztxt.extend_from_slice(payload);
        ztxt.extend_from_slice(&[0, 0, 0, 1]);
        assert_eq!(super::check_png_metadata("icon3.png", &build_test_png_with_chunk(b"zTXt", &ztxt)), vec![hit("icon3.png", 2, "prism")]);
        let clean_png = build_test_png_with_chunk(b"tEXt", b"svg:title\x00CurveBook seven-pane logo");
        assert!(super::check_png_metadata("clean.png", &clean_png).is_empty());

        let dist = TestDir::new("brands-dist");
        dist.write("CurveBook.app/Contents/Info.plist", "<dict><key>CFBundleName</key><string>CurveBook</string></dict>\n");
        dist.write_bytes("CurveBook.app/Contents/Resources/app.png", &clean_png);
        dist.write("web/index.html", "<title>CurveBook</title><input accept=\".curvebook,.pzfx,.prism\">\n");
        assert!(super::scan_dist_artefacts(dist.path()).unwrap().is_empty());
        dist.write("docs/audit/leaked.md", "leak");
        dist.write("web/PrismStyle.css", "/* Prism theme */");
        let bad = super::scan_dist_artefacts(dist.path()).unwrap();
        assert!(bad.iter().any(|h| h.path == "docs/audit/leaked.md"), "{bad:?}");
        assert!(bad.iter().any(|h| h.path == "web/PrismStyle.css" && h.brand == "prism"), "{bad:?}");

        // ICNS & GIF metadata
        let mk_icns = |body: &[u8]| {
            let chunk_len = (8 + body.len()) as u32;
            let mut v = Vec::from(&b"icns"[..]);
            v.extend_from_slice(&(8 + chunk_len).to_be_bytes());
            v.extend_from_slice(b"info");
            v.extend_from_slice(&chunk_len.to_be_bytes());
            v.extend_from_slice(body);
            v
        };
        assert!(super::check_icon_container_metadata("clean.icns", &mk_icns(b"bplist00icon_assetcatalog")).is_empty());
        let icns_hits = super::check_icon_container_metadata("dirty.icns", &mk_icns(b"bplist00 LumenCraft OriginPro icon"));
        assert!(icns_hits.iter().any(|h| h.brand == "lumencraft") && icns_hits.iter().any(|h| h.brand == "originpro"), "{icns_hits:?}");

        let mut clean_gif = Vec::from(&b"GIF89a\x01\x00\x01\x00\x00\x00\x00\x21\xff\x0bNETSCAPE2.0\x03\x01\x00\x00\x00\x3b"[..]);
        assert!(super::check_gif_metadata("clean.gif", &clean_gif).is_empty());
        clean_gif.truncate(13);
        let comment = b"Created in GraphPad";
        clean_gif.extend_from_slice(&[0x21, 0xfe, comment.len() as u8]);
        clean_gif.extend_from_slice(comment);
        clean_gif.extend_from_slice(&[0x00, 0x3b]);
        assert_eq!(super::check_gif_metadata("dirty.gif", &clean_gif), vec![hit("dirty.gif#gif_ext1", 1, "graphpad")]);
    }

    #[test]
    fn r021_repo_run_positive_and_negative_regression_suite() {
        let pos = TestDir::git_repo("r021-pos");
        let clean_png = build_test_png_with_chunk(b"tEXt", b"svg:title\x00CurveBook seven-pane logo");
        for (rel, content) in [
            (
                "ASSETS.md",
                "# Asset attribution\n\n| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n| `assets/icon.png` | CurveBook contributors | Original artwork | MIT | app icon |\n",
            ),
            ("ATTRIBUTION.md", "# Third-party attributions\n"),
            ("NOTICE", "CurveBook Contributors\n"),
            ("AGENTS.md", "# AGENTS\n- **Scope:** Existing local `plan/prism/` frames.\n"),
            ("README.md", "# CurveBook\nNot affiliated with ArtCraft. <!-- brand-ok -->\n"),
            ("crates/format/src/prism.rs", "pub fn ext_filter() -> &'static str { \"*.prism\" }\n"),
            ("crates/format/src/pzfx.rs", "pub const ROOT: &str = \"<PrismFile></PrismFile>\";\n"),
            (
                "crates/ui-egui/src/import_dialog.rs",
                "pub const CMD: &str = \"table.importPrism\";\npub const FILTER: &str = \"Import (*.curvebook, *.pzfx, *.prism)\";\n",
            ),
            ("crates/ui-egui/src/i18n.rs", "it: \"Copia report metodi e statistica\",\n"),
            ("crates/fit/src/dsl.rs", "pub struct Op { jmp_end: usize }\n"),
            (
                "packaging/macos/Info.plist.in",
                "<dict>\n  <key>CFBundleTypeName</key>\n  <string>External Scientific Project (.prism, .pzfx)</string>\n  <key>CFBundleTypeExtensions</key>\n  <array>\n    <string>prism</string>\n  </array>\n</dict>\n",
            ),
            (
                "packaging/linux/org.curvebook.CurveBook.mime.xml",
                "<mime-info>\n<mime-type type=\"application/x-prism-project\">\n<comment>External Project (.prism)</comment>\n<glob pattern=\"*.prism\"/>\n</mime-type>\n</mime-info>\n",
            ),
            (
                "packaging/linux/org.curvebook.CurveBook.desktop",
                "[Desktop Entry]\nName=CurveBook\nMimeType=application/vnd.curvebook.project;application/x-prism-project;application/x-prism-xml;\nKeywords=biostatistics;fitting;\n",
            ),
            ("packaging/web/index.html", "<!doctype html><title>CurveBook</title><input accept=\".prism,.pzfx\">"),
        ] {
            pos.write(rel, content);
        }
        pos.write_bytes("assets/icon.png", &clean_png);
        pos.git_add_all();
        let pos_files = crate::repo_files(pos.path()).unwrap();
        assert_eq!(super::scan_tree(pos.path(), &pos_files).map(|(hits, _)| hits), Ok(Vec::new()));
        assert_eq!(crate::cleanroom::run(pos.path()), Ok(()));
        assert_eq!(crate::assets::run(pos.path()), Ok(()));

        for (name, rel, content) in [
            ("title", "crates/ui-egui/src/title.rs", "pub const T: &str = \"Prism Studio\";\n"),
            ("camel", "crates/ui-egui/src/style.rs", "pub struct PrismStyle;\n"),
            ("snake", "crates/ui-egui/src/compat.rs", "pub fn like_prism() {}\n"),
            ("vendor_snake", "crates/ui-egui/src/theme.rs", "pub fn graphpad_theme() {}\n"),
            ("waiver_blocked", "crates/engine/src/help.rs", "pub const H: &str = \"Import GraphPad\"; // brand-ok\n"),
            ("cfg_test_leak", "crates/engine/src/mod.rs", "#[cfg(test)] mod t { const X: &str = \"Prism Archive\"; }\n"),
            ("tests_leak", "crates/engine/tests/leak.rs", "const X: &str = \"OriginPro import\";\n"),
            ("docs_md_leak", "docs/guide.md", "# Guide\nExport from Prism 11.\n"),
            ("path_leak", "crates/ui-egui/src/PrismStyle.rs", "pub struct X;\n"),
            ("xlstat_leak", "crates/fit/src/helper.rs", "pub fn XlStatHelper() {}\n"),
            ("statistica_cap", "crates/ui-egui/src/label.rs", "pub const L: &str = \"StatisticaReport\";\n"),
            ("dist_leak", "packaging/web/bad.html", "<!doctype html><title>Prism</title>"),
        ] {
            let neg = TestDir::new(&format!("r021-neg-{name}"));
            neg.write("ASSETS.md", "# Asset attribution\n\n| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n");
            neg.write("ATTRIBUTION.md", "# Attributions\n");
            neg.write("NOTICE", "CurveBook Contributors\n");
            neg.write(rel, content);
            let files: Vec<String> = ["ASSETS.md", "ATTRIBUTION.md", "NOTICE", rel].into_iter().map(String::from).collect();
            let (hits, _) = super::scan_tree(neg.path(), &files).unwrap();
            assert!(!hits.is_empty(), "negative probe `{name}` (`{rel}`) returned 0 brand hits");
        }
    }

    #[test]
    fn mcp_rpc_validator_enforces_narrow_open_file_boundary_and_rejects_contaminated_surfaces() {
        let clean_init = serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","serverInfo":{"name":"curvebook","version":"0.1.0"},"instructions":"CurveBook MCP server for scientific tables."}});
        assert!(super::check_mcp_rpc_response("init", "initialize", None, &clean_init).is_empty());

        let dirty_init = serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"serverInfo":{"name":"lumencraft-mcp","version":"0.1.0"},"instructions":"Compatible with GraphPad Prism workflows."}});
        let init_hits = super::check_mcp_rpc_response("init", "initialize", None, &dirty_init);
        for b in ["lumencraft", "graphpad", "prism"] {
            assert!(init_hits.iter().any(|h| h.brand == b), "{b} missing in {init_hits:?}");
        }

        let clean_tools = serde_json::json!({"jsonrpc":"2.0","id":2,"result":{"tools":[
            {"name":"open_file","description":"Open or import a .curvebook, .cbtemplate, .prism, .pzfx, or .csv file.","inputSchema":{"type":"object","properties":{"format":{"type":"string","enum":["curvebook","prism","pzfx","csv"]},"title":{"type":"string","description":"Optional sheet title."}}}},
            {"name":"table_import_prism","description":"[table.importPrism] Import External Archive — Import an external ZIP-based project archive.","inputSchema":{"type":"object"}}
        ]}});
        assert!(super::check_mcp_rpc_response("tools", "tools/list", None, &clean_tools).is_empty());

        let dirty_tools = serde_json::json!({"jsonrpc":"2.0","id":2,"result":{"tools":[
            {"name":"open_file","description":"Open a GraphPad Prism (.prism) project.","inputSchema":{"type":"object","properties":{"title":{"type":"string","description":"Optional .prism project title."}}}},
            {"name":"doc_inspect","description":"Inspect .prism or OriginPro sheets.","inputSchema":{"type":"object"}}
        ]}});
        let tool_hits = super::check_mcp_rpc_response("tools", "tools/list", None, &dirty_tools);
        assert!(tool_hits.iter().any(|h| h.brand == "graphpad") && tool_hits.iter().any(|h| h.brand == "originpro"));
        assert_eq!(tool_hits.iter().filter(|h| h.brand == "prism").count(), 1, "{tool_hits:?}");

        let dirty_prompt = serde_json::json!({"jsonrpc":"2.0","id":3,"result":{"description":"ArtCraft workflow","messages":[{"role":"user","content":{"type":"text","text":"Run ANOVA like Minitab or Prism."}}]}});
        let prompt_hits = super::check_mcp_rpc_response("prompt", "prompts/get", None, &dirty_prompt);
        for b in ["artcraft", "minitab", "prism"] {
            assert!(prompt_hits.iter().any(|h| h.brand == b), "{b} missing in {prompt_hits:?}");
        }

        let dirty_res = serde_json::json!({"jsonrpc":"2.0","id":4,"result":{"contents":[{"uri":"sheet://data-1","mimeType":"application/json","text":"{\"title\":\"SPSS Cohort\",\"legacy\":\"LumenCraft\"}"}]}});
        let res_hits = super::check_mcp_rpc_response("res", "resources/read", None, &dirty_res);
        assert!(res_hits.iter().any(|h| h.brand == "spss") && res_hits.iter().any(|h| h.brand == "lumencraft"));

        let b64_png = curvebook_platform::base64::encode(&build_test_png_with_chunk(b"tEXt", b"svg:title\x00LumenCraft preview"));
        let dirty_preview = serde_json::json!({"jsonrpc":"2.0","id":5,"result":{"content":[{"type":"image","mimeType":"image/png","data":b64_png},{"type":"text","text":"{\"title\":\"Clean\"}"}],"isError":false}});
        assert_eq!(
            super::check_mcp_rpc_response("preview", "tools/call", Some("render_preview"), &dirty_preview),
            vec![hit("preview:/result/content/0/data#png", 2, "lumencraft")]
        );

        let dirty_svg = serde_json::json!({"jsonrpc":"2.0","id":6,"result":{"content":[{"type":"text","text":"<svg xmlns=\"http://www.w3.org/2000/svg\"><title>Prism Plot</title></svg>"}],"isError":false}});
        assert!(super::check_mcp_rpc_response("export", "tools/call", Some("export"), &dirty_svg).iter().any(|h| h.brand == "prism"));
    }

    #[test]
    fn r12_8_dot_prism_generated_docs_and_audit_skip_boundaries() {
        let prism_hits = |title: &str| {
            let html = format!(
                "<html><head><title>{title}</title><meta name=\"description\" content=\"{title}\"></head><body><h1>{title}</h1></body></html>"
            );
            check_html_metadata("docs/page.html", &html).into_iter().filter(|h| h.brand == "prism").count()
        };
        for ok in ["Open .pzfx and .prism files", "Filter *.prism or experiment.prism", "Download https://example.com/files/experiment.prism"] {
            assert_eq!(prism_hits(ok), 0, "{ok}");
        }
        for bad in
            ["Open Prism files", "Open .Prism files", "See example.prism.com", "See https://example.prism/download", "See http://example.prism"]
        {
            assert!(prism_hits(bad) > 0, "{bad}");
        }
        let mut hits = Vec::new();
        super::collect_line_words("docs/page.html", 1, "Export .prism tables", &["prism"], super::PrismPolicy::Strict, &mut hits);
        assert!(hits.is_empty());
        super::collect_line_words("docs/page.html", 1, "Export prism tables", &["prism"], super::PrismPolicy::Strict, &mut hits);
        assert_eq!(hits.len(), 1);

        let ok_html = "<!doctype html><html><body><p>Supports <code>table.importPrism</code> and <code>&quot;prism&quot;</code> for <code>.prism</code> files (<a href=\"https://github.com/HauserGroup/curvebook/blob/main/crates/format/src/prism.rs\">source</a>).</p><p>Anchor <a href=\"#mcp-tool-table_import_prism\">tool</a> and <a href=\"#format-prism\">format</a>.</p></body></html>\n";
        for gen_path in [
            "docs-site/manual/v0.1/registry/formats.html",
            "docs-site/manual/latest/registry/formats.html",
            "target/docs-manual/latest/registry/formats.html",
        ] {
            assert!(scan(gen_path, ok_html).is_empty() && check_html_metadata(gen_path, ok_html).is_empty(), "{gen_path}");
        }
        let agents_html = "<p>Scope: crates/format/src/lib.rs (<code>plan/prism/</code> reference notes)</p>\n";
        assert!(scan("docs-site/manual/v0.1/contribute/agents.html", agents_html).is_empty());
        assert!(check_html_metadata("docs-site/manual/v0.1/contribute/agents.html", agents_html).is_empty());
        let bad_gen = scan("docs-site/manual/v0.1/files.html", "<p>Matches GraphPad Prism output with PrismStyle headers.</p>\n");
        assert!(bad_gen.iter().any(|h| h.brand == "graphpad") && bad_gen.iter().any(|h| h.brand == "prism"));
        for strict_path in ["docs-site/manual/index.html", "target/docs-manual/index.html", "docs/manual/v0.1/registry/formats.html"] {
            assert!(scan(strict_path, "<p>table.importPrism</p>\n").iter().any(|h| h.brand == "prism"), "{strict_path}");
        }

        for p in [
            "docs/audit/README.md",
            "docs/history/audit/findings/r1/S02.md",
            "./docs/history/audit",
            "docs/history/roadmap/r0.md",
            "docs/evidence/overview.md",
        ] {
            assert!(!skip_path(p), "{p}");
        }
        assert!(skip_path("xtask/src/brands.rs"));
    }

    #[test]
    fn sec22_escapes_svg_comments_dist_and_macos_stapled_ticket_guards() {
        let dir = TestDir::new("brands-sec22");
        dir.write("crates/core/src/esc.rs", "pub fn a() -> &'static str { \"\\x47raph\\u{0050}ad\" }\npub fn b() -> &'static str { \"\\x61rtcraft\" }\npub fn c() -> &'static str { \"\\x76ectorcraft\" }\n");
        dir.write("assets/ui/sneaky.svg", "<svg xmlns=\"http://www.w3.org/2000/svg\">\n<!-- OriginPro comment -->\n<g aria-label=\"Graph&#80;ad\"><textPath>Minitab</textPath></g>\n<image href=\"data:image/svg+xml;charset=utf-8;base64,PHN2Zz48dGl0bGU+U3lzdGF0PC90aXRsZT48L3N2Zz4=\"/>\n</svg>\n");
        dir.write("packaging/macos/INFO.PLIST", "<string>GraphPad</string>\n");
        dir.write("ASSETS.md", "| Asset | Author |\n|---|---|\n| `a` | Prism optical curve |\n");

        let files: Vec<String> =
            ["crates/core/src/esc.rs", "assets/ui/sneaky.svg", "packaging/macos/INFO.PLIST", "ASSETS.md"].into_iter().map(String::from).collect();
        let repo_hits = scan_repo_checked(dir.path(), &files).unwrap();
        for (suffix, brand) in [
            ("esc.rs", "graphpad"),
            ("esc.rs", "artcraft"),
            ("esc.rs", "vectorcraft"),
            ("sneaky.svg", "originpro"),
            ("sneaky.svg", "graphpad"),
            ("sneaky.svg", "minitab"),
            ("sneaky.svg", "systat"),
            ("INFO.PLIST", "graphpad"),
            ("ASSETS.md", "prism"),
        ] {
            assert!(repo_hits.iter().any(|h| h.path.ends_with(suffix) && h.brand == brand), "missing ({suffix}, {brand}) in {repo_hits:?}");
        }

        let svg_meta = check_svg_metadata("meta.svg", "<svg><!-- SigmaPlot --><text><textPath>Graph&#80;ad</textPath></text></svg>");
        assert!(svg_meta.iter().any(|h| h.brand == "sigmaplot") && svg_meta.iter().any(|h| h.brand == "graphpad"));
        for degenerate in ["<!--><g/>", "<!---><g/>", "<!----><g/>", "<!-->"] {
            assert!(check_svg_metadata("degenerate.svg", degenerate).is_empty());
            let mut hits = Vec::new();
            super::scan_svg("degenerate.svg", degenerate, &mut hits);
            assert!(hits.is_empty(), "{degenerate}: {hits:?}");
        }

        let dist_vc = TestDir::new("dist-vc");
        dist_vc.write_bytes("curvebook", b"\x7FELF\xFF\xFEclean_elf_stub");
        dist_vc.write_bytes("notes.txt", b"Powered by VectorCraft.\n");
        assert!(super::scan_dist_artefacts(dist_vc.path()).unwrap().iter().any(|h| h.brand == "vectorcraft"));

        let dist_bad = TestDir::new("dist-bad");
        dist_bad.write_bytes("index.html", b"\xFF\xFE\x00\xD8broken");
        assert!(super::scan_dist_artefacts(dist_bad.path()).is_ok());
        dist_bad.write_bytes("index.html", b"\xFFbad-utf8");
        assert!(super::scan_dist_artefacts(dist_bad.path()).is_err());

        // Apple stapled ticket guards (DA-03-02).
        let valid_ticket = b"s8ch\x01\x00\x00\x00\x30\x82\x01\x02\xFF\xFEnotarization-ticket";
        let ticket_dir = TestDir::new("macos-ticket");
        ticket_dir.write_bytes("CurveBook.app/Contents/CodeResources", valid_ticket);
        assert!(super::scan_dist_artefacts(ticket_dir.path()).unwrap().is_empty());
        ticket_dir.write_bytes("CurveBook.app/Contents/CodeResources", b"\xFF\x00not-a-stapled-ticket");
        assert!(super::scan_dist_artefacts(ticket_dir.path()).is_err());
        ticket_dir.write_bytes("CurveBook.app/Contents/CodeResources", valid_ticket);
        ticket_dir.write_bytes("CurveBook.app/Contents/Resources/bogus.txt", valid_ticket);
        assert!(super::scan_dist_artefacts(ticket_dir.path()).is_err());

        for ok_path in [
            "CurveBook.app/Contents/CodeResources",
            "macos/CurveBook.app/Contents/CodeResources",
            "CurveBook.dmg!CurveBook.app/Contents/CodeResources",
        ] {
            assert!(super::is_macos_stapled_ticket(ok_path, valid_ticket), "{ok_path}");
        }
        for bad_path in [
            ".app/Contents/CodeResources",
            "foo/.app/Contents/CodeResources",
            "project.curvebook!CurveBook.app/Contents/CodeResources",
            "template.cbtemplate!CurveBook.app/Contents/CodeResources",
            "archive.zip!CurveBook.app/Contents/CodeResources",
            "a.dmg!b.zip!CurveBook.app/Contents/CodeResources",
        ] {
            assert!(!super::is_macos_stapled_ticket(bad_path, valid_ticket), "{bad_path}");
        }
        assert!(!super::is_macos_stapled_ticket("CurveBook.app/Contents/CodeResources", b"s8ch\xFFnot-a-der-cms-envelope"));
        let mut oversized = valid_ticket.to_vec();
        oversized.resize(super::MAX_MACOS_STAPLED_TICKET_BYTES + 1, 0xAB);
        assert!(!super::is_macos_stapled_ticket("CurveBook.app/Contents/CodeResources", &oversized));

        let zip_dir = TestDir::new("macos-ticket-zip");
        let mut zip = crate::zip_read::TestZipWriter::new();
        zip.add_file("CurveBook.app/Contents/CodeResources", valid_ticket.to_vec()).unwrap();
        zip_dir.write_bytes("project.curvebook", &zip.finish().unwrap());
        assert!(super::scan_dist_artefacts(zip_dir.path()).is_err());
    }
}
