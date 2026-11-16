//! `cargo xtask web-fonts`: stage font faces listed in `crates/text/web-fonts.txt` from
//! `$CURVEBOOK_FONTS_DIR/fonts/` into the Trunk staging directory (`$TRUNK_STAGING_DIR/fonts` or
//! `dist/web/fonts`). No-op when `CURVEBOOK_FONTS_DIR` is unset.

use std::path::{Path, PathBuf};

const MANIFEST_REL: &str = "crates/text/web-fonts.txt";

pub fn run(root: &Path) -> Result<(), String> {
    let Some(fonts_root) = std::env::var_os("CURVEBOOK_FONTS_DIR").filter(|v| !v.is_empty()) else {
        println!("web-fonts: CURVEBOOK_FONTS_DIR not set; skipping optional font staging");
        return Ok(());
    };
    let manifest_path = root.join(MANIFEST_REL);
    let manifest = std::fs::read_to_string(&manifest_path).map_err(|e| format!("{MANIFEST_REL}: {e}"))?;
    let faces = parse_manifest(&manifest);

    let staging = std::env::var_os("TRUNK_STAGING_DIR").map_or_else(|| root.join("dist/web"), PathBuf::from).join("fonts");
    std::fs::create_dir_all(&staging).map_err(|e| format!("mkdir {}: {e}", staging.display()))?;

    let src_base = PathBuf::from(fonts_root).join("fonts");
    let mut copied = 0usize;
    for rel in &faces {
        let src = src_base.join(rel);
        let dst = staging.join(rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        std::fs::copy(&src, &dst).map_err(|e| format!("copy {} -> {}: {e}", src.display(), dst.display()))?;
        copied += 1;
    }
    println!("web-fonts: staged {copied} font face(s) into {}", staging.display());
    Ok(())
}

#[must_use]
pub fn parse_manifest(text: &str) -> Vec<String> {
    text.lines().map(|l| l.split('#').next().unwrap_or_default().trim()).filter(|l| !l.is_empty()).map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use super::parse_manifest;

    #[test]
    fn ignores_comments_and_blank_lines() {
        let m = "# comment\n\nInter/Inter-Regular.ttf # body\nJetBrainsMono/JetBrainsMono-Regular.ttf\n";
        assert_eq!(parse_manifest(m), vec![String::from("Inter/Inter-Regular.ttf"), String::from("JetBrainsMono/JetBrainsMono-Regular.ttf"),]);
    }
}
