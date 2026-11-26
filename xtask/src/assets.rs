//! `cargo xtask assets`: every non-code asset must be attributed in `ASSETS.md` (and
//! `ATTRIBUTION.md` for third-party / attribution-bearing works) with an allowlisted permissive
//! licence, and zero font binaries may be tracked in the repository.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use crate::brands::{is_forbidden_bundled_dir, is_plan_prism_path};
use crate::scan_util::{ZIP_CONTAINER_EXTS, collect_dist_files, format_error_list, looks_like_zip, lower_ext};
use crate::zip_read::{ZipArchive, ZipLimits};

const FONT_EXT: &[&str] = &["ttf", "otf", "ttc", "otc", "woff", "woff2", "eot", "pfb", "pfa", "dfont", "sfnt", "bdf", "pcf", "fon", "fnt"];
const EXTRA_ZIP_ARCHIVE_EXT: &[&str] = &["docx", "odt", "jar", "whl"];

const VISUAL_ASSET_EXT: &[&str] =
    &["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "ico", "icns", "svg", "svgz", "avif", "pdf", "eps", "cur", "ani"];

const ORIGINAL_PROVENANCE_PREFIXES: &[&str] = &[
    "assets/app-icon/",
    "assets/ui/",
    "assets/icons/",
    "assets/diagrams/",
    "assets/schematics/",
    "assets/thumbnails/",
    "assets/screenshots/",
    "docs/images/",
];

const ORIGINAL_PROVENANCE_KEYWORDS: &[&str] =
    &["icon", "logo", "schematic", "diagram", "thumb", "screenshot", "archetype", "baseline", "cursor", "toolbar", "badge", "splash"];

const FORBIDDEN_PROVENANCE_PHRASES: &[&str] = &[
    "reference frame",
    "reference video",
    "video frame",
    "reference screenshot",
    "reference app",
    "reference application",
    "copied from",
    "scraped from",
    "extracted from",
    "traced from",
    "captured from",
    "derived from vendor",
    "derived from reference",
    "stock icon",
    "proprietary software",
    "competitor screenshot",
    "vendor screenshot",
    "vendor sample",
    "vendor guide",
    "plan/prism",
];

const FORBIDDEN_PROVENANCE_BRANDS: &[&str] = &[
    "graphpad",
    "dotmatics",
    "prism",
    "instat",
    "statmate",
    "originlab",
    "originpro",
    "sigmaplot",
    "minitab",
    "spss",
    "jmp",
    "systat",
    "statistica",
    "xlstat",
    "artcraft",
    "storytold",
    "storyteller",
    "lumencraft",
];

const ASSET_EXT: &[&str] = &[
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
    "svg",
    "svgz",
    "avif",
    "pdf",
    "eps",
    "icc",
    "icm",
    "cur",
    "ani",
    "ase",
    "aco",
    "gpl",
    "ttf",
    "otf",
    "ttc",
    "otc",
    "woff",
    "woff2",
    "eot",
    "pfb",
    "pfa",
    "dfont",
    "sfnt",
    "bdf",
    "pcf",
    "fon",
    "fnt",
    "curvebook",
    "cbtemplate",
    "prism",
    "pzfx",
    "pzf",
    "xlsx",
    "mp4",
    "wav",
    "mp3",
];

const ASSET_DIRS: &[&str] = &["assets/", "docs/images/", "examples/"];

const ALLOWED_ASSET_LICENCES: &[&str] = &[
    "MIT",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Zlib",
    "Unicode-3.0",
    "CC0-1.0",
    "LicenseRef-Public-Domain",
    "CC-BY-4.0",
    "CC-BY-SA-4.0",
    "OFL-1.1",
    "Ubuntu-font-1.0",
    "Bitstream-Vera",
];

const ATTRIBUTION_REQUIRED_LICENCES: &[&str] = &["CC-BY-4.0", "CC-BY-SA-4.0"];

#[derive(Debug, Clone, PartialEq, Eq)]
struct AssetRow {
    path: String,
    author: String,
    source: String,
    licence_tokens: Vec<String>,
    licence_note_path: Option<String>,
    notes: String,
}

#[must_use]
pub fn is_font_file(path: &str) -> bool {
    let ext = lower_ext(path);
    FONT_EXT.contains(&ext.as_str())
}

/// Returns `true` when `bytes` starts with an unambiguous font binary header
/// (`OTTO` OpenType CFF, `ttcf` TrueType/OpenType Collection, `wOFF` WOFF1, `wOF2` WOFF2,
/// or a valid TrueType sfnt offset table).
#[must_use]
pub fn has_font_magic(bytes: &[u8]) -> bool {
    if bytes.starts_with(b"OTTO") || bytes.starts_with(b"ttcf") || bytes.starts_with(b"wOFF") || bytes.starts_with(b"wOF2") {
        return true;
    }
    // TrueType sfnt header: version 0x00010000 + consistent binary-search directory fields.
    if bytes.len() >= 12 && bytes.starts_with(&[0x00, 0x01, 0x00, 0x00]) {
        let num_tables = u16::from_be_bytes([bytes[4], bytes[5]]);
        let search_range = u16::from_be_bytes([bytes[6], bytes[7]]);
        let entry_selector = u16::from_be_bytes([bytes[8], bytes[9]]);
        let range_shift = u16::from_be_bytes([bytes[10], bytes[11]]);
        if (4..=64).contains(&num_tables) {
            let max_pow2 = 1u16 << (15 - num_tables.leading_zeros());
            let expected_search = max_pow2.saturating_mul(16);
            let expected_selector = (15 - num_tables.leading_zeros()) as u16;
            let expected_shift = num_tables.saturating_mul(16).saturating_sub(expected_search);
            return search_range == expected_search && entry_selector == expected_selector && range_shift == expected_shift;
        }
    }
    false
}

fn is_zip_archive_ext(ext: &str) -> bool {
    ZIP_CONTAINER_EXTS.contains(&ext) || EXTRA_ZIP_ARCHIVE_EXT.contains(&ext)
}

#[must_use]
pub fn is_asset(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let ext = lower_ext(path);
    ASSET_DIRS.iter().any(|d| lower.starts_with(d)) || ASSET_EXT.contains(&ext.as_str())
}

#[must_use]
pub fn is_original_only_visual_asset(path: &str) -> bool {
    let ext = lower_ext(path);
    if !VISUAL_ASSET_EXT.contains(&ext.as_str()) {
        return false;
    }
    let lower = path.to_ascii_lowercase();
    if matches!(ext.as_str(), "cur" | "ani") || ORIGINAL_PROVENANCE_PREFIXES.iter().any(|prefix| lower.starts_with(prefix)) {
        return true;
    }
    ORIGINAL_PROVENANCE_KEYWORDS.iter().any(|kw| lower.contains(kw))
}

fn is_placeholder_cell(cell: &str) -> bool {
    let lower = cell.trim().to_ascii_lowercase();
    lower.is_empty() || matches!(lower.as_str(), "-" | "--" | "---" | "?" | "n/a" | "na" | "none" | "tbd" | "unknown")
}

fn is_separator_row(cells: &[&str]) -> bool {
    !cells.is_empty() && cells.iter().all(|c| !c.is_empty() && c.bytes().all(|b| matches!(b, b'-' | b':' | b' ')))
}

fn is_header_row(cells: &[&str]) -> bool {
    let first = cells.first().map(|s| s.to_ascii_lowercase()).unwrap_or_default();
    let fourth = cells.get(3).map(|s| s.to_ascii_lowercase()).unwrap_or_default();
    first == "asset" && (fourth.contains("licence") || fourth.contains("license"))
}

fn parse_table_cells(line: &str) -> Option<Vec<&str>> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix('|').and_then(|s| s.strip_suffix('|'))?;
    Some(inner.split('|').map(str::trim).collect())
}

fn has_copyleft_note_token(lower: &str) -> bool {
    if lower.contains("licenseref-") || lower.contains("cc-by-sa") || lower.contains("cc_by_sa") {
        return true;
    }
    lower.split(|c: char| !c.is_ascii_alphanumeric()).any(|tok| {
        matches!(tok, "gpl" | "agpl" | "lgpl" | "mpl" | "sspl")
            || ["gpl", "agpl", "lgpl", "mpl", "sspl"]
                .iter()
                .any(|prefix| tok.strip_prefix(prefix).is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit() || c == 'v')))
    })
}

fn is_safe_relative_licence_note(note: &str) -> bool {
    if note.is_empty()
        || note.starts_with('/')
        || note.contains('\\')
        || note.contains('\0')
        || note.chars().any(char::is_whitespace)
        || !note.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'-' | b'_'))
    {
        return false;
    }
    if Path::new(note).components().any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_))) {
        return false;
    }
    let lower_full = note.to_ascii_lowercase();
    if has_copyleft_note_token(&lower_full) {
        return false;
    }
    let fname = lower_full.rsplit('/').next().unwrap_or(lower_full.as_str());
    fname.starts_with("license")
        || fname.starts_with("licence")
        || fname.starts_with("copying")
        || fname.starts_with("notice")
        || fname.ends_with(".txt")
        || fname.ends_with(".md")
}

fn split_trailing_licence_note(raw: &str) -> (&str, Option<&str>) {
    let trimmed = raw.trim();
    if let Some(without_close) = trimmed.strip_suffix("`)")
        && let Some((head, note)) = without_close.rsplit_once(" (`")
        && is_safe_relative_licence_note(note)
    {
        return (head.trim(), Some(note));
    }
    (trimmed, None)
}

fn strip_trailing_licence_note(raw: &str) -> &str {
    split_trailing_licence_note(raw).0
}

fn validate_licence_expression(raw: &str) -> Result<Vec<String>, String> {
    let expr = strip_trailing_licence_note(raw);
    if expr.is_empty() {
        return Err(String::from("empty licence cell"));
    }
    let mut ids = Vec::new();
    let mut expect_id = true;
    for tok in expr.split_whitespace() {
        let tok = tok.trim_matches(|c| c == '(' || c == ')');
        if tok.is_empty() {
            continue;
        }
        if expect_id {
            let upper = tok.to_ascii_uppercase();
            if upper.starts_with("GPL") || upper.starts_with("AGPL") || upper.starts_with("LGPL") {
                return Err(format!("copyleft licence `{tok}` is forbidden"));
            }
            if !ALLOWED_ASSET_LICENCES.contains(&tok) {
                return Err(format!("disallowed or unknown licence `{tok}`"));
            }
            ids.push(tok.to_owned());
            expect_id = false;
        } else if tok == "OR" || tok == "AND" {
            expect_id = true;
        } else {
            return Err(format!("unexpected token `{tok}` in licence expression `{expr}`"));
        }
    }
    if expect_id || ids.is_empty() {
        return Err(format!("incomplete licence expression `{expr}`"));
    }
    Ok(ids)
}

fn parse_attribution_table(md: &str, table_name: &str) -> Result<BTreeMap<String, AssetRow>, String> {
    let mut rows = BTreeMap::new();
    for (idx, line) in md.lines().enumerate() {
        let line_no = idx + 1;
        let Some(cells) = parse_table_cells(line) else {
            continue;
        };
        if is_separator_row(&cells) || is_header_row(&cells) {
            continue;
        }
        let [asset_cell, author_cell, source_cell, licence_cell, ..] = cells.as_slice() else {
            return Err(format!("{table_name}:{line_no}: expected at least 4 table columns (Asset | Author | Source | Licence)"));
        };
        let Some(path) = asset_cell.strip_prefix('`').and_then(|s| s.strip_suffix('`')).map(str::trim) else {
            return Err(format!("{table_name}:{line_no}: asset cell `{asset_cell}` must be backticked"));
        };
        if path.is_empty() {
            return Err(format!("{table_name}:{line_no}: empty asset path"));
        }
        if is_placeholder_cell(author_cell) {
            return Err(format!("{table_name}:{line_no} (`{path}`): missing or placeholder author `{author_cell}`"));
        }
        if is_placeholder_cell(source_cell) {
            return Err(format!("{table_name}:{line_no} (`{path}`): missing or placeholder source `{source_cell}`"));
        }
        let (_, licence_note) = split_trailing_licence_note(licence_cell);
        let licence_tokens = validate_licence_expression(licence_cell).map_err(|e| format!("{table_name}:{line_no} (`{path}`): {e}"))?;
        if rows.contains_key(path) {
            return Err(format!("{table_name}:{line_no}: duplicate row for `{path}`"));
        }
        let notes = cells.get(4).map(|s| (*s).trim().to_owned()).unwrap_or_default();
        rows.insert(
            path.to_owned(),
            AssetRow {
                path: path.to_owned(),
                author: (*author_cell).to_owned(),
                source: (*source_cell).to_owned(),
                licence_tokens,
                licence_note_path: licence_note.map(str::to_owned),
                notes,
            },
        );
    }
    Ok(rows)
}

fn row_requires_attribution_entry(row: &AssetRow) -> bool {
    let third_party_author = !row.author.to_ascii_lowercase().contains("curvebook");
    let attr_licence = row.licence_tokens.iter().any(|tok| ATTRIBUTION_REQUIRED_LICENCES.contains(&tok.as_str()));
    third_party_author || attr_licence
}

fn has_ascii_word(lower: &str, word: &str) -> bool {
    let bytes = lower.as_bytes();
    let wlen = word.len();
    let mut start = 0usize;
    while let Some(rel) = lower.get(start..).and_then(|s| s.find(word)) {
        let pos = start + rel;
        let end = pos + wlen;
        let left_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
        let right_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if left_ok && right_ok {
            return true;
        }
        start = pos + lower.get(pos..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
    }
    false
}

fn check_forbidden_provenance_text(path: &str, field_name: &str, value: &str) -> Result<(), String> {
    let lower = value.to_ascii_lowercase();
    for phrase in FORBIDDEN_PROVENANCE_PHRASES {
        if lower.contains(phrase) {
            return Err(format!("{path}: {field_name} contains forbidden non-original provenance phrase `{phrase}`"));
        }
    }
    for brand in FORBIDDEN_PROVENANCE_BRANDS {
        if has_ascii_word(&lower, brand) {
            return Err(format!("{path}: {field_name} references forbidden external product or vendor `{brand}`"));
        }
    }
    Ok(())
}

fn check_row_safety(row: &AssetRow) -> Result<(), String> {
    check_forbidden_provenance_text(&row.path, "author", &row.author)?;
    check_forbidden_provenance_text(&row.path, "source", &row.source)?;
    check_forbidden_provenance_text(&row.path, "notes", &row.notes)?;
    Ok(())
}

fn extract_backticked_asset_refs(source: &str) -> Vec<&str> {
    let mut refs = Vec::new();
    let mut rest = source;
    while let Some((_, after)) = rest.split_once('`') {
        let Some((token_raw, tail)) = after.split_once('`') else {
            break;
        };
        let token = token_raw.trim();
        if token.contains('/') {
            refs.push(token);
        }
        rest = tail;
    }
    refs
}

fn is_direct_original_source(source: &str) -> bool {
    let lower = source.to_ascii_lowercase();
    if lower.contains("original")
        || lower.contains("cargo xtask screenshots")
        || lower.contains("generated in code")
        || lower.contains("rendered in-process")
    {
        return true;
    }
    lower.contains("curvebook")
        && (lower.contains("approved")
            || lower.contains("artwork")
            || lower.contains("generated")
            || lower.contains("created")
            || lower.contains("drawn")
            || lower.contains("authored")
            || lower.contains("rendered")
            || lower.contains("user-supplied"))
}

fn verify_original_provenance_chain(row: &AssetRow, all_rows: &BTreeMap<String, AssetRow>, visited: &mut BTreeSet<String>) -> Result<(), String> {
    if !visited.insert(row.path.clone()) {
        return Err(format!("{}: circular asset provenance chain in ASSETS.md", row.path));
    }
    check_row_safety(row)?;
    if !row.author.to_ascii_lowercase().contains("curvebook") {
        return Err(format!(
            "{}: UI icon, schematic, thumbnail, or screenshot baseline must declare CurveBook authorship (got `{}`)",
            row.path, row.author
        ));
    }
    let src_lower = row.source.to_ascii_lowercase();
    let notes_lower = row.notes.to_ascii_lowercase();
    if src_lower.contains("http://") || src_lower.contains("https://") || notes_lower.contains("http://") || notes_lower.contains("https://") {
        return Err(format!("{}: UI icon, schematic, thumbnail, or screenshot baseline cannot cite external URLs in ASSETS.md", row.path));
    }

    let parent_refs = extract_backticked_asset_refs(&row.source);
    if !parent_refs.is_empty() {
        for parent_path in parent_refs {
            let Some(parent_row) = all_rows.get(parent_path) else {
                return Err(format!("{}: cites parent asset `{parent_path}` missing from ASSETS.md", row.path));
            };
            verify_original_provenance_chain(parent_row, all_rows, visited)?;
        }
        visited.remove(&row.path);
        return Ok(());
    }

    if !is_direct_original_source(&row.source) {
        return Err(format!(
            "{}: UI icon, schematic, thumbnail, or screenshot baseline must declare original CurveBook provenance in ASSETS.md (source: `{}`)",
            row.path, row.source
        ));
    }
    visited.remove(&row.path);
    Ok(())
}

fn verify_original_provenance(row: &AssetRow, all_rows: &BTreeMap<String, AssetRow>) -> Result<(), String> {
    let mut visited = BTreeSet::new();
    verify_original_provenance_chain(row, all_rows, &mut visited)
}

/// Paths that require an `ASSETS.md` row (`path` in backticks with valid author, source, and
/// allowlisted SPDX licence) and lack one.
#[cfg(test)]
#[must_use]
pub fn missing(files: &[String], assets_md: &str) -> Vec<String> {
    let valid_paths: std::collections::BTreeSet<String> = match parse_attribution_table(assets_md, "ASSETS.md") {
        Ok(rows) => rows.into_keys().collect(),
        Err(_) => parse_valid_rows_lenient(assets_md),
    };
    files.iter().filter(|f| is_asset(f) && !valid_paths.contains(f.as_str())).cloned().collect()
}

#[cfg(test)]
fn parse_valid_rows_lenient(assets_md: &str) -> std::collections::BTreeSet<String> {
    let mut valid = std::collections::BTreeSet::new();
    for line in assets_md.lines() {
        let Some(cells) = parse_table_cells(line) else {
            continue;
        };
        if is_separator_row(&cells) || is_header_row(&cells) {
            continue;
        }
        let [asset_cell, author_cell, source_cell, licence_cell, ..] = cells.as_slice() else {
            continue;
        };
        if let Some(path) = asset_cell.strip_prefix('`').and_then(|s| s.strip_suffix('`')).map(str::trim)
            && !path.is_empty()
            && !is_placeholder_cell(author_cell)
            && !is_placeholder_cell(source_cell)
            && validate_licence_expression(licence_cell).is_ok()
        {
            valid.insert(path.to_owned());
        }
    }
    valid
}

/// Verify all repository files against zero-bundled-fonts, `ASSETS.md`, and `ATTRIBUTION.md`.
pub fn verify_assets(files: &[String], assets_md: &str, attribution_md: &str) -> Result<usize, String> {
    let fonts: Vec<&str> = files.iter().map(String::as_str).filter(|f| is_font_file(f)).collect();
    if !fonts.is_empty() {
        return Err(format_error_list("committed font file(s) forbidden by zero-bundled-fonts policy", &fonts));
    }

    let asset_rows = parse_attribution_table(assets_md, "ASSETS.md")?;
    let attr_rows = parse_attribution_table(attribution_md, "ATTRIBUTION.md")?;

    let mut errors = Vec::new();
    for row in asset_rows.values().chain(attr_rows.values()) {
        if let Err(err) = check_row_safety(row) {
            errors.push(err);
        }
    }
    for row in asset_rows.values() {
        if is_original_only_visual_asset(&row.path)
            && let Err(err) = verify_original_provenance(row, &asset_rows)
        {
            errors.push(err);
        }
    }

    let mut asset_count = 0usize;
    for file in files {
        if !is_asset(file) {
            continue;
        }
        asset_count += 1;
        let Some(row) = asset_rows.get(file.as_str()) else {
            errors.push(format!("{file}: missing valid ASSETS.md table row (author, source, allowlisted licence)"));
            continue;
        };
        if row_requires_attribution_entry(row) && !attr_rows.contains_key(&row.path) {
            errors.push(format!("{file}: third-party or attribution-licensed asset lacks an ATTRIBUTION.md row"));
        }
    }

    errors.sort();
    errors.dedup();
    if errors.is_empty() { Ok(asset_count) } else { Err(format_error_list("asset attribution error(s)", &errors)) }
}

fn check_document_prose(doc_name: &str, text: &str, allow_packaged: bool) -> Result<(), String> {
    let masked = crate::brands::mask_attributed_upstream_url(doc_name, text, allow_packaged);
    let check_vectorcraft = !crate::brands::is_attribution_or_notice_path(doc_name, allow_packaged);
    for (idx, line) in masked.lines().enumerate() {
        let line_no = idx + 1;
        let lower = line.to_ascii_lowercase();
        for phrase in FORBIDDEN_PROVENANCE_PHRASES {
            if lower.contains(phrase) {
                return Err(format!("{doc_name}:{line_no}: contains forbidden provenance phrase `{phrase}`"));
            }
        }
        for brand in FORBIDDEN_PROVENANCE_BRANDS {
            if has_ascii_word(&lower, brand) {
                return Err(format!("{doc_name}:{line_no}: references forbidden vendor or suite `{brand}`"));
            }
        }
        if check_vectorcraft && has_ascii_word(&lower, crate::brands::ADAPTED_UPSTREAM_BRAND) {
            return Err(format!(
                "{doc_name}:{line_no}: references adapted source brand `{}` outside ATTRIBUTION.md/NOTICE",
                crate::brands::ADAPTED_UPSTREAM_BRAND
            ));
        }
    }
    Ok(())
}

const MAX_DIST_ASSET_BYTES: usize = 256 * 1024 * 1024;

fn verify_asset_binaries(root: &Path, files: &[String], errors: &mut Vec<String>) {
    for path in files {
        let ext = lower_ext(path);
        let is_archive_ext = is_zip_archive_ext(&ext);
        let is_meta_ext =
            matches!(ext.as_str(), "png" | "ico" | "icns" | "svg" | "svgz" | "gif" | "pdf" | "jpg" | "jpeg" | "tif" | "tiff" | "webp" | "avif");
        let Ok(bytes) = curvebook_platform::fs::read_regular_file_bytes(&root.join(path), MAX_DIST_ASSET_BYTES) else {
            if is_archive_ext || is_meta_ext || is_asset(path) {
                errors.push(format!("{path}: unreadable or oversized asset file"));
            }
            continue;
        };
        if has_font_magic(&bytes) {
            errors.push(format!("{path}: committed font binary (font magic header) forbidden by zero-bundled-fonts policy"));
            continue;
        }
        if is_archive_ext || looks_like_zip(&bytes) {
            scan_repo_zip_for_fonts(path, &bytes, 0, errors);
        }
        if is_meta_ext || crate::brands::detect_binary_asset_magic(&bytes).is_some() {
            check_asset_embedded_metadata(path, &bytes, errors);
        }
    }
}

fn scan_repo_zip_for_fonts(display_path: &str, bytes: &[u8], depth: usize, errors: &mut Vec<String>) {
    if depth >= 2 {
        errors.push(format!("{display_path}: nested archive depth exceeded while inspecting for bundled fonts"));
        return;
    }
    match ZipArchive::from_bytes_with_limits(bytes, ZipLimits::default()) {
        Ok(zip) => {
            for (name, entry_bytes) in zip.iter() {
                let nested = format!("{display_path}!{name}");
                let entry_ext = lower_ext(name);
                if is_font_file(name) || has_font_magic(entry_bytes) {
                    errors.push(format!("{nested}: bundled font file forbidden by zero-bundled-fonts policy"));
                } else if is_zip_archive_ext(&entry_ext) || looks_like_zip(entry_bytes) {
                    scan_repo_zip_for_fonts(&nested, entry_bytes, depth + 1, errors);
                }
            }
        }
        Err(err) => {
            errors.push(format!("{display_path}: corrupt ZIP archive while checking bundled fonts: {err}"));
        }
    }
}

/// Full repository asset-provenance verification: checks zero bundled fonts, `ASSETS.md` /
/// `ATTRIBUTION.md` / `NOTICE` prose, orphan rows, forbidden `plan/prism` paths, original
/// CurveBook visual asset provenance, and PNG/ICO/ICNS/SVG embedded metadata chunks.
pub fn verify_repo_provenance(root: &Path, files: &[String], assets_md: &str, attribution_md: &str, notice: &str) -> Result<usize, String> {
    let count = verify_assets(files, assets_md, attribution_md)?;
    let mut errors = Vec::new();

    for path in files {
        if is_plan_prism_path(path) {
            errors.push(format!("{path}: committed `plan/prism` reference directory is forbidden"));
        }
    }

    let tracked: BTreeSet<&str> = files.iter().map(String::as_str).collect();
    let asset_rows = parse_attribution_table(assets_md, "ASSETS.md")?;
    let attr_rows = parse_attribution_table(attribution_md, "ATTRIBUTION.md")?;
    for path in asset_rows.keys() {
        if !tracked.contains(path.as_str()) {
            errors.push(format!("ASSETS.md: orphan entry `{path}` does not match any tracked file"));
        }
    }
    for path in attr_rows.keys() {
        if !tracked.contains(path.as_str()) {
            errors.push(format!("ATTRIBUTION.md: orphan entry `{path}` does not match any tracked file"));
        }
        if !asset_rows.contains_key(path.as_str()) {
            errors.push(format!("ATTRIBUTION.md: entry `{path}` missing corresponding ASSETS.md row"));
        }
    }

    for row in asset_rows.values().chain(attr_rows.values()) {
        if let Some(note_path) = &row.licence_note_path {
            if let Err(err) = reject_symlink_path_components(root, note_path, "licence note file") {
                errors.push(err);
            } else if !crate::scan_util::is_regular_file(&root.join(note_path)) {
                errors.push(format!("{}: referenced licence note `{note_path}` must exist as a regular non-symlink file", row.path));
            }
        }
    }

    for (name, body) in [("ASSETS.md", assets_md), ("ATTRIBUTION.md", attribution_md), ("NOTICE", notice)] {
        if let Err(err) = check_document_prose(name, body, false) {
            errors.push(err);
        }
    }

    verify_asset_binaries(root, files, &mut errors);

    errors.sort();
    errors.dedup();
    if errors.is_empty() { Ok(count) } else { Err(format_error_list("asset provenance error(s)", &errors)) }
}

pub fn run(root: &Path) -> Result<(), String> {
    let files = crate::repo_files(root)?;
    let md = std::fs::read_to_string(root.join("ASSETS.md")).map_err(|e| format!("ASSETS.md: {e}"))?;
    let attr_md = std::fs::read_to_string(root.join("ATTRIBUTION.md")).map_err(|e| format!("ATTRIBUTION.md: {e}"))?;
    let notice = std::fs::read_to_string(root.join("NOTICE")).map_err(|e| format!("NOTICE: {e}"))?;
    let count = verify_repo_provenance(root, &files, &md, &attr_md, &notice)?;
    println!("assets: all {count} asset files attributed in ASSETS.md with clean provenance and allowlisted licences");
    Ok(())
}

/// Verified SHA-256 digest catalog of attributed repository assets in `ASSETS.md`.
#[derive(Debug, Clone)]
pub(crate) struct RepoAssetCatalog {
    allowed_sha256: BTreeSet<[u8; 32]>,
}

fn validate_catalog_rel_path(rel: &str) -> Result<(), String> {
    if rel.is_empty() || rel.starts_with('/') || rel.contains('\\') || rel.contains('\0') {
        return Err(format!("ASSETS.md: unsafe asset path `{rel}`"));
    }
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(_) => {}
            Component::CurDir | Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("ASSETS.md: unsafe asset path `{rel}`"));
            }
        }
    }
    Ok(())
}

fn reject_symlink_path_components(root: &Path, rel: &str, label: &str) -> Result<(), String> {
    let mut cur = root.to_path_buf();
    for comp in Path::new(rel).components() {
        if let Component::Normal(seg) = comp {
            cur.push(seg);
            let meta = std::fs::symlink_metadata(&cur).map_err(|e| format!("{label} `{rel}`: {e}"))?;
            if meta.file_type().is_symlink() {
                return Err(format!("{label} `{rel}` must not traverse symlink `{}`", cur.display()));
            }
        }
    }
    Ok(())
}

/// Load and verify `ASSETS.md`, `ATTRIBUTION.md`, and `NOTICE` from `root`, indexing the SHA-256
/// digest of every attributed repository asset so unpacked release containers can be verified.
pub(crate) fn load_repo_asset_catalog(root: &Path) -> Result<RepoAssetCatalog, String> {
    let assets_md = std::fs::read_to_string(root.join("ASSETS.md")).map_err(|e| format!("ASSETS.md: {e}"))?;
    let attr_md = std::fs::read_to_string(root.join("ATTRIBUTION.md")).map_err(|e| format!("ATTRIBUTION.md: {e}"))?;
    let notice = std::fs::read_to_string(root.join("NOTICE")).map_err(|e| format!("NOTICE: {e}"))?;

    for (name, body) in [("ASSETS.md", assets_md.as_str()), ("ATTRIBUTION.md", attr_md.as_str()), ("NOTICE", notice.as_str())] {
        check_document_prose(name, body, false)?;
    }

    let asset_rows = parse_attribution_table(&assets_md, "ASSETS.md")?;
    let attr_rows = parse_attribution_table(&attr_md, "ATTRIBUTION.md")?;
    for row in asset_rows.values().chain(attr_rows.values()) {
        check_row_safety(row)?;
    }

    let mut allowed_sha256 = BTreeSet::new();
    for row in asset_rows.values() {
        validate_catalog_rel_path(&row.path)?;
        if row_requires_attribution_entry(row) && !attr_rows.contains_key(&row.path) {
            return Err(format!("{}: third-party or attribution-licensed asset lacks an ATTRIBUTION.md row", row.path));
        }
        if is_original_only_visual_asset(&row.path) {
            verify_original_provenance(row, &asset_rows)?;
        }
        if let Some(note_path) = &row.licence_note_path {
            reject_symlink_path_components(root, note_path, "ASSETS.md licence note file")?;
            if !crate::scan_util::is_regular_file(&root.join(note_path)) {
                return Err(format!("{}: referenced licence note `{note_path}` must exist as a regular non-symlink file", row.path));
            }
        }
        reject_symlink_path_components(root, &row.path, "ASSETS.md attributed file")?;
        let full = root.join(&row.path);
        let meta = std::fs::symlink_metadata(&full).map_err(|e| format!("ASSETS.md attributed file `{}`: {e}", row.path))?;
        if meta.file_type().is_symlink() || !meta.file_type().is_file() {
            return Err(format!("ASSETS.md attributed file `{}` must be a regular non-symlink file", row.path));
        }
        let bytes = curvebook_platform::fs::read_regular_file_bytes(&full, MAX_DIST_ASSET_BYTES)
            .map_err(|e| format!("ASSETS.md attributed file `{}`: {e}", row.path))?;
        allowed_sha256.insert(crate::release_scan::sha256_bytes(&bytes));
    }
    for row in attr_rows.values() {
        if let Some(note_path) = &row.licence_note_path {
            reject_symlink_path_components(root, note_path, "ATTRIBUTION.md licence note file")?;
            if !crate::scan_util::is_regular_file(&root.join(note_path)) {
                return Err(format!("{}: referenced licence note `{note_path}` must exist as a regular non-symlink file", row.path));
            }
        }
    }
    Ok(RepoAssetCatalog { allowed_sha256 })
}

/// Recursively verify an unpacked release container directory against `catalog`:
/// - forbids bundled font files (`.ttf`, `.otf`, `.woff`, `.woff2`, ...)
/// - forbids bundled `plan/prism`, `docs/audit`, or `docs/history/audit` trees
/// - verifies prose and table rows of any bundled `ASSETS.md`, `ATTRIBUTION.md`, or `NOTICE`
/// - verifies binary/SVG metadata and cryptographic `ASSETS.md` provenance of every packaged asset
pub(crate) fn scan_dist_artefacts(catalog: &RepoAssetCatalog, dist_dir: &Path) -> Result<(usize, Vec<String>), String> {
    if !dist_dir.exists() {
        return Err(format!("dist directory `{}` does not exist", dist_dir.display()));
    }
    let mut rel_files = Vec::new();
    collect_dist_files(dist_dir, dist_dir, &mut rel_files)?;
    rel_files.sort();

    let mut state = DistAssetScanState { catalog, dist_dir, checked_assets: 0, violations: Vec::new() };
    for rel in rel_files {
        if is_forbidden_bundled_dir(&rel) {
            state.violations.push(format!("{rel}: bundled reference or historical audit directory is forbidden"));
            continue;
        }
        let bytes = curvebook_platform::fs::read_regular_file_bytes(&dist_dir.join(&rel), MAX_DIST_ASSET_BYTES)
            .map_err(|e| format!("read dist file `{rel}`: {e}"))?;
        scan_dist_asset_bytes(&mut state, &rel, &bytes, 0);
    }
    state.violations.sort();
    state.violations.dedup();
    Ok((state.checked_assets, state.violations))
}

struct DistAssetScanState<'a> {
    catalog: &'a RepoAssetCatalog,
    dist_dir: &'a Path,
    checked_assets: usize,
    violations: Vec<String>,
}

fn scan_dist_asset_bytes(state: &mut DistAssetScanState<'_>, display_path: &str, bytes: &[u8], depth: usize) {
    if is_font_file(display_path) || has_font_magic(bytes) {
        state.checked_assets += 1;
        state.violations.push(format!("{display_path}: bundled font file forbidden by zero-bundled-fonts policy"));
        return;
    }

    check_bundled_attribution_doc(display_path, bytes, &mut state.violations);

    if is_asset(display_path) {
        state.checked_assets += 1;
        check_asset_embedded_metadata(display_path, bytes, &mut state.violations);
        let digest = crate::release_scan::sha256_bytes(bytes);
        if !state.catalog.allowed_sha256.contains(&digest) && !is_flatpak_appstream_icon_cache(state.catalog, state.dist_dir, display_path, bytes) {
            state
                .violations
                .push(format!("{display_path}: unattributed asset in release package (SHA-256 does not match any attributed entry in ASSETS.md)"));
        }
    } else if crate::brands::detect_binary_asset_magic(bytes).is_some() {
        check_asset_embedded_metadata(display_path, bytes, &mut state.violations);
    }

    let ext = lower_ext(display_path);
    if is_zip_archive_ext(&ext) || looks_like_zip(bytes) {
        if depth >= 2 {
            state.violations.push(format!("{display_path}: nested archive depth exceeded (max depth 2)"));
            return;
        }
        match ZipArchive::from_bytes_with_limits(bytes, ZipLimits::default()) {
            Ok(zip) => {
                for (name, entry_bytes) in zip.iter() {
                    let nested = format!("{display_path}!{name}");
                    if is_forbidden_bundled_dir(name) {
                        state.violations.push(format!("{nested}: bundled reference or historical audit directory is forbidden"));
                        continue;
                    }
                    scan_dist_asset_bytes(state, &nested, entry_bytes, depth + 1);
                }
            }
            Err(err) => {
                state.violations.push(format!("{display_path}: cannot inspect nested archive: {err}"));
            }
        }
    }
}

fn check_bundled_attribution_doc(display_path: &str, bytes: &[u8], violations: &mut Vec<String>) {
    let entry_part = display_path.rsplit('!').next().unwrap_or(display_path);
    let Some(file_name) = Path::new(entry_part).file_name().and_then(|n| n.to_str()) else {
        return;
    };
    if !matches!(file_name, "ASSETS.md" | "ATTRIBUTION.md" | "NOTICE") {
        return;
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        violations.push(format!("{display_path}: bundled {file_name} is not valid UTF-8"));
        return;
    };
    if let Err(err) = check_document_prose(display_path, text, true) {
        violations.push(err);
    }
    if matches!(file_name, "ASSETS.md" | "ATTRIBUTION.md") {
        match parse_attribution_table(text, display_path) {
            Ok(rows) => {
                for row in rows.values() {
                    if let Err(err) = check_row_safety(row) {
                        violations.push(format!("{display_path}: {err}"));
                    }
                }
            }
            Err(err) => violations.push(err),
        }
    }
}

fn check_asset_embedded_metadata(display_path: &str, bytes: &[u8], violations: &mut Vec<String>) {
    let ext = lower_ext(display_path);
    let effective_fmt =
        if matches!(ext.as_str(), "png" | "ico" | "icns" | "gif" | "jpg" | "jpeg" | "tif" | "tiff" | "webp" | "avif" | "pdf" | "svg" | "svgz") {
            ext.as_str()
        } else if let Some(magic_fmt) = crate::brands::detect_binary_asset_magic(bytes) {
            magic_fmt
        } else {
            ext.as_str()
        };
    let hits = match effective_fmt {
        "png" => match crate::brands::check_png_metadata_checked(display_path, bytes) {
            Ok(h) => h,
            Err(err) => {
                violations.push(err);
                Vec::new()
            }
        },
        "ico" | "icns" => match crate::brands::check_icon_container_metadata_checked(display_path, bytes) {
            Ok(h) => h,
            Err(err) => {
                violations.push(err);
                Vec::new()
            }
        },
        "gif" => match crate::brands::check_gif_metadata_checked(display_path, bytes) {
            Ok(h) => h,
            Err(err) => {
                violations.push(err);
                Vec::new()
            }
        },
        "jpg" | "jpeg" | "tif" | "tiff" | "webp" | "avif" => match crate::brands::check_raster_metadata_checked(display_path, effective_fmt, bytes) {
            Ok(h) => h,
            Err(err) => {
                violations.push(err);
                Vec::new()
            }
        },
        "pdf" => match crate::brands::check_pdf_metadata_checked(display_path, bytes) {
            Ok(h) => h,
            Err(err) => {
                violations.push(err);
                Vec::new()
            }
        },
        "svg" => match crate::text_decode::decode_utf8_or_utf16(bytes) {
            Ok(Some(svg)) => crate::brands::check_svg_metadata(display_path, &svg),
            Ok(None) => {
                violations.push(format!("{display_path}: invalid UTF-8/UTF-16 in SVG"));
                Vec::new()
            }
            Err(e) => {
                violations.push(format!("{display_path}: invalid SVG text: {e}"));
                Vec::new()
            }
        },
        "svgz" => match crate::brands::inflate_gzip_bounded(bytes) {
            Ok(raw) => match crate::text_decode::decode_utf8_or_utf16(&raw) {
                Ok(Some(svg)) => crate::brands::check_svg_metadata(display_path, &svg),
                _ => {
                    violations.push(format!("{display_path}: invalid UTF-8/UTF-16 inside .svgz"));
                    Vec::new()
                }
            },
            Err(()) => {
                violations.push(format!("{display_path}: corrupt or oversized .svgz gzip payload"));
                Vec::new()
            }
        },
        _ => Vec::new(),
    };
    for hit in hits {
        violations.push(format!("{}:{}: embedded metadata mentions `{}`", hit.path, hit.line, hit.brand));
    }
}

fn is_valid_icon_dim(dim: &str) -> bool {
    let Some((w, h)) = dim.split_once('x') else {
        return false;
    };
    !w.is_empty()
        && !h.is_empty()
        && w.bytes().all(|b| b.is_ascii_digit())
        && h.bytes().all(|b| b.is_ascii_digit())
        && w.parse::<u32>().is_ok_and(|v| v > 0)
        && h.parse::<u32>().is_ok_and(|v| v > 0)
}

fn decode_png_to_rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new_with_limits(std::io::Cursor::new(bytes), png::Limits { bytes: 64 * 1024 * 1024 });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let buf_len = reader.output_buffer_size()?;
    let mut buf = vec![0u8; buf_len];
    let info = reader.next_frame(&mut buf).ok()?;
    let raw = buf.get(..info.buffer_size())?;
    let pixel_count = usize::try_from(info.width).ok()?.checked_mul(usize::try_from(info.height).ok()?)?;
    let rgba = match info.color_type {
        png::ColorType::Rgba => raw.to_vec(),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity(pixel_count.checked_mul(4)?);
            for &[r, g, b] in raw.as_chunks::<3>().0 {
                out.extend_from_slice(&[r, g, b, 255]);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity(pixel_count.checked_mul(4)?);
            for &[g, a] in raw.as_chunks::<2>().0 {
                out.extend_from_slice(&[g, g, g, a]);
            }
            out
        }
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity(pixel_count.checked_mul(4)?);
            for &g in raw {
                out.extend_from_slice(&[g, g, g, 255]);
            }
            out
        }
        png::ColorType::Indexed => return None,
    };
    if rgba.len() != pixel_count.checked_mul(4)? {
        return None;
    }
    Some((info.width, info.height, rgba))
}

fn is_flatpak_appstream_icon_cache(catalog: &RepoAssetCatalog, dist_dir: &Path, display_path: &str, candidate_bytes: &[u8]) -> bool {
    const MARKER: &str = "/share/app-info/icons/flatpak/";
    const ICON_FILE: &str = "/org.curvebook.CurveBook.png";
    if display_path.contains('!') || display_path.contains('\\') || display_path.contains('\0') {
        return false;
    }
    let Some((prefix, after)) = display_path.split_once(MARKER) else {
        return false;
    };
    if Path::new(prefix).components().any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_))) {
        return false;
    }
    let Some(dim) = after.strip_suffix(ICON_FILE) else {
        return false;
    };
    if !is_valid_icon_dim(dim) {
        return false;
    }
    let hicolor_rel = format!("{prefix}/share/icons/hicolor/{dim}/apps/org.curvebook.CurveBook.png");
    let Ok(src_bytes) = curvebook_platform::fs::read_regular_file_bytes(&dist_dir.join(hicolor_rel), MAX_DIST_ASSET_BYTES) else {
        return false;
    };
    if !catalog.allowed_sha256.contains(&crate::release_scan::sha256_bytes(&src_bytes)) {
        return false;
    }
    match (decode_png_to_rgba(candidate_bytes), decode_png_to_rgba(&src_bytes)) {
        (Some(cand_px), Some(src_px)) => cand_px == src_px,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{is_asset, is_font_file, is_forbidden_bundled_dir, is_original_only_visual_asset, missing, verify_assets};
    use crate::scan_util::test_support::TestDir;

    fn zip_single(name: &str, body: &[u8]) -> Vec<u8> {
        let mut zw = crate::zip_read::TestZipWriter::new();
        zw.add_file(name, body.to_vec()).unwrap();
        zw.finish().unwrap()
    }

    #[test]
    fn detects_unattributed_assets() {
        let files =
            vec![String::from("assets/app-icon/curvebook.svg"), String::from("crates/stats/src/lib.rs"), String::from("docs/images/plot.png")];
        let md = "| `assets/app-icon/curvebook.svg` | CurveBook contributors | original | MIT OR Apache-2.0 |";
        assert_eq!(missing(&files, md), vec![String::from("docs/images/plot.png")]);
        assert!(!is_asset("crates/stats/src/lib.rs"));
        assert!(is_asset("examples/dose.curvebook") && is_asset("examples/dose.cbtemplate"));
    }

    #[test]
    fn rejects_proprietary_or_placeholder_rows_and_prose_mentions() {
        let files = vec![String::from("assets/plain.png"), String::from("assets/prose.png")];
        let md = "See also `assets/prose.png` in prose.\n| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n| `assets/plain.png` | unknown | copied from a vendor web page | Proprietary, all rights reserved | bad |\n";
        assert_eq!(missing(&files, md), vec![String::from("assets/plain.png"), String::from("assets/prose.png")]);
        let err = verify_assets(&files, md, "").unwrap_err();
        assert!(err.contains("assets/plain.png"), "{err}");
    }

    #[test]
    fn detects_cursor_font_image_and_palette_extensions_and_forbids_fonts() {
        for path in [
            "crates/tools/src/grab.cur",
            "crates/tools/src/spin.ani",
            "docs/images/icon.svgz",
            "docs/images/photo.avif",
            "palettes/okabe.ase",
            "palettes/okabe.aco",
            "palettes/okabe.gpl",
            "fonts/Face.ttc",
            "fonts/Face.eot",
            "fonts/Face.pfb",
        ] {
            assert!(is_asset(path), "expected is_asset({path})");
        }
        for font in ["fonts/a.ttf", "fonts/b.otf", "fonts/c.ttc", "fonts/d.woff", "fonts/e.woff2", "fonts/f.eot", "fonts/g.pfb"] {
            assert!(is_font_file(font), "expected is_font_file({font})");
            let row = format!("| `{font}` | Some Foundry | local | OFL-1.1 |\n");
            let err = verify_assets(&[font.to_owned()], &row, &row).unwrap_err();
            assert!(err.contains("committed font file"), "expected font ban error for {font}, got: {err}");
        }
    }

    #[test]
    fn enforces_attribution_md_for_third_party_and_cc_by_assets_while_allowing_curvebook_assets() {
        let files = vec![String::from("assets/mole.png"), String::from("assets/third.svg")];
        let assets_ok = "| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n| `assets/mole.png` | CurveBook contributors | Original artwork | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | ok |\n| `assets/third.svg` | Upstream Icon Set | https://example.org/icon | CC-BY-4.0 | third-party |\n";
        let err = verify_assets(&files, assets_ok, "").unwrap_err();
        assert!(err.contains("ATTRIBUTION.md") && err.contains("assets/third.svg"), "{err}");

        let attr_ok = "| Asset | Author | Source | SPDX license | Notes |\n|---|---|---|---|---|\n| `assets/third.svg` | Upstream Icon Set | https://example.org/icon | CC-BY-4.0 | attributed |\n";
        assert_eq!(verify_assets(&files, assets_ok, attr_ok).unwrap(), 2);
    }

    #[test]
    fn enforces_original_provenance_on_ui_icons_schematics_thumbnails_and_screenshots() {
        for path in [
            "assets/ui/archetypes/xy.svg",
            "assets/icons/toolbar-analyze.svg",
            "assets/diagrams/anova-schematic.svg",
            "assets/thumbnails/bar-thumb.png",
            "docs/images/workspace-split-fit.png",
            "assets/app-icon/source/approved-seven-pane.png",
            "assets/app-icon/source/approved-mole.png",
            "assets/app-icon/curvebook-win95-2508.png",
        ] {
            assert!(is_original_only_visual_asset(path), "expected original provenance check on {path}");
        }

        let good_files = vec![
            String::from("assets/ui/archetypes/xy.svg"),
            String::from("assets/diagrams/anova-schematic.svg"),
            String::from("assets/thumbnails/bar-thumb.png"),
            String::from("docs/images/welcome.png"),
        ];
        let good_md = "\
| Asset | Author | Source | Licence | Notes |
|---|---|---|---|---|
| `assets/ui/archetypes/xy.svg` | CurveBook contributors | Original SVG schematic drawn for CurveBook | MIT OR Apache-2.0 | Archetype icon |
| `assets/diagrams/anova-schematic.svg` | CurveBook contributors | Original diagram created for CurveBook | MIT OR Apache-2.0 | Schematic |
| `assets/thumbnails/bar-thumb.png` | CurveBook contributors | Rendered from `assets/ui/archetypes/xy.svg` | MIT OR Apache-2.0 | Thumbnail |
| `docs/images/welcome.png` | CurveBook contributors | Deterministic workspace screenshot generated by cargo xtask screenshots | MIT OR Apache-2.0 | Baseline |
";
        assert_eq!(verify_assets(&good_files, good_md, "").unwrap(), 4);

        for (files, md, attr, needle) in [
            (
                vec!["assets/ui/icons/save.svg"],
                "| `assets/ui/icons/save.svg` | Upstream Icon Set | https://example.org/save.svg | CC-BY-4.0 | imported |\n",
                "| `assets/ui/icons/save.svg` | Upstream Icon Set | https://example.org/save.svg | CC-BY-4.0 | imported |\n",
                "CurveBook authorship",
            ),
            (
                vec!["docs/images/frame.png"],
                "| `docs/images/frame.png` | CurveBook contributors | Derived from reference frame 12 | MIT OR Apache-2.0 | screenshot |\n",
                "",
                "non-original provenance phrase",
            ),
            (
                vec!["assets/thumbnails/card.png"],
                "| `assets/thumbnails/card.png` | CurveBook contributors | Exported raster image | MIT OR Apache-2.0 | card |\n",
                "",
                "original CurveBook provenance",
            ),
            (
                vec!["assets/ui/a.png", "assets/ui/b.png"],
                "| `assets/ui/a.png` | CurveBook contributors | Rendered from `assets/ui/b.png` | MIT OR Apache-2.0 | a |\n| `assets/ui/b.png` | CurveBook contributors | Rendered from `assets/ui/a.png` | MIT OR Apache-2.0 | b |\n",
                "",
                "circular asset provenance chain",
            ),
            (
                vec!["assets/ui/orphan.png"],
                "| `assets/ui/orphan.png` | CurveBook contributors | Rendered from `assets/ui/missing.svg` | MIT OR Apache-2.0 | x |\n",
                "",
                "missing from ASSETS.md",
            ),
        ] {
            let owned: Vec<String> = files.into_iter().map(str::to_owned).collect();
            let err = verify_assets(&owned, md, attr).unwrap_err();
            assert!(err.contains(needle), "expected `{needle}` in `{err}`");
        }
    }

    #[test]
    fn repo_assets_md_preserves_seven_pane_art_and_passes_provenance() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let assets_md = std::fs::read_to_string(root.join("ASSETS.md")).unwrap();
        let attr_md = std::fs::read_to_string(root.join("ATTRIBUTION.md")).unwrap();
        let files: Vec<String> = [
            "assets/app-icon/source/approved-mole.png",
            "assets/app-icon/source/approved-seven-pane.png",
            "assets/app-icon/source/seven-pane-win95-1254.png",
            "assets/app-icon/curvebook-win95-2508.png",
            "assets/app-icon/curvebook.svg",
            "docs/images/workspace-split-fit.png",
            "docs/images/workspace-layout-dark.png",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(verify_assets(&files, &assets_md, &attr_md).unwrap(), 7);
    }

    #[test]
    fn repo_provenance_rejects_orphan_rows_contaminated_notice_and_plan_prism_paths() {
        let root = TestDir::new("assets-prov");
        root.write("assets/ui/icon.svg", "<svg><title>CurveBook icon</title></svg>");
        let files = vec![String::from("assets/ui/icon.svg")];
        let assets_ok = "| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n| `assets/ui/icon.svg` | CurveBook contributors | Original SVG icon drawn for CurveBook | MIT OR Apache-2.0 | UI icon |\n";
        assert_eq!(super::verify_repo_provenance(root.path(), &files, assets_ok, "", "CurveBook contributors").unwrap(), 1);

        let err_notice = super::verify_repo_provenance(root.path(), &files, assets_ok, "", "Derived from GraphPad artwork").unwrap_err();
        assert!(err_notice.contains("NOTICE") && err_notice.contains("graphpad"), "{err_notice}");

        let assets_orphan = format!("{assets_ok}| `assets/ui/ghost.svg` | CurveBook contributors | Original SVG | MIT | orphan |\n");
        assert!(super::verify_repo_provenance(root.path(), &files, &assets_orphan, "", "CurveBook contributors").unwrap_err().contains("orphan"));

        let files_plan = vec![String::from("assets/ui/icon.svg"), String::from("plan/prism/frame01.jpg")];
        assert!(super::verify_repo_provenance(root.path(), &files_plan, assets_ok, "", "CurveBook contributors").unwrap_err().contains("plan/prism"));

        root.write("assets/ui/icon.svg", "<svg><title>OriginPro icon</title></svg>");
        assert!(super::verify_repo_provenance(root.path(), &files, assets_ok, "", "CurveBook contributors").unwrap_err().contains("originpro"));
    }

    #[test]
    fn archived_audit_history_is_forbidden_in_bundles() {
        assert!(is_forbidden_bundled_dir("docs/history/audit"));
        assert!(is_forbidden_bundled_dir("web/docs/history/audit/findings"));
        assert!(!is_forbidden_bundled_dir("docs/history/roadmap"));
    }

    #[test]
    fn r15_1_document_prose_permits_only_exact_upstream_url_in_attributed_section() {
        let story = ["sto", "ry", "told"].concat();
        let vc = ["vec", "tor", "craft"].concat();
        let vc_title = ["Vec", "tor", "Cra", "ft"].concat();
        let exact_url = format!("https://github.com/{story}/{vc}");

        let valid_attr = format!("## Attributed Source Code\n\nAdapted helpers from {vc_title} (`{exact_url}`).\n");
        let valid_notice = format!("CurveBook\n\n## Attributed Source Code\n\nFrom {vc_title} ({exact_url}, commit abc).\n");
        assert_eq!(super::check_document_prose("ATTRIBUTION.md", &valid_attr, false), Ok(()));
        assert_eq!(super::check_document_prose("NOTICE", &valid_notice, false), Ok(()));
        assert_eq!(super::check_document_prose("curvebook-web-0.1.0/NOTICE", &valid_notice, true), Ok(()));

        for (doc, allow_pkg, bad) in [
            ("ASSETS.md", false, format!("Adapted from {exact_url}\n")),
            ("ASSETS.md", false, format!("Adapted from {vc_title}\n")),
            ("ATTRIBUTION.md", false, format!("## Other\n{exact_url}\n")),
            ("ATTRIBUTION.md", false, format!("## Attributed Source Code\n{exact_url}/blob/main/foo.rs\n")),
            ("ATTRIBUTION.md", false, format!("## Attributed Source Code\nhttp://github.com/{story}/{vc}\n")),
            ("ATTRIBUTION.md", false, format!("## Attributed Source Code\n{story}\n")),
            ("docs/ATTRIBUTION.md", false, format!("## Attributed Source Code\n{exact_url}\n")),
            ("NOTICE", false, format!("Preamble {exact_url}\n")),
        ] {
            assert!(super::check_document_prose(doc, &bad, allow_pkg).is_err(), "expected failure for {doc} (allow_pkg={allow_pkg}): {bad:?}");
        }
    }

    #[test]
    fn sec22_handles_bare_pipe_case_prefixes_zip_fonts_pdf_svgz_and_nested_depth() {
        assert_eq!(super::parse_table_cells("|"), None);
        assert!(is_asset("ASSETS/APP-ICON/ICON.PNG"));
        assert!(is_original_only_visual_asset("ASSETS/APP-ICON/ICON.PNG"));

        let root = TestDir::new("assets-sec22");
        root.write_bytes("assets/ui/bundle.zip", &zip_single("fonts/Sneaky.ttf", b"font"));
        let zip_md =
            "| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n| `assets/ui/bundle.zip` | CurveBook | Original | MIT | z |\n";
        let err_zip = super::verify_repo_provenance(root.path(), &[String::from("assets/ui/bundle.zip")], zip_md, "", "CurveBook").unwrap_err();
        assert!(err_zip.contains("Sneaky.ttf"), "{err_zip}");

        root.write_bytes("assets/ui/report.pdf", b"%PDF-1.4\n1 0 obj\n<< /Creator (GraphPad Prism) >>\nendobj\n");
        let pdf_md =
            "| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n| `assets/ui/report.pdf` | CurveBook | Original | MIT | p |\n";
        let err_pdf = super::verify_repo_provenance(root.path(), &[String::from("assets/ui/report.pdf")], pdf_md, "", "CurveBook").unwrap_err();
        assert!(err_pdf.contains("graphpad") || err_pdf.contains("prism"), "{err_pdf}");

        let lvl2 = zip_single("ok.txt", b"clean");
        let lvl1 = zip_single("inner.zip", &lvl2);
        root.write_bytes("dist/deep.zip", &zip_single("mid.zip", &lvl1));
        let cat = super::RepoAssetCatalog { allowed_sha256: std::collections::BTreeSet::new() };
        let (_, dist_errs) = super::scan_dist_artefacts(&cat, &root.join("dist")).unwrap();
        assert!(dist_errs.iter().any(|e| e.contains("nested archive depth")), "{dist_errs:?}");
    }

    fn encode_1x1_png(rgba: [u8; 4], compression: png::Compression) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, 1, 1);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(compression);
        let mut writer = enc.write_header().unwrap();
        writer.write_image_data(&rgba).unwrap();
        writer.finish().unwrap();
        out
    }

    #[test]
    fn aud_12_08_09_43_flatpak_pixel_check_xlsx_font_magic_raster_meta_and_catalog_guards() {
        // AUD-12-08: Flatpak appstream icon cache requires matching decoded RGBA pixels.
        let src_png = encode_1x1_png([10, 20, 30, 255], png::Compression::default());
        let reencoded_same = encode_1x1_png([10, 20, 30, 255], png::Compression::Fast);
        let different_png = encode_1x1_png([200, 10, 10, 255], png::Compression::default());
        let mut allowed = std::collections::BTreeSet::new();
        allowed.insert(crate::release_scan::sha256_bytes(&src_png));
        let cat = super::RepoAssetCatalog { allowed_sha256: allowed };

        let fp_dir = TestDir::new("assets-aud12-08");
        fp_dir.write_bytes("files/share/icons/hicolor/128x128/apps/org.curvebook.CurveBook.png", &src_png);
        fp_dir.write_bytes("files/share/app-info/icons/flatpak/128x128/org.curvebook.CurveBook.png", &reencoded_same);
        let (_, ok_errs) = super::scan_dist_artefacts(&cat, fp_dir.path()).unwrap();
        assert!(ok_errs.is_empty(), "same-pixel re-encoded flatpak icon must pass: {ok_errs:?}");

        fp_dir.write_bytes("files/share/app-info/icons/flatpak/128x128/org.curvebook.CurveBook.png", &different_png);
        let (_, bad_errs) = super::scan_dist_artefacts(&cat, fp_dir.path()).unwrap();
        assert!(bad_errs.iter().any(|e| e.contains("unattributed asset")), "different-pixel flatpak icon must fail: {bad_errs:?}");

        // AUD-12-09: extended font extensions, font magic bytes, .xlsx ZIP font scan, corrupt dist .zip, JPEG metadata.
        for ext_path in ["a.otc", "b.pfa", "c.dfont", "d.sfnt", "e.bdf", "f.pcf", "g.fon", "h.fnt"] {
            assert!(is_font_file(ext_path), "expected font ext: {ext_path}");
        }
        assert!(super::has_font_magic(b"OTTO\x00\x01"));
        assert!(super::has_font_magic(b"wOF2\x00\x01"));

        let repo_dir = TestDir::new("assets-aud12-09");
        repo_dir.write_bytes("examples/book.xlsx", &zip_single("xl/fonts/Hidden.ttf", b"font"));
        repo_dir.write_bytes("assets/renamed.bin", b"wOF2fontpayload");
        let xlsx_md = "| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n| `examples/book.xlsx` | CurveBook | Original | MIT | x |\n| `assets/renamed.bin` | CurveBook | Original | MIT | b |\n";
        let err_repo = super::verify_repo_provenance(
            repo_dir.path(),
            &[String::from("examples/book.xlsx"), String::from("assets/renamed.bin")],
            xlsx_md,
            "",
            "CurveBook",
        )
        .unwrap_err();
        assert!(err_repo.contains("Hidden.ttf"), "{err_repo}");
        assert!(err_repo.contains("font magic"), "{err_repo}");

        let dist_dir = TestDir::new("assets-aud12-09-dist");
        dist_dir.write_bytes("corrupt.zip", b"NOT_A_VALID_ZIP");
        let mut jpeg_dirty = vec![0xFF, 0xD8, 0xFF, 0xFE, 0x00, 0x10];
        jpeg_dirty.extend_from_slice(b"GraphPad Prism");
        jpeg_dirty.extend_from_slice(&[0xFF, 0xD9]);
        dist_dir.write_bytes("photo.jpg", &jpeg_dirty);
        let (_, d_errs) = super::scan_dist_artefacts(&cat, dist_dir.path()).unwrap();
        assert!(d_errs.iter().any(|e| e.contains("corrupt.zip")), "{d_errs:?}");
        assert!(d_errs.iter().any(|e| e.contains("photo.jpg") && e.contains("graphpad")), "{d_errs:?}");

        // AUD-12-43: licence note rejects copyleft SPDX injection (`MIT (`GPL-3.0-only`)`), missing
        // licence note files, traversal, and intermediate symlinked parent directories.
        for bad_lic_cell in ["MIT (`or GPL-3.0`)", "MIT (`GPL-3.0-only`)", "MIT (`AGPL-3.0-or-later`)"] {
            let bad_lic = format!(
                "| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n| `assets/x.png` | CurveBook | Original | {bad_lic_cell} | n |\n"
            );
            assert!(verify_assets(&[String::from("assets/x.png")], &bad_lic, "").is_err(), "must reject `{bad_lic_cell}`");
        }

        // Image+ZIP polyglot in repo and dist checks BOTH ZIP fonts AND image metadata.
        let mut jpeg_zip = vec![0xFF, 0xD8, 0xFF, 0xFE, 0x00, 0x0B];
        jpeg_zip.extend_from_slice(b"OriginPro");
        jpeg_zip.extend_from_slice(&[0xFF, 0xD9]);
        jpeg_zip.extend_from_slice(&zip_single("fonts/Sneaky.ttf", b"font"));
        let poly_repo = TestDir::new("assets-polyglot");
        poly_repo.write_bytes("assets/poly.jpg", &jpeg_zip);
        let poly_md =
            "| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n| `assets/poly.jpg` | CurveBook | Original | MIT | p |\n";
        let poly_err = super::verify_repo_provenance(poly_repo.path(), &[String::from("assets/poly.jpg")], poly_md, "", "CurveBook").unwrap_err();
        assert!(poly_err.contains("Sneaky.ttf") && poly_err.contains("originpro"), "polyglot must catch BOTH font and JPEG meta: {poly_err}");

        // Entity-encoded SVG data:image/ URI caught by assets provenance check (AUD-12-01).
        let svg_repo = TestDir::new("assets-svg-entity");
        svg_repo.write(
            "assets/ui/icon.svg",
            "<svg><image href=\"data&#58;image/svg+xml;base64,PHN2Zz48dGl0bGU+T3JpZ2luUHJvPC90aXRsZT48L3N2Zz4=\"/></svg>",
        );
        let svg_md =
            "| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n| `assets/ui/icon.svg` | CurveBook | Original SVG | MIT | s |\n";
        let svg_err = super::verify_repo_provenance(svg_repo.path(), &[String::from("assets/ui/icon.svg")], svg_md, "", "CurveBook").unwrap_err();
        assert!(svg_err.contains("originpro"), "assets must catch entity-encoded SVG data URI: {svg_err}");

        let trav_dir = TestDir::new("assets-aud12-43");
        trav_dir.write(
            "ASSETS.md",
            "| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n| `../escape.png` | CurveBook | Original | MIT | x |\n",
        );
        trav_dir.write("ATTRIBUTION.md", "");
        trav_dir.write("NOTICE", "CurveBook\n");
        assert!(super::load_repo_asset_catalog(trav_dir.path()).unwrap_err().contains("unsafe asset path"));

        // Non-existent trailing licence note file must fail catalog & repo provenance checks.
        trav_dir.write("assets/ok.png", "png");
        trav_dir.write(
            "ASSETS.md",
            "| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n| `assets/ok.png` | CurveBook | Original | MIT (`assets/missing-LICENSE.txt`) | x |\n",
        );
        assert!(super::load_repo_asset_catalog(trav_dir.path()).is_err());

        #[cfg(unix)]
        {
            let sym_dir = TestDir::new("assets-aud12-43-symparent");
            let outside = sym_dir.join("outside");
            std::fs::create_dir_all(&outside).unwrap();
            std::fs::write(outside.join("icon.png"), b"png").unwrap();
            std::fs::create_dir_all(sym_dir.join("assets")).unwrap();
            std::os::unix::fs::symlink(&outside, sym_dir.join("assets/symparent")).unwrap();
            sym_dir.write(
                "ASSETS.md",
                "| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n| `assets/symparent/icon.png` | CurveBook | Original | MIT | x |\n",
            );
            sym_dir.write("ATTRIBUTION.md", "");
            sym_dir.write("NOTICE", "CurveBook\n");
            assert!(super::load_repo_asset_catalog(sym_dir.path()).unwrap_err().contains("symlink"));
        }

        // Round-3 #216 (AUD-12-43 & AUD-12-01): reject LicenseRef-* / copyleft licence notes,
        // verify ATTRIBUTION.md licence_note_path existence in load_repo_asset_catalog, and catch <style> SVG data URIs.
        for bad_note in ["MIT (`LicenseRef-GPL-3.0`)", "MIT (`LICENSE-GPLv3.txt`)", "MIT (`LICENSE-MPL-2.0.txt`)", "MIT (`LICENSE-CC-BY-SA.txt`)"] {
            let bad_md = format!(
                "| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n| `assets/x.png` | CurveBook | Original | {bad_note} | n |\n"
            );
            assert!(verify_assets(&[String::from("assets/x.png")], &bad_md, "").is_err(), "must reject `{bad_note}`");
        }
        let attr_note_dir = TestDir::new("assets-attr-missing-note");
        attr_note_dir.write("assets/ok.png", "png");
        attr_note_dir.write(
            "ASSETS.md",
            "| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n| `assets/ok.png` | CurveBook | Original | MIT | x |\n",
        );
        attr_note_dir.write(
            "ATTRIBUTION.md",
            "| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n| `assets/ok.png` | CurveBook | Original | MIT (`assets/no-such-LICENSE.txt`) | x |\n",
        );
        attr_note_dir.write("NOTICE", "CurveBook\n");
        assert!(super::load_repo_asset_catalog(attr_note_dir.path()).is_err());

        let style_svg_repo = TestDir::new("assets-svg-style-uri");
        style_svg_repo.write(
            "assets/ui/icon.svg",
            "<svg><style>.a { background: url(data:image/svg+xml;base64,PHN2Zz48dGl0bGU+T3JpZ2luUHJvPC90aXRsZT48L3N2Zz4=); }</style></svg>",
        );
        let style_svg_md =
            "| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n| `assets/ui/icon.svg` | CurveBook | Original SVG | MIT | s |\n";
        let style_err =
            super::verify_repo_provenance(style_svg_repo.path(), &[String::from("assets/ui/icon.svg")], style_svg_md, "", "CurveBook").unwrap_err();
        assert!(style_err.contains("originpro"), "assets must catch <style> data URI: {style_err}");
    }
}
