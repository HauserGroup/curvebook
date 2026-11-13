//! `cargo xtask assets`: every non-code asset must be attributed in `ASSETS.md`.

use std::path::Path;

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
    "pdf",
    "eps",
    "icc",
    "icm",
    "ttf",
    "otf",
    "woff",
    "woff2",
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

#[must_use]
pub fn is_asset(path: &str) -> bool {
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    ASSET_DIRS.iter().any(|d| path.starts_with(d)) || ASSET_EXT.contains(&ext.as_str())
}

/// Paths that require an `ASSETS.md` row (`path` in backticks) and lack one.
#[must_use]
pub fn missing(files: &[String], assets_md: &str) -> Vec<String> {
    files.iter().filter(|f| is_asset(f) && !assets_md.contains(&format!("`{f}`"))).cloned().collect()
}

pub fn run(root: &Path) -> Result<(), String> {
    let files = crate::repo_files(root)?;
    let md = std::fs::read_to_string(root.join("ASSETS.md")).map_err(|e| format!("ASSETS.md: {e}"))?;
    let miss = missing(&files, &md);
    if miss.is_empty() {
        println!("assets: all {} asset files attributed in ASSETS.md", files.iter().filter(|f| is_asset(f)).count());
        Ok(())
    } else {
        Err(format!("{} asset file(s) lack an ASSETS.md entry (author, source, licence):\n  {}", miss.len(), miss.join("\n  ")))
    }
}

#[cfg(test)]
mod tests {
    use super::{is_asset, missing};

    #[test]
    fn detects_unattributed_assets() {
        let files =
            vec![String::from("assets/app-icon/lumencraft.svg"), String::from("crates/stats/src/lib.rs"), String::from("docs/images/plot.png")];
        let md = "| `assets/app-icon/lumencraft.svg` | LumenCraft contributors | original | MIT OR Apache-2.0 |";
        assert_eq!(missing(&files, md), vec![String::from("docs/images/plot.png")]);
        assert!(!is_asset("crates/stats/src/lib.rs"));
        assert!(is_asset("examples/dose.lumencraft"));
    }
}
