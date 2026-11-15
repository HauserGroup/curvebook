//! `cargo xtask assets`: every non-code asset must be attributed in `ASSETS.md` (and
//! `ATTRIBUTION.md` for third-party / attribution-bearing works) with an allowlisted permissive
//! licence, and zero font binaries may be tracked in the repository.

use std::collections::BTreeMap;
use std::path::Path;

const FONT_EXT: &[&str] = &["ttf", "otf", "ttc", "woff", "woff2", "eot", "pfb"];

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
        rows.insert(
            path.to_owned(),
            AssetRow { path: path.to_owned(), author: (*author_cell).to_owned(), source: (*source_cell).to_owned(), licence_tokens },
        );
    }
    Ok(rows)
}

fn row_requires_attribution_entry(row: &AssetRow) -> bool {
    let third_party_author = !row.author.to_ascii_lowercase().contains("lumencraft");
    let attr_licence = row.licence_tokens.iter().any(|tok| ATTRIBUTION_REQUIRED_LICENCES.contains(&tok.as_str()));
    third_party_author || attr_licence
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
    use super::{is_asset, is_font_file, missing, verify_assets};

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
}
