//! `cargo xtask assets`: every non-code asset must be attributed in `ASSETS.md` (and
//! `ATTRIBUTION.md` for third-party / attribution-bearing works) with an allowlisted permissive
//! licence, and zero font binaries may be tracked in the repository.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const FONT_EXT: &[&str] = &["ttf", "otf", "ttc", "woff", "woff2", "eot", "pfb"];

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
    "artcraft",
    "storytold",
    "storyteller",
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
    "woff",
    "woff2",
    "eot",
    "pfb",
    "lumencraft",
    "lctemplate",
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
    notes: String,
}

fn lower_ext(path: &str) -> String {
    Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default()
}

#[must_use]
pub fn is_font_file(path: &str) -> bool {
    let ext = lower_ext(path);
    FONT_EXT.contains(&ext.as_str())
}

#[must_use]
pub fn is_asset(path: &str) -> bool {
    let ext = lower_ext(path);
    ASSET_DIRS.iter().any(|d| path.starts_with(d)) || ASSET_EXT.contains(&ext.as_str())
}

#[must_use]
pub fn is_original_only_visual_asset(path: &str) -> bool {
    let ext = lower_ext(path);
    if !VISUAL_ASSET_EXT.contains(&ext.as_str()) {
        return false;
    }
    if matches!(ext.as_str(), "cur" | "ani") || ORIGINAL_PROVENANCE_PREFIXES.iter().any(|prefix| path.starts_with(prefix)) {
        return true;
    }
    let lower = path.to_ascii_lowercase();
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
    if !trimmed.starts_with('|') || !trimmed.ends_with('|') {
        return None;
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    Some(inner.split('|').map(str::trim).collect())
}

fn strip_trailing_licence_note(raw: &str) -> &str {
    let trimmed = raw.trim();
    if trimmed.ends_with("`)")
        && let Some(idx) = trimmed.rfind(" (`")
    {
        return trimmed[..idx].trim();
    }
    trimmed
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
        let licence_tokens = validate_licence_expression(licence_cell).map_err(|e| format!("{table_name}:{line_no} (`{path}`): {e}"))?;
        if rows.contains_key(path) {
            return Err(format!("{table_name}:{line_no}: duplicate row for `{path}`"));
        }
        let notes = cells.get(4).map(|s| (*s).trim().to_owned()).unwrap_or_default();
        rows.insert(
            path.to_owned(),
            AssetRow { path: path.to_owned(), author: (*author_cell).to_owned(), source: (*source_cell).to_owned(), licence_tokens, notes },
        );
    }
    Ok(rows)
}

fn row_requires_attribution_entry(row: &AssetRow) -> bool {
    let third_party_author = !row.author.to_ascii_lowercase().contains("lumencraft");
    let attr_licence = row.licence_tokens.iter().any(|tok| ATTRIBUTION_REQUIRED_LICENCES.contains(&tok.as_str()));
    third_party_author || attr_licence
}

fn has_ascii_word(lower: &str, word: &str) -> bool {
    let bytes = lower.as_bytes();
    let wlen = word.len();
    let mut start = 0usize;
    while let Some(rel) = lower[start..].find(word) {
        let pos = start + rel;
        let end = pos + wlen;
        let left_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
        let right_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if left_ok && right_ok {
            return true;
        }
        start = pos + 1;
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
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else {
            break;
        };
        let token = after[..end].trim();
        if token.contains('/') {
            refs.push(token);
        }
        rest = &after[end + 1..];
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
    lower.contains("lumencraft")
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
    if !row.author.to_ascii_lowercase().contains("lumencraft") {
        return Err(format!(
            "{}: UI icon, schematic, thumbnail, or screenshot baseline must declare LumenCraft authorship (got `{}`)",
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
            "{}: UI icon, schematic, thumbnail, or screenshot baseline must declare original LumenCraft provenance in ASSETS.md (source: `{}`)",
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
        return Err(format!("{} committed font file(s) forbidden by zero-bundled-fonts policy:\n  {}", fonts.len(), fonts.join("\n  ")));
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
    if errors.is_empty() { Ok(asset_count) } else { Err(format!("{} asset attribution error(s):\n  {}", errors.len(), errors.join("\n  "))) }
}

pub fn run(root: &Path) -> Result<(), String> {
    let files = crate::repo_files(root)?;
    let md = std::fs::read_to_string(root.join("ASSETS.md")).map_err(|e| format!("ASSETS.md: {e}"))?;
    let attr_md = std::fs::read_to_string(root.join("ATTRIBUTION.md")).map_err(|e| format!("ATTRIBUTION.md: {e}"))?;
    let count = verify_assets(&files, &md, &attr_md)?;
    println!("assets: all {count} asset files attributed in ASSETS.md with allowlisted licences");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{is_asset, is_font_file, is_original_only_visual_asset, missing, verify_assets};

    #[test]
    fn detects_unattributed_assets() {
        let files =
            vec![String::from("assets/app-icon/lumencraft.svg"), String::from("crates/stats/src/lib.rs"), String::from("docs/images/plot.png")];
        let md = "| `assets/app-icon/lumencraft.svg` | LumenCraft contributors | original | MIT OR Apache-2.0 |";
        assert_eq!(missing(&files, md), vec![String::from("docs/images/plot.png")]);
        assert!(!is_asset("crates/stats/src/lib.rs"));
        assert!(is_asset("examples/dose.lumencraft"));
    }

    #[test]
    fn rejects_proprietary_or_placeholder_rows_and_prose_mentions() {
        let files = vec![String::from("assets/plain.png"), String::from("assets/prose.png")];
        let md = "\
See also `assets/prose.png` in prose.
| Asset | Author | Source | Licence | Notes |
|---|---|---|---|---|
| `assets/plain.png` | unknown | copied from a vendor web page | Proprietary, all rights reserved | bad |
";
        let miss = missing(&files, md);
        assert_eq!(miss, vec![String::from("assets/plain.png"), String::from("assets/prose.png")]);
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
            assert!(is_asset(path), "expected is_asset({path}) to be true");
        }

        for font in ["fonts/a.ttf", "fonts/b.otf", "fonts/c.ttc", "fonts/d.woff", "fonts/e.woff2", "fonts/f.eot", "fonts/g.pfb"] {
            assert!(is_font_file(font), "expected is_font_file({font})");
            let files = vec![font.to_owned()];
            let md = format!("| `{font}` | Some Foundry | local | OFL-1.1 |\n");
            let attr = format!("| `{font}` | Some Foundry | local | OFL-1.1 |\n");
            let err = verify_assets(&files, &md, &attr).unwrap_err();
            assert!(err.contains("committed font file"), "expected font ban error for {font}, got: {err}");
        }
    }

    #[test]
    fn enforces_attribution_md_for_third_party_and_cc_by_assets_while_allowing_lumen_assets() {
        let files = vec![String::from("assets/mole.png"), String::from("assets/third.svg")];
        let assets_ok = "\
| Asset | Author | Source | Licence | Notes |
|---|---|---|---|---|
| `assets/mole.png` | LumenCraft contributors | Original artwork | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | ok |
| `assets/third.svg` | Upstream Icon Set | https://example.org/icon | CC-BY-4.0 | third-party |
";
        let err = verify_assets(&files, assets_ok, "").unwrap_err();
        assert!(err.contains("ATTRIBUTION.md") && err.contains("assets/third.svg"), "{err}");

        let attr_ok = "\
| Asset | Author | Source | SPDX license | Notes |
|---|---|---|---|---|
| `assets/third.svg` | Upstream Icon Set | https://example.org/icon | CC-BY-4.0 | attributed |
";
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
            "assets/app-icon/lumencraft-win95-2508.png",
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
| `assets/ui/archetypes/xy.svg` | LumenCraft contributors | Original SVG schematic drawn for LumenCraft | MIT OR Apache-2.0 | Archetype icon |
| `assets/diagrams/anova-schematic.svg` | LumenCraft contributors | Original diagram created for LumenCraft | MIT OR Apache-2.0 | Schematic |
| `assets/thumbnails/bar-thumb.png` | LumenCraft contributors | Rendered from `assets/ui/archetypes/xy.svg` | MIT OR Apache-2.0 | Thumbnail |
| `docs/images/welcome.png` | LumenCraft contributors | Deterministic workspace screenshot generated by cargo xtask screenshots | MIT OR Apache-2.0 | Baseline |
";
        assert_eq!(verify_assets(&good_files, good_md, "").unwrap(), 4);

        // Reject third-party icon / external URL even if listed in ATTRIBUTION.md.
        let third_party_files = vec![String::from("assets/ui/icons/save.svg")];
        let third_party_md = "| `assets/ui/icons/save.svg` | Upstream Icon Set | https://example.org/save.svg | CC-BY-4.0 | imported |\n";
        let third_party_attr = "| `assets/ui/icons/save.svg` | Upstream Icon Set | https://example.org/save.svg | CC-BY-4.0 | imported |\n";
        let err_third = verify_assets(&third_party_files, third_party_md, third_party_attr).unwrap_err();
        assert!(err_third.contains("LumenCraft authorship"), "{err_third}");

        // Reject reference frame / derived vendor wording.
        let ref_files = vec![String::from("docs/images/frame.png")];
        let ref_md = "| `docs/images/frame.png` | LumenCraft contributors | Derived from reference frame 12 | MIT OR Apache-2.0 | screenshot |\n";
        let err_ref = verify_assets(&ref_files, ref_md, "").unwrap_err();
        assert!(err_ref.contains("non-original provenance phrase"), "{err_ref}");

        // Reject non-original ambiguous source on a UI thumbnail.
        let vague_files = vec![String::from("assets/thumbnails/card.png")];
        let vague_md = "| `assets/thumbnails/card.png` | LumenCraft contributors | Exported raster image | MIT OR Apache-2.0 | card |\n";
        let err_vague = verify_assets(&vague_files, vague_md, "").unwrap_err();
        assert!(err_vague.contains("original LumenCraft provenance"), "{err_vague}");

        // Reject dangling and circular provenance chains.
        let bad_chain_files = vec![String::from("assets/ui/a.png"), String::from("assets/ui/b.png")];
        let cycle_md = "\
| `assets/ui/a.png` | LumenCraft contributors | Rendered from `assets/ui/b.png` | MIT OR Apache-2.0 | a |
| `assets/ui/b.png` | LumenCraft contributors | Rendered from `assets/ui/a.png` | MIT OR Apache-2.0 | b |
";
        let err_cycle = verify_assets(&bad_chain_files, cycle_md, "").unwrap_err();
        assert!(err_cycle.contains("circular asset provenance chain"), "{err_cycle}");

        let dangling_files = vec![String::from("assets/ui/orphan.png")];
        let dangling_md = "| `assets/ui/orphan.png` | LumenCraft contributors | Rendered from `assets/ui/missing.svg` | MIT OR Apache-2.0 | x |\n";
        let err_dangling = verify_assets(&dangling_files, dangling_md, "").unwrap_err();
        assert!(err_dangling.contains("missing from ASSETS.md"), "{err_dangling}");
    }

    #[test]
    fn repo_assets_md_preserves_seven_pane_art_and_passes_provenance() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let assets_md = std::fs::read_to_string(root.join("ASSETS.md")).unwrap();
        let attr_md = std::fs::read_to_string(root.join("ATTRIBUTION.md")).unwrap();
        let files = vec![
            String::from("assets/app-icon/source/approved-seven-pane.png"),
            String::from("assets/app-icon/lumencraft-win95-2508.png"),
            String::from("assets/app-icon/lumencraft.svg"),
            String::from("docs/images/workspace-split-fit.png"),
            String::from("docs/images/workspace-layout-dark.png"),
        ];
        assert_eq!(verify_assets(&files, &assets_md, &attr_md).unwrap(), 5);
    }
}
