//! `cargo xtask assets`: every non-code asset must be attributed in `ASSETS.md` (and
//! `ATTRIBUTION.md` for third-party / attribution-bearing works) with an allowlisted permissive
//! licence, and zero font binaries may be tracked in the repository.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use crate::brands::{
    ADAPTED_UPSTREAM_BRAND, BRANDS, PRE_RENAME_ASSET_BRANDS, check_pdf_metadata_checked, check_svg_metadata, inflate_gzip_bounded,
    is_attribution_or_notice_path, is_corpus_path, is_forbidden_bundled_dir, is_plan_prism_path, mask_attributed_upstream_url,
};
use crate::scan_util::{ZIP_CONTAINER_EXTS, collect_dist_files, format_error_list, is_regular_file, looks_like_zip, lower_ext};
use crate::zip_read::{ZipArchive, ZipLimits};

const FONT_EXT: &str = "ttf otf ttc otc woff woff2 eot pfb pfa dfont sfnt bdf pcf fon fnt";
const EXTRA_ZIP_ARCHIVE_EXT: &str = "docx odt jar whl";
const VISUAL_ASSET_EXT: &str = "png jpg jpeg gif webp bmp tif tiff ico icns svg svgz avif pdf eps cur ani";
const ORIGINAL_PROVENANCE_PREFIXES: &str =
    "assets/app-icon/ assets/ui/ assets/icons/ assets/diagrams/ assets/schematics/ assets/thumbnails/ assets/screenshots/ docs/images/";
const ORIGINAL_PROVENANCE_KEYWORDS: &str = "icon logo schematic diagram thumb screenshot archetype baseline cursor toolbar badge splash";
const FORBIDDEN_PROVENANCE_PHRASES: &str = "reference frame|reference video|video frame|reference screenshot|reference app|reference application|copied from|scraped from|extracted from|traced from|captured from|derived from vendor|derived from reference|stock icon|proprietary software|competitor screenshot|vendor screenshot|vendor sample|vendor guide|plan/prism";
const PROVENANCE_SUITE_BRANDS: &[&str] = &["artcraft", "storytold", "storyteller"];
const ASSET_EXT: &str = "png jpg jpeg gif webp bmp tif tiff ico icns svg svgz avif pdf eps icc icm cur ani ase aco gpl ttf otf ttc otc woff woff2 eot pfb pfa dfont sfnt bdf pcf fon fnt curvebook cbtemplate prism pzfx pzf xlsx mp4 wav mp3";
const ALLOWED_ASSET_LICENCES: &str = "MIT Apache-2.0 BSD-2-Clause BSD-3-Clause ISC Zlib Unicode-3.0 CC0-1.0 LicenseRef-Public-Domain CC-BY-4.0 CC-BY-SA-4.0 OFL-1.1 Ubuntu-font-1.0 Bitstream-Vera";
const ATTRIBUTION_REQUIRED_LICENCES: &[&str] = &["CC-BY-4.0", "CC-BY-SA-4.0"];
const MAX_DIST_ASSET_BYTES: usize = 256 * 1024 * 1024;

fn words_has(list: &str, needle: &str) -> bool {
    !needle.is_empty() && list.split_ascii_whitespace().any(|w| w == needle)
}

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
    words_has(FONT_EXT, &lower_ext(path))
}

#[must_use]
pub fn has_font_magic(bytes: &[u8]) -> bool {
    if [b"OTTO", b"ttcf", b"wOFF", b"wOF2"].iter().any(|m| bytes.starts_with(*m)) {
        return true;
    }
    if bytes.len() >= 12 && bytes.starts_with(&[0x00, 0x01, 0x00, 0x00]) {
        let num_tables = u16::from_be_bytes([bytes[4], bytes[5]]);
        if (4..=64).contains(&num_tables) {
            let sel = (15 - num_tables.leading_zeros()) as u16;
            let search = (1u16 << sel).saturating_mul(16);
            return u16::from_be_bytes([bytes[6], bytes[7]]) == search
                && u16::from_be_bytes([bytes[8], bytes[9]]) == sel
                && u16::from_be_bytes([bytes[10], bytes[11]]) == num_tables.saturating_mul(16).saturating_sub(search);
        }
    }
    false
}

fn is_zip_archive_ext(ext: &str) -> bool {
    ZIP_CONTAINER_EXTS.contains(&ext) || words_has(EXTRA_ZIP_ARCHIVE_EXT, ext)
}
#[must_use]
pub fn is_asset(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    ["assets/", "docs/images/", "examples/"].iter().any(|d| lower.starts_with(d)) || words_has(ASSET_EXT, &lower_ext(path))
}
#[must_use]
pub fn is_original_only_visual_asset(path: &str) -> bool {
    let ext = lower_ext(path);
    if !words_has(VISUAL_ASSET_EXT, &ext) {
        return false;
    }
    let lower = path.to_ascii_lowercase();
    matches!(ext.as_str(), "cur" | "ani")
        || ORIGINAL_PROVENANCE_PREFIXES.split_ascii_whitespace().any(|p| lower.starts_with(p))
        || ORIGINAL_PROVENANCE_KEYWORDS.split_ascii_whitespace().any(|kw| lower.contains(kw))
}
fn is_placeholder_cell(cell: &str) -> bool {
    matches!(cell.trim().to_ascii_lowercase().as_str(), "" | "-" | "--" | "---" | "?" | "n/a" | "na" | "none" | "tbd" | "unknown")
}

fn is_safe_relative_licence_note(note: &str) -> bool {
    if note.is_empty()
        || note.starts_with('/')
        || note.contains(['\\', '\0'])
        || note.chars().any(char::is_whitespace)
        || !note.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'-' | b'_'))
        || Path::new(note).components().any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
    {
        return false;
    }
    let lower = note.to_ascii_lowercase();
    let copyleft = lower.contains("licenseref-")
        || lower.contains("cc-by-sa")
        || lower.contains("cc_by_sa")
        || lower.split(|c: char| !c.is_ascii_alphanumeric()).any(|tok| {
            ["gpl", "agpl", "lgpl", "mpl", "sspl"]
                .iter()
                .any(|p| tok == *p || tok.strip_prefix(p).is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit() || c == 'v')))
        });
    let fname = lower.rsplit('/').next().unwrap_or(lower.as_str());
    !copyleft
        && (["license", "licence", "copying", "notice"].iter().any(|p| fname.starts_with(p)) || fname.ends_with(".txt") || fname.ends_with(".md"))
}

fn split_trailing_licence_note(raw: &str) -> (&str, Option<&str>) {
    let trimmed = raw.trim();
    trimmed
        .strip_suffix("`)")
        .and_then(|s| s.rsplit_once(" (`"))
        .filter(|(_, n)| is_safe_relative_licence_note(n))
        .map_or((trimmed, None), |(h, n)| (h.trim(), Some(n)))
}

fn validate_licence_expression(raw: &str) -> Result<Vec<String>, String> {
    let expr = split_trailing_licence_note(raw).0;
    if expr.is_empty() {
        return Err(String::from("empty licence cell"));
    }
    let (mut ids, mut expect_id) = (Vec::new(), true);
    for tok in expr.split_whitespace().map(|t| t.trim_matches(['(', ')'])).filter(|t| !t.is_empty()) {
        if expect_id {
            let upper = tok.to_ascii_uppercase();
            if upper.starts_with("GPL") || upper.starts_with("AGPL") || upper.starts_with("LGPL") {
                return Err(format!("copyleft licence `{tok}` is forbidden"));
            }
            if !words_has(ALLOWED_ASSET_LICENCES, tok) {
                return Err(format!("disallowed or unknown licence `{tok}`"));
            }
            ids.push(tok.to_owned());
            expect_id = false;
        } else if matches!(tok, "OR" | "AND") {
            expect_id = true;
        } else {
            return Err(format!("unexpected token `{tok}` in licence expression `{expr}`"));
        }
    }
    if expect_id || ids.is_empty() { Err(format!("incomplete licence expression `{expr}`")) } else { Ok(ids) }
}

fn parse_attribution_table(md: &str, table_name: &str) -> Result<BTreeMap<String, AssetRow>, String> {
    let mut rows = BTreeMap::new();
    for (idx, line) in md.lines().enumerate() {
        let line_no = idx + 1;
        let Some(cells): Option<Vec<&str>> =
            line.trim().strip_prefix('|').and_then(|s| s.strip_suffix('|')).map(|i| i.split('|').map(str::trim).collect())
        else {
            continue;
        };
        let is_sep = !cells.is_empty() && cells.iter().all(|c| !c.is_empty() && c.bytes().all(|b| matches!(b, b'-' | b':' | b' ')));
        let is_hdr =
            cells.first().is_some_and(|s| s.eq_ignore_ascii_case("asset")) && cells.get(3).is_some_and(|s| s.to_ascii_lowercase().contains("licen"));
        if is_sep || is_hdr {
            continue;
        }
        let [asset_cell, author_cell, source_cell, licence_cell, ..] = cells.as_slice() else {
            return Err(format!("{table_name}:{line_no}: expected at least 4 table columns (Asset | Author | Source | Licence)"));
        };
        let Some(path) = asset_cell.strip_prefix('`').and_then(|s| s.strip_suffix('`')).map(str::trim).filter(|p| !p.is_empty()) else {
            return Err(format!("{table_name}:{line_no}: asset cell `{asset_cell}` must be a non-empty backticked path"));
        };
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
        rows.insert(
            path.to_owned(),
            AssetRow {
                path: path.to_owned(),
                author: (*author_cell).to_owned(),
                source: (*source_cell).to_owned(),
                licence_tokens,
                licence_note_path: licence_note.map(str::to_owned),
                notes: cells.get(4).map(|s| (*s).trim().to_owned()).unwrap_or_default(),
            },
        );
    }
    Ok(rows)
}

fn row_requires_attribution_entry(row: &AssetRow) -> bool {
    !row.author.to_ascii_lowercase().contains("curvebook") || row.licence_tokens.iter().any(|t| ATTRIBUTION_REQUIRED_LICENCES.contains(&t.as_str()))
}

fn has_ascii_word(lower: &str, word: &str) -> bool {
    let (bytes, mut start) = (lower.as_bytes(), 0usize);
    while let Some(rel) = lower.get(start..).and_then(|s| s.find(word)) {
        let (pos, end) = (start + rel, start + rel + word.len());
        if (pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric()) && (end == bytes.len() || !bytes[end].is_ascii_alphanumeric()) {
            return true;
        }
        start = pos + lower.get(pos..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
    }
    false
}

fn find_forbidden_provenance_brand(lower: &str) -> Option<&'static str> {
    BRANDS
        .split_ascii_whitespace()
        .chain(PRE_RENAME_ASSET_BRANDS.split_ascii_whitespace())
        .chain(PROVENANCE_SUITE_BRANDS.iter().copied())
        .find(|b| has_ascii_word(lower, b))
}

fn check_row_safety(row: &AssetRow) -> Result<(), String> {
    for (field, val) in [("author", row.author.as_str()), ("source", row.source.as_str()), ("notes", row.notes.as_str())] {
        let lower = val.to_ascii_lowercase();
        if let Some(phrase) = FORBIDDEN_PROVENANCE_PHRASES.split('|').find(|p| lower.contains(p)) {
            return Err(format!("{}: {field} contains forbidden non-original provenance phrase `{phrase}`", row.path));
        }
        if let Some(brand) = find_forbidden_provenance_brand(&lower) {
            return Err(format!("{}: {field} references forbidden external product or vendor `{brand}`", row.path));
        }
    }
    Ok(())
}

fn check_licence_note_file(root: &Path, row: &AssetRow) -> Result<(), String> {
    if let Some(note_path) = &row.licence_note_path {
        reject_symlink_path_components(root, note_path, "licence note file")?;
        if !is_regular_file(&root.join(note_path)) {
            return Err(format!("{}: referenced licence note `{note_path}` must exist as a regular non-symlink file", row.path));
        }
    }
    Ok(())
}

fn extract_backticked_asset_refs(mut rest: &str) -> Vec<&str> {
    let mut refs = Vec::new();
    while let Some((_, after)) = rest.split_once('`') {
        let Some((tok, tail)) = after.split_once('`') else { break };
        if tok.trim().contains('/') {
            refs.push(tok.trim());
        }
        rest = tail;
    }
    refs
}

fn is_direct_original_source(source: &str) -> bool {
    let l = source.to_ascii_lowercase();
    ["original", "cargo xtask screenshots", "generated in code", "rendered in-process"].iter().any(|k| l.contains(k))
        || (l.contains("curvebook")
            && ["approved", "artwork", "generated", "created", "drawn", "authored", "rendered", "user-supplied"].iter().any(|k| l.contains(k)))
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
    if [row.source.as_str(), row.notes.as_str()]
        .iter()
        .any(|s| s.to_ascii_lowercase().contains("http://") || s.to_ascii_lowercase().contains("https://"))
    {
        return Err(format!("{}: UI icon, schematic, thumbnail, or screenshot baseline cannot cite external URLs in ASSETS.md", row.path));
    }
    let parents = extract_backticked_asset_refs(&row.source);
    if !parents.is_empty() {
        for p in parents {
            let parent = all_rows.get(p).ok_or_else(|| format!("{}: cites parent asset `{p}` missing from ASSETS.md", row.path))?;
            verify_original_provenance_chain(parent, all_rows, visited)?;
        }
    } else if !is_direct_original_source(&row.source) {
        return Err(format!(
            "{}: UI icon, schematic, thumbnail, or screenshot baseline must declare original CurveBook provenance in ASSETS.md (source: `{}`)",
            row.path, row.source
        ));
    }
    visited.remove(&row.path);
    Ok(())
}

pub fn verify_assets(files: &[String], assets_md: &str, attribution_md: &str) -> Result<usize, String> {
    let fonts: Vec<&str> = files.iter().map(String::as_str).filter(|f| is_font_file(f)).collect();
    if !fonts.is_empty() {
        return Err(format_error_list("committed font file(s) forbidden by zero-bundled-fonts policy", &fonts));
    }
    let (asset_rows, attr_rows, mut errors) =
        (parse_attribution_table(assets_md, "ASSETS.md")?, parse_attribution_table(attribution_md, "ATTRIBUTION.md")?, Vec::new());
    for row in asset_rows.values().chain(attr_rows.values()) {
        if let Err(err) = check_row_safety(row) {
            errors.push(err);
        }
    }
    for row in asset_rows.values().filter(|r| is_original_only_visual_asset(&r.path)) {
        if let Err(err) = verify_original_provenance_chain(row, &asset_rows, &mut BTreeSet::new()) {
            errors.push(err);
        }
    }
    let mut count = 0usize;
    for file in files.iter().filter(|f| is_asset(f)) {
        count += 1;
        match asset_rows.get(file.as_str()) {
            None => errors.push(format!("{file}: missing valid ASSETS.md table row (author, source, allowlisted licence)")),
            Some(row) if row_requires_attribution_entry(row) && !attr_rows.contains_key(&row.path) => {
                errors.push(format!("{file}: third-party or attribution-licensed asset lacks an ATTRIBUTION.md row"))
            }
            Some(_) => {}
        }
    }
    errors.sort();
    errors.dedup();
    if errors.is_empty() { Ok(count) } else { Err(format_error_list("asset attribution error(s)", &errors)) }
}

fn check_document_prose(doc_name: &str, text: &str, allow_packaged: bool) -> Result<(), String> {
    let masked = mask_attributed_upstream_url(doc_name, text, allow_packaged);
    let check_vc = !is_attribution_or_notice_path(doc_name, allow_packaged);
    for (idx, line) in masked.lines().enumerate() {
        let lower = line.to_ascii_lowercase();
        if let Some(p) = FORBIDDEN_PROVENANCE_PHRASES.split('|').find(|p| lower.contains(p)) {
            return Err(format!("{doc_name}:{}: contains forbidden provenance phrase `{p}`", idx + 1));
        }
        if let Some(b) = find_forbidden_provenance_brand(&lower) {
            return Err(format!("{doc_name}:{}: references forbidden vendor or suite `{b}`", idx + 1));
        }
        if check_vc && has_ascii_word(&lower, ADAPTED_UPSTREAM_BRAND) {
            return Err(format!("{doc_name}:{}: references adapted source brand `{ADAPTED_UPSTREAM_BRAND}` outside ATTRIBUTION.md/NOTICE", idx + 1));
        }
    }
    Ok(())
}

fn scan_repo_zip_for_fonts(display_path: &str, bytes: &[u8], depth: usize, errors: &mut Vec<String>) {
    if depth >= 2 {
        return errors.push(format!("{display_path}: nested archive depth exceeded while inspecting for bundled fonts"));
    }
    match ZipArchive::from_bytes_with_limits(bytes, ZipLimits::default()) {
        Ok(zip) => {
            for (name, entry_bytes) in zip.iter() {
                let nested = format!("{display_path}!{name}");
                if is_font_file(name) || has_font_magic(entry_bytes) {
                    errors.push(format!("{nested}: bundled font file forbidden by zero-bundled-fonts policy"));
                } else if is_zip_archive_ext(&lower_ext(name)) || looks_like_zip(entry_bytes) {
                    scan_repo_zip_for_fonts(&nested, entry_bytes, depth + 1, errors);
                }
            }
        }
        Err(err) => errors.push(format!("{display_path}: corrupt ZIP archive while checking bundled fonts: {err}")),
    }
}

fn check_asset_embedded_metadata(display_path: &str, bytes: &[u8], violations: &mut Vec<String>) {
    let decode_svg = |b: &[u8], kind: &str| {
        crate::text_decode::decode_utf8_or_utf16(b)
            .ok()
            .flatten()
            .map(|svg| check_svg_metadata(display_path, &svg))
            .ok_or_else(|| format!("{display_path}: invalid UTF-8/UTF-16 in {kind}"))
    };
    let res = match lower_ext(display_path).as_str() {
        "pdf" => check_pdf_metadata_checked(display_path, bytes),
        "svg" => decode_svg(bytes, "SVG"),
        "svgz" => inflate_gzip_bounded(bytes)
            .map_err(|()| format!("{display_path}: corrupt or oversized .svgz gzip payload"))
            .and_then(|raw| decode_svg(&raw, ".svgz")),
        _ => Ok(Vec::new()),
    };
    match res {
        Ok(hits) => hits.into_iter().for_each(|h| violations.push(format!("{}:{}: embedded metadata mentions `{}`", h.path, h.line, h.brand))),
        Err(e) => violations.push(e),
    }
}

fn reject_symlink_path_components(root: &Path, rel: &str, label: &str) -> Result<(), String> {
    let mut cur = root.to_path_buf();
    for comp in Path::new(rel).components() {
        if let Component::Normal(seg) = comp {
            cur.push(seg);
            if std::fs::symlink_metadata(&cur).map_err(|e| format!("{label} `{rel}`: {e}"))?.file_type().is_symlink() {
                return Err(format!("{label} `{rel}` must not traverse symlink `{}`", cur.display()));
            }
        }
    }
    Ok(())
}

fn read_provenance_docs(root: &Path) -> Result<(String, String, String), String> {
    let r = |n: &str| std::fs::read_to_string(root.join(n)).map_err(|e| format!("{n}: {e}"));
    Ok((r("ASSETS.md")?, r("ATTRIBUTION.md")?, r("NOTICE")?))
}

pub fn verify_repo_provenance(root: &Path, files: &[String], assets_md: &str, attribution_md: &str, notice: &str) -> Result<usize, String> {
    let count = verify_assets(files, assets_md, attribution_md)?;
    let mut errors = Vec::new();
    for path in files {
        if is_plan_prism_path(path) {
            errors.push(format!("{path}: committed `plan/prism` reference directory is forbidden"));
        }
        if is_corpus_path(path) {
            errors.push(format!("{path}: committed `corpus/` directory is forbidden"));
        }
    }
    let tracked: BTreeSet<&str> = files.iter().map(String::as_str).collect();
    let (asset_rows, attr_rows) = (parse_attribution_table(assets_md, "ASSETS.md")?, parse_attribution_table(attribution_md, "ATTRIBUTION.md")?);
    asset_rows
        .keys()
        .filter(|p| !tracked.contains(p.as_str()))
        .for_each(|p| errors.push(format!("ASSETS.md: orphan entry `{p}` does not match any tracked file")));
    for path in attr_rows.keys() {
        if !tracked.contains(path.as_str()) {
            errors.push(format!("ATTRIBUTION.md: orphan entry `{path}` does not match any tracked file"));
        }
        if !asset_rows.contains_key(path.as_str()) {
            errors.push(format!("ATTRIBUTION.md: entry `{path}` missing corresponding ASSETS.md row"));
        }
    }
    for row in asset_rows.values().chain(attr_rows.values()) {
        if let Err(err) = check_licence_note_file(root, row) {
            errors.push(err);
        }
    }
    for (name, body) in [("ASSETS.md", assets_md), ("ATTRIBUTION.md", attribution_md), ("NOTICE", notice)] {
        if let Err(err) = check_document_prose(name, body, false) {
            errors.push(err);
        }
    }
    for path in files {
        let ext = lower_ext(path);
        let (is_arch, is_meta) = (is_zip_archive_ext(&ext), matches!(ext.as_str(), "svg" | "svgz" | "pdf"));
        let Ok(bytes) = curvebook_platform::fs::read_regular_file_bytes(&root.join(path), MAX_DIST_ASSET_BYTES) else {
            if is_arch || is_meta || is_asset(path) {
                errors.push(format!("{path}: unreadable or oversized asset file"));
            }
            continue;
        };
        if has_font_magic(&bytes) {
            errors.push(format!("{path}: committed font binary (font magic header) forbidden by zero-bundled-fonts policy"));
            continue;
        }
        if is_arch || looks_like_zip(&bytes) {
            scan_repo_zip_for_fonts(path, &bytes, 0, &mut errors);
        }
        if is_meta {
            check_asset_embedded_metadata(path, &bytes, &mut errors);
        }
    }
    errors.sort();
    errors.dedup();
    if errors.is_empty() { Ok(count) } else { Err(format_error_list("asset provenance error(s)", &errors)) }
}

pub fn run(root: &Path) -> Result<(), String> {
    let files = crate::repo_files(root)?;
    let (md, attr_md, notice) = read_provenance_docs(root)?;
    let count = verify_repo_provenance(root, &files, &md, &attr_md, &notice)?;
    println!("assets: all {count} asset files attributed in ASSETS.md with clean provenance and allowlisted licences");
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct RepoAssetCatalog {
    allowed_sha256: BTreeSet<[u8; 32]>,
}

fn validate_catalog_rel_path(rel: &str) -> Result<(), String> {
    if rel.is_empty() || rel.starts_with('/') || rel.contains(['\\', '\0']) || Path::new(rel).components().any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(format!("ASSETS.md: unsafe asset path `{rel}`"));
    }
    Ok(())
}

pub(crate) fn load_repo_asset_catalog(root: &Path) -> Result<RepoAssetCatalog, String> {
    let (assets_md, attr_md, notice) = read_provenance_docs(root)?;
    for (name, body) in [("ASSETS.md", assets_md.as_str()), ("ATTRIBUTION.md", attr_md.as_str()), ("NOTICE", notice.as_str())] {
        check_document_prose(name, body, false)?;
    }
    let (asset_rows, attr_rows) = (parse_attribution_table(&assets_md, "ASSETS.md")?, parse_attribution_table(&attr_md, "ATTRIBUTION.md")?);
    for row in asset_rows.values().chain(attr_rows.values()) {
        check_row_safety(row)?;
        check_licence_note_file(root, row)?;
    }
    let mut allowed_sha256 = BTreeSet::new();
    for row in asset_rows.values() {
        validate_catalog_rel_path(&row.path)?;
        if row_requires_attribution_entry(row) && !attr_rows.contains_key(&row.path) {
            return Err(format!("{}: third-party or attribution-licensed asset lacks an ATTRIBUTION.md row", row.path));
        }
        if is_original_only_visual_asset(&row.path) {
            verify_original_provenance_chain(row, &asset_rows, &mut BTreeSet::new())?;
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
    Ok(RepoAssetCatalog { allowed_sha256 })
}

pub(crate) fn scan_dist_artefacts(catalog: &RepoAssetCatalog, dist_dir: &Path) -> Result<(usize, Vec<String>), String> {
    if !dist_dir.exists() {
        return Err(format!("dist directory `{}` does not exist", dist_dir.display()));
    }
    let mut rel_files = Vec::new();
    collect_dist_files(dist_dir, dist_dir, &mut rel_files)?;
    rel_files.sort();
    let (mut checked, mut violations) = (0usize, Vec::new());
    for rel in rel_files {
        if is_corpus_path(&rel) {
            violations.push(format!("{rel}: bundled `corpus/` directory is forbidden"));
            continue;
        }
        if is_forbidden_bundled_dir(&rel) {
            violations.push(format!("{rel}: bundled reference or historical audit directory is forbidden"));
            continue;
        }
        let bytes = curvebook_platform::fs::read_regular_file_bytes(&dist_dir.join(&rel), MAX_DIST_ASSET_BYTES)
            .map_err(|e| format!("read dist file `{rel}`: {e}"))?;
        scan_dist_asset_bytes(catalog, &rel, &bytes, 0, &mut checked, &mut violations);
    }
    violations.sort();
    violations.dedup();
    Ok((checked, violations))
}

fn scan_dist_asset_bytes(
    catalog: &RepoAssetCatalog,
    display_path: &str,
    bytes: &[u8],
    depth: usize,
    checked: &mut usize,
    violations: &mut Vec<String>,
) {
    if is_font_file(display_path) || has_font_magic(bytes) {
        *checked += 1;
        return violations.push(format!("{display_path}: bundled font file forbidden by zero-bundled-fonts policy"));
    }
    check_bundled_attribution_doc(display_path, bytes, violations);
    if is_asset(display_path) {
        *checked += 1;
        check_asset_embedded_metadata(display_path, bytes, violations);
        if !catalog.allowed_sha256.contains(&crate::release_scan::sha256_bytes(bytes)) {
            violations
                .push(format!("{display_path}: unattributed asset in release package (SHA-256 does not match any attributed entry in ASSETS.md)"));
        }
    }
    if is_zip_archive_ext(&lower_ext(display_path)) || looks_like_zip(bytes) {
        if depth >= 2 {
            return violations.push(format!("{display_path}: nested archive depth exceeded (max depth 2)"));
        }
        match ZipArchive::from_bytes_with_limits(bytes, ZipLimits::default()) {
            Ok(zip) => {
                for (name, entry_bytes) in zip.iter() {
                    let nested = format!("{display_path}!{name}");
                    if is_corpus_path(name) {
                        violations.push(format!("{nested}: bundled `corpus/` directory is forbidden"));
                        continue;
                    }
                    if is_forbidden_bundled_dir(name) {
                        violations.push(format!("{nested}: bundled reference or historical audit directory is forbidden"));
                    } else {
                        scan_dist_asset_bytes(catalog, &nested, entry_bytes, depth + 1, checked, violations);
                    }
                }
            }
            Err(err) => violations.push(format!("{display_path}: cannot inspect nested archive: {err}")),
        }
    }
}

fn check_bundled_attribution_doc(display_path: &str, bytes: &[u8], violations: &mut Vec<String>) {
    let entry = display_path.rsplit('!').next().unwrap_or(display_path);
    let Some(name) = Path::new(entry).file_name().and_then(|n| n.to_str()).filter(|n| matches!(*n, "ASSETS.md" | "ATTRIBUTION.md" | "NOTICE")) else {
        return;
    };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return violations.push(format!("{display_path}: bundled {name} is not valid UTF-8"));
    };
    if let Err(err) = check_document_prose(display_path, text, true) {
        violations.push(err);
    }
    if matches!(name, "ASSETS.md" | "ATTRIBUTION.md") {
        match parse_attribution_table(text, display_path) {
            Ok(rows) => rows.values().for_each(|r| {
                if let Err(e) = check_row_safety(r) {
                    violations.push(format!("{display_path}: {e}"));
                }
            }),
            Err(err) => violations.push(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan_util::test_support::TestDir;

    fn zip_single(name: &str, body: &[u8]) -> Vec<u8> {
        let mut zw = crate::zip_read::TestZipWriter::new();
        zw.add_file(name, body.to_vec()).unwrap();
        zw.finish().unwrap()
    }

    #[test]
    fn enforces_fonts_licences_attribution_and_original_provenance() {
        for font in ["a.ttf", "b.otf", "c.woff2", "d.ttc", "e.sfnt"] {
            assert!(is_font_file(font) && verify_assets(&[font.to_owned()], "", "").unwrap_err().contains("committed font file"));
        }
        assert!(has_font_magic(b"OTTO\x00\x01") && has_font_magic(b"wOF2\x00\x01") && is_asset("ASSETS/APP-ICON/ICON.PNG"));
        assert!(is_original_only_visual_asset("docs/images/welcome.png") && is_forbidden_bundled_dir("docs/history/audit/x"));

        let good_files: Vec<_> = ["assets/ui/xy.svg", "assets/thumbnails/t.png", "docs/images/w.png"].into_iter().map(String::from).collect();
        let good_md = "| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n\
            | `assets/ui/xy.svg` | CurveBook contributors | Original SVG | MIT OR Apache-2.0 | icon |\n\
            | `assets/thumbnails/t.png` | CurveBook contributors | Rendered from `assets/ui/xy.svg` | MIT OR Apache-2.0 | thumb |\n\
            | `docs/images/w.png` | CurveBook contributors | Generated by cargo xtask screenshots | MIT OR Apache-2.0 | shot |\n";
        assert_eq!(verify_assets(&good_files, good_md, "").unwrap(), 3);

        for (files, md, attr, needle) in [
            (vec!["assets/third.svg"], "| `assets/third.svg` | Upstream | https://e.org | CC-BY-4.0 | t |\n", "", "ATTRIBUTION.md"),
            (
                vec!["assets/ui/a.svg"],
                "| `assets/ui/a.svg` | Upstream | https://e.org | CC-BY-4.0 | t |\n",
                "| `assets/ui/a.svg` | Upstream | https://e.org | CC-BY-4.0 | t |\n",
                "CurveBook authorship",
            ),
            (
                vec!["docs/images/f.png"],
                "| `docs/images/f.png` | CurveBook | Derived from reference frame 1 | MIT | s |\n",
                "",
                "non-original provenance phrase",
            ),
            (
                vec!["assets/ui/a.png", "assets/ui/b.png"],
                "| `assets/ui/a.png` | CurveBook | From `assets/ui/b.png` | MIT | a |\n| `assets/ui/b.png` | CurveBook | From `assets/ui/a.png` | MIT | b |\n",
                "",
                "circular",
            ),
            (vec!["assets/x.png"], "| `assets/x.png` | CurveBook | Original | GPL-3.0-only | x |\n", "", "copyleft"),
            (vec!["assets/x.png"], "| `assets/x.png` | CurveBook | Original | MIT (`GPL-3.0-only`) | x |\n", "", "unexpected token"),
        ] {
            let err = verify_assets(&files.into_iter().map(String::from).collect::<Vec<_>>(), md, attr).unwrap_err();
            assert!(err.contains(needle), "expected `{needle}` in `{err}`");
        }
    }

    #[test]
    fn enforces_repo_provenance_zip_fonts_pdf_svg_and_dist_catalog() {
        let root = TestDir::new("assets-prov");
        root.write("assets/ui/icon.svg", "<svg><title>CurveBook</title></svg>");
        let ok_md = "| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n| `assets/ui/icon.svg` | CurveBook contributors | Original SVG | MIT | ok |\n";
        assert_eq!(verify_repo_provenance(root.path(), &[String::from("assets/ui/icon.svg")], ok_md, "", "CurveBook").unwrap(), 1);

        let url = format!("https://github.com/{}/{}", ["sto", "ry", "told"].concat(), ["vec", "tor", "craft"].concat());
        assert_eq!(check_document_prose("ATTRIBUTION.md", &format!("## Attributed Source Code\nFrom VectorCraft (`{url}`).\n"), false), Ok(()));
        assert!(check_document_prose("ASSETS.md", &format!("From {url}\n"), false).is_err());

        root.write_bytes("examples/book.xlsx", &zip_single("xl/fonts/Hidden.ttf", b"font"));
        root.write_bytes("assets/renamed.bin", b"wOF2font");
        root.write_bytes("assets/ui/bad.pdf", b"%PDF-1.4\n<< /Creator (GraphPad Prism) >>\n");
        let md = "| `examples/book.xlsx` | CurveBook | Original | MIT | x |\n| `assets/renamed.bin` | CurveBook | Original | MIT | b |\n| `assets/ui/bad.pdf` | CurveBook | Original | MIT | p |\n";
        let err = verify_repo_provenance(
            root.path(),
            &["examples/book.xlsx", "assets/renamed.bin", "assets/ui/bad.pdf"].map(String::from),
            md,
            "",
            "CurveBook",
        )
        .unwrap_err();
        assert!(err.contains("Hidden.ttf") && err.contains("font magic") && (err.contains("graphpad") || err.contains("prism")), "{err}");

        root.write("ASSETS.md", "| `../escape.png` | CurveBook | Original | MIT | x |\n");
        root.write("ATTRIBUTION.md", "");
        root.write("NOTICE", "CurveBook\n");
        assert!(load_repo_asset_catalog(root.path()).unwrap_err().contains("unsafe asset path"));

        let dist = TestDir::new("assets-dist");
        dist.write_bytes("corrupt.zip", b"NOT_A_ZIP");
        let (_, errs) = scan_dist_artefacts(&RepoAssetCatalog { allowed_sha256: Default::default() }, dist.path()).unwrap();
        assert!(errs.iter().any(|e| e.contains("corrupt.zip")), "{errs:?}");
    }

    #[test]
    fn corpus_gate_rejects_tracked_and_packaged_corpus_while_allowing_untracked_local_files() {
        // 1. Tracked corpus/ file fails verify_repo_provenance.
        let tracked_root = TestDir::new("assets-corpus-tracked");
        tracked_root.write("corpus/sample.txt", "clean");
        let tracked_err = super::verify_repo_provenance(tracked_root.path(), &[String::from("corpus/sample.txt")], "", "", "CurveBook").unwrap_err();
        assert!(tracked_err.contains("corpus/"), "tracked corpus file must fail assets gate: {tracked_err}");

        // 2. Bundled corpus/ file and archived corpus/ entry fail scan_dist_artefacts.
        let dist_root = TestDir::new("assets-corpus-dist");
        dist_root.write("app/corpus/sample.txt", "clean");
        dist_root.write_bytes("pkg.zip", &zip_single("corpus/nested.txt", b"clean"));
        let cat = super::RepoAssetCatalog { allowed_sha256: std::collections::BTreeSet::new() };
        let (_, dist_errs) = super::scan_dist_artefacts(&cat, dist_root.path()).unwrap();
        assert!(dist_errs.iter().any(|e| e.contains("app/corpus/sample.txt") && e.contains("corpus/")), "{dist_errs:?}");
        assert!(dist_errs.iter().any(|e| e.contains("pkg.zip!corpus/nested.txt") && e.contains("corpus/")), "{dist_errs:?}");

        // 3. Untracked gitignored local corpus/ files must NOT fail assets::run.
        let git_root = TestDir::git_repo("assets-corpus-untracked");
        git_root.write(".gitignore", "/corpus\n");
        git_root.write("ASSETS.md", "| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n");
        git_root.write("ATTRIBUTION.md", "| Asset | Author | Source | Licence | Notes |\n|---|---|---|---|---|\n");
        git_root.write("NOTICE", "CurveBook\n");
        git_root.git_add_all();
        git_root.write("corpus/third-party-prism/local.pzfx", "<PrismFile/>");
        assert!(super::run(git_root.path()).is_ok(), "untracked local corpus/ files must not fail assets gate");
    }
}
