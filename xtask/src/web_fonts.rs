//! `cargo xtask web-fonts`: stage font faces listed in `crates/text/web-fonts.txt` from
//! `$CURVEBOOK_FONTS_DIR/fonts/` into the Trunk staging directory (`$TRUNK_STAGING_DIR/fonts` or
//! `dist/web/fonts`). No-op when `CURVEBOOK_FONTS_DIR` is unset.
//!
//! Also provides `R10.1` / `R10.9` / `R10.22` web release bundle budget and packaging readiness
//! validation (`<= 6_243_591` B compressed `curvebook-web-*.zip`).

use std::io::Read;
use std::path::{Path, PathBuf};

const MANIFEST_REL: &str = "crates/text/web-fonts.txt";
const TRUNK_TOML_REL: &str = "apps/curvebook-web/Trunk.toml";
const WEB_INDEX_REL: &str = "apps/curvebook-web/index.html";
const PACKAGE_SH_REL: &str = "packaging/web/package.sh";

/// `R10.22` web release zip budget (`1.5 * 4,162,394 B` pre-`R0.20` baseline).
pub const MAX_WEB_RELEASE_ZIP_BYTES: u64 = 6_243_591;

pub fn run(root: &Path) -> Result<(), String> {
    let Some(fonts_root) = std::env::var_os("CURVEBOOK_FONTS_DIR").filter(|v| !v.is_empty()) else {
        println!("web-fonts: CURVEBOOK_FONTS_DIR not set; skipping optional font staging");
        return Ok(());
    };
    let manifest_path = root.join(MANIFEST_REL);
    let manifest = std::fs::read_to_string(&manifest_path).map_err(|e| format!("{MANIFEST_REL}: {e}"))?;
    let faces = parse_manifest(&manifest);
    validate_manifest_entries(&faces)?;

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

fn validate_manifest_entries(faces: &[String]) -> Result<(), String> {
    for rel in faces {
        if rel.starts_with('/') || rel.starts_with('\\') || rel.contains("..") {
            return Err(format!("{MANIFEST_REL}: unsafe font manifest entry `{rel}`"));
        }
    }
    Ok(())
}

/// Verify that a built `curvebook-web-*.zip` archive is a valid non-empty ZIP within `MAX_WEB_RELEASE_ZIP_BYTES`.
pub fn verify_web_bundle_zip_bytes(zip_path: &Path) -> Result<u64, String> {
    let meta = std::fs::metadata(zip_path).map_err(|e| format!("stat {}: {e}", zip_path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a regular file", zip_path.display()));
    }
    let size = meta.len();
    if size == 0 {
        return Err(format!("{} is empty (0 B)", zip_path.display()));
    }
    if size > MAX_WEB_RELEASE_ZIP_BYTES {
        return Err(format!("web release bundle {} exceeds R10.22 budget: {size} B > {MAX_WEB_RELEASE_ZIP_BYTES} B", zip_path.display()));
    }
    let mut file = std::fs::File::open(zip_path).map_err(|e| format!("open {}: {e}", zip_path.display()))?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).map_err(|e| format!("read header {}: {e}", zip_path.display()))?;
    if &magic != b"PK\x03\x04" {
        return Err(format!("{} does not start with ZIP local file signature", zip_path.display()));
    }
    Ok(size)
}

/// Parse whether `CURVEBOOK_REQUIRE_WEB_BUNDLE` requires a built `dist/release/curvebook-web-*.zip`.
///
/// Unset, empty, `0`, `false`, `no`, and `off` (case-insensitive) return `false`; any other value
/// (such as `1` or `true`) returns `true` so release/CI bundle checks fail closed.
#[must_use]
pub fn parse_require_web_bundle_env(raw: Option<&std::ffi::OsStr>) -> bool {
    crate::workflows::parse_ci_env_value(raw)
}

/// Execute `cargo xtask web-bundle`.
pub fn run_bundle_check(root: &Path, args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("usage: cargo xtask web-bundle (unexpected args: `{}`)", args.join(" ")));
    }
    check_web_bundle_readiness(root)
}

/// Verify web release packaging readiness and check any built `dist/release/curvebook-web-*.zip` against `MAX_WEB_RELEASE_ZIP_BYTES`.
pub fn check_web_bundle_readiness(root: &Path) -> Result<(), String> {
    check_web_bundle_readiness_with_env(root, std::env::var_os("CURVEBOOK_REQUIRE_WEB_BUNDLE").as_deref())
}

/// Testable core of [`check_web_bundle_readiness`] accepting an explicit `CURVEBOOK_REQUIRE_WEB_BUNDLE` value.
pub fn check_web_bundle_readiness_with_env(root: &Path, require_bundle_env: Option<&std::ffi::OsStr>) -> Result<(), String> {
    let manifest_raw = std::fs::read_to_string(root.join(MANIFEST_REL)).map_err(|e| format!("read {MANIFEST_REL}: {e}"))?;
    let faces = parse_manifest(&manifest_raw);
    validate_manifest_entries(&faces)?;

    let trunk_toml = std::fs::read_to_string(root.join(TRUNK_TOML_REL)).map_err(|e| format!("read {TRUNK_TOML_REL}: {e}"))?;
    for required in ["public_url = \"./\"", "filehash = true", "web-fonts"] {
        if !trunk_toml.contains(required) {
            return Err(format!("{TRUNK_TOML_REL} missing required setting `{required}`"));
        }
    }

    let index_html = std::fs::read_to_string(root.join(WEB_INDEX_REL)).map_err(|e| format!("read {WEB_INDEX_REL}: {e}"))?;
    for required in ["default-src 'self'", "data-wasm-opt"] {
        if !index_html.contains(required) {
            return Err(format!("{WEB_INDEX_REL} missing required attribute `{required}`"));
        }
    }

    let pkg_sh = std::fs::read_to_string(root.join(PACKAGE_SH_REL)).map_err(|e| format!("read {PACKAGE_SH_REL}: {e}"))?;
    if !pkg_sh.contains("--cargo-profile web-release") {
        return Err(format!("{PACKAGE_SH_REL} missing `--cargo-profile web-release`"));
    }

    let cargo_toml = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| format!("read Cargo.toml: {e}"))?;
    for required in ["[profile.web-release]", "lto = \"fat\"", "strip = \"symbols\""] {
        if !cargo_toml.contains(required) {
            return Err(format!("Cargo.toml missing `{required}` for web release size budget"));
        }
    }

    let release_dir = root.join("dist/release");
    let mut checked_zips = 0usize;
    if release_dir.is_dir() {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&release_dir).map_err(|e| format!("read {}: {e}", release_dir.display()))? {
            let path = entry.map_err(|e| format!("read entry in {}: {e}", release_dir.display()))?.path();
            if path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("curvebook-web-") && n.ends_with(".zip")) {
                entries.push(path);
            }
        }
        entries.sort();
        for zip_path in entries {
            let size = verify_web_bundle_zip_bytes(&zip_path)?;
            println!("web-bundle: {} = {size} B (budget <= {MAX_WEB_RELEASE_ZIP_BYTES} B)", zip_path.display());
            checked_zips += 1;
        }
    }

    let require_bundle = parse_require_web_bundle_env(require_bundle_env);
    if checked_zips == 0 {
        if require_bundle {
            return Err(String::from("CURVEBOOK_REQUIRE_WEB_BUNDLE is set, but no dist/release/curvebook-web-*.zip archive exists"));
        }
        println!(
            "web-bundle: CI readiness OK ({} staged manifest entry/ies, budget <= {MAX_WEB_RELEASE_ZIP_BYTES} B; no local zip in dist/release/)",
            faces.len()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_WEB_RELEASE_ZIP_BYTES, check_web_bundle_readiness, check_web_bundle_readiness_with_env, parse_manifest, parse_require_web_bundle_env,
        validate_manifest_entries, verify_web_bundle_zip_bytes,
    };
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};

    fn temp_test_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("cb-xtask-webfonts-{label}-{}-{nanos}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn write_minimal_web_workspace(root: &Path) {
        std::fs::create_dir_all(root.join("crates/text")).unwrap();
        std::fs::create_dir_all(root.join("apps/curvebook-web")).unwrap();
        std::fs::create_dir_all(root.join("packaging/web")).unwrap();
        std::fs::write(root.join("crates/text/web-fonts.txt"), "Inter/Inter-Regular.ttf\n").unwrap();
        std::fs::write(
            root.join("apps/curvebook-web/Trunk.toml"),
            "[build]\npublic_url = \"./\"\nfilehash = true\ncommand_arguments = [\"xtask\", \"web-fonts\"]\n",
        )
        .unwrap();
        std::fs::write(root.join("apps/curvebook-web/index.html"), "<meta content=\"default-src 'self'\"><link data-wasm-opt=\"z\">").unwrap();
        std::fs::write(root.join("packaging/web/package.sh"), "trunk build --release --cargo-profile web-release\n").unwrap();
        std::fs::write(root.join("Cargo.toml"), "[profile.web-release]\ninherits = \"release\"\nlto = \"fat\"\nstrip = \"symbols\"\n").unwrap();
    }

    #[test]
    fn ignores_comments_and_blank_lines() {
        let m = "# comment\n\nInter/Inter-Regular.ttf # body\nJetBrainsMono/JetBrainsMono-Regular.ttf\n";
        let parsed = parse_manifest(m);
        assert_eq!(parsed, vec![String::from("Inter/Inter-Regular.ttf"), String::from("JetBrainsMono/JetBrainsMono-Regular.ttf")]);
        assert!(validate_manifest_entries(&parsed).is_ok());
        assert!(validate_manifest_entries(&[String::from("../escape.ttf")]).is_err());
    }

    #[test]
    fn verifies_web_bundle_zip_budget_bounds() {
        let dir = temp_test_dir("budget");
        let ok_zip = dir.join("curvebook-web-ok.zip");
        let mut ok_bytes = b"PK\x03\x04".to_vec();
        ok_bytes.resize(4096, 0);
        std::fs::write(&ok_zip, &ok_bytes).unwrap();
        assert_eq!(verify_web_bundle_zip_bytes(&ok_zip), Ok(4096));

        let bad_magic = dir.join("bad.zip");
        std::fs::write(&bad_magic, b"NOTZIP").unwrap();
        assert!(verify_web_bundle_zip_bytes(&bad_magic).is_err());

        let over_budget = dir.join("over.zip");
        let file = std::fs::File::create(&over_budget).unwrap();
        file.set_len(MAX_WEB_RELEASE_ZIP_BYTES + 1).unwrap();
        assert!(verify_web_bundle_zip_bytes(&over_budget).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn require_web_bundle_env_allows_missing_zip_when_unset_and_fails_closed_when_enabled() {
        assert!(!parse_require_web_bundle_env(None));
        assert!(!parse_require_web_bundle_env(Some(OsStr::new(""))));
        assert!(!parse_require_web_bundle_env(Some(OsStr::new("0"))));
        assert!(!parse_require_web_bundle_env(Some(OsStr::new("false"))));
        assert!(parse_require_web_bundle_env(Some(OsStr::new("1"))));
        assert!(parse_require_web_bundle_env(Some(OsStr::new("true"))));

        let dir = temp_test_dir("require-bundle");
        write_minimal_web_workspace(&dir);

        // Unset / falsy skips missing zip cleanly after checking static readiness.
        assert!(check_web_bundle_readiness_with_env(&dir, None).is_ok());
        assert!(check_web_bundle_readiness_with_env(&dir, Some(OsStr::new("0"))).is_ok());

        // CURVEBOOK_REQUIRE_WEB_BUNDLE=1 fails closed when dist/release/ has no web zip.
        let missing_err = check_web_bundle_readiness_with_env(&dir, Some(OsStr::new("1"))).unwrap_err();
        assert!(missing_err.contains("CURVEBOOK_REQUIRE_WEB_BUNDLE"), "unexpected error: {missing_err}");

        // Once a valid in-budget zip exists in dist/release/, CURVEBOOK_REQUIRE_WEB_BUNDLE=1 passes.
        let rel_dir = dir.join("dist/release");
        std::fs::create_dir_all(&rel_dir).unwrap();
        let zip_path = rel_dir.join("curvebook-web-0.1.0-alpha.2.zip");
        let mut zip_bytes = b"PK\x03\x04".to_vec();
        zip_bytes.resize(2048, 0);
        std::fs::write(&zip_path, &zip_bytes).unwrap();
        assert!(check_web_bundle_readiness_with_env(&dir, Some(OsStr::new("1"))).is_ok());

        // Over-budget zip fails closed in both unset and CURVEBOOK_REQUIRE_WEB_BUNDLE=1 modes.
        let file = std::fs::OpenOptions::new().write(true).open(&zip_path).unwrap();
        file.set_len(MAX_WEB_RELEASE_ZIP_BYTES + 1).unwrap();
        assert!(check_web_bundle_readiness_with_env(&dir, None).is_err());
        assert!(check_web_bundle_readiness_with_env(&dir, Some(OsStr::new("1"))).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn workspace_web_bundle_readiness_passes() {
        let root = crate::workspace_root().unwrap();
        assert!(check_web_bundle_readiness_with_env(&root, None).is_ok());
        assert!(check_web_bundle_readiness(&root).is_ok());
    }
}
