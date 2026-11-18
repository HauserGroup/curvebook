//! `cargo xtask bundle`: build release binaries and assemble `dist/CurveBook.app` on macOS.

use std::path::{Path, PathBuf};
use std::process::Command;

const PLIST_IN: &str = "packaging/macos/Info.plist.in";
const ICON_ICNS: &str = "assets/app-icon/curvebook.icns";
const PLUTIL_BIN: &str = "/usr/bin/plutil";
const CODESIGN_BIN: &str = "/usr/bin/codesign";

pub fn run(root: &Path) -> Result<(), String> {
    let cargo_toml = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| format!("Cargo.toml: {e}"))?;
    let version = crate::version::read_workspace_version(&cargo_toml)?;
    let sha = git_short_sha(root);

    build_release_binaries(root, &sha)?;

    let dist_dir = root.join("dist");
    let app_dir = dist_dir.join("CurveBook.app");
    if app_dir.exists() {
        std::fs::remove_dir_all(&app_dir).map_err(|e| format!("clean {}: {e}", app_dir.display()))?;
    }

    if let Err(err) = assemble_validate_and_sign(root, &app_dir, &version, &sha) {
        let _ = std::fs::remove_dir_all(&app_dir);
        let _ = std::fs::remove_dir(&dist_dir);
        return Err(err);
    }

    println!("bundle: assembled, validated, and signed {} (v{version}, {sha})", app_dir.display());
    Ok(())
}

fn build_release_binaries(root: &Path, sha: &str) -> Result<(), String> {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root).env("CURVEBOOK_BUILD_SHA", sha).args(["build", "--release"]);
    if std::env::var("CARGO_NET_OFFLINE").is_ok_and(|val| val == "true" || val == "1") {
        cmd.arg("--offline");
    }
    cmd.args(["-p", "curvebook", "-p", "curvebook-cli"]);
    let status = cmd.status().map_err(|e| format!("cargo build --release: {e}"))?;
    if !status.success() {
        return Err(String::from("release build failed"));
    }
    Ok(())
}

fn assemble_validate_and_sign(root: &Path, app_dir: &Path, version: &str, sha: &str) -> Result<(), String> {
    let contents = app_dir.join("Contents");
    let macos_dir = contents.join("MacOS");
    let res_dir = contents.join("Resources");
    std::fs::create_dir_all(&macos_dir).map_err(|e| format!("mkdir {}: {e}", macos_dir.display()))?;
    std::fs::create_dir_all(&res_dir).map_err(|e| format!("mkdir {}: {e}", res_dir.display()))?;

    let plist_tpl = std::fs::read_to_string(root.join(PLIST_IN)).map_err(|e| format!("{PLIST_IN}: {e}"))?;
    let plist = render_info_plist(&plist_tpl, version, sha);
    let plist_path = contents.join("Info.plist");
    std::fs::write(&plist_path, plist).map_err(|e| format!("write Info.plist: {e}"))?;
    validate_info_plist(&plist_path)?;

    let release_dir = resolve_target_dir(root).join("release");
    for bin in ["curvebook", "curvebook-cli"] {
        let src = release_dir.join(bin);
        let dst = macos_dir.join(bin);
        std::fs::copy(&src, &dst).map_err(|e| format!("copy {}: {e}", src.display()))?;
        set_executable(&dst)?;
    }

    let icns = root.join(ICON_ICNS);
    std::fs::copy(&icns, res_dir.join("curvebook.icns")).map_err(|e| format!("copy {ICON_ICNS}: {e}"))?;
    for doc in ["README.md", "LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "ASSETS.md"] {
        let src = root.join(doc);
        if src.is_file() {
            std::fs::copy(&src, res_dir.join(doc)).map_err(|e| format!("copy {doc}: {e}"))?;
        }
    }

    adhoc_codesign_and_verify(app_dir)?;
    Ok(())
}

fn resolve_target_dir(root: &Path) -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(raw) => {
            let path = PathBuf::from(raw);
            if path.is_absolute() { path } else { root.join(path) }
        }
        None => root.join("target"),
    }
}

/// Validate a generated `Info.plist` file (unsubstituted placeholders + `/usr/bin/plutil -lint` on macOS).
pub fn validate_info_plist(plist_path: &Path) -> Result<(), String> {
    let content = std::fs::read_to_string(plist_path).map_err(|e| format!("read {}: {e}", plist_path.display()))?;
    for token in ["@VERSION@", "@SHORT_VERSION@", "@BUILD_SHA@"] {
        if content.contains(token) {
            return Err(format!("Info.plist validation failed: unsubstituted placeholder `{token}` in {}", plist_path.display()));
        }
    }
    if cfg!(target_os = "macos") {
        run_macos_tool(PLUTIL_BIN, &["-lint"], plist_path, "Info.plist plutil -lint")?;
    }
    Ok(())
}

/// Ad-hoc codesign a macOS `.app` bundle and verify the resulting signature strictly.
pub fn adhoc_codesign_and_verify(app_dir: &Path) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    run_macos_tool(CODESIGN_BIN, &["--force", "--deep", "--sign", "-"], app_dir, "ad-hoc codesign")?;
    run_macos_tool(CODESIGN_BIN, &["--verify", "--strict", "--deep"], app_dir, "codesign --verify")?;
    Ok(())
}

fn run_macos_tool(tool: &str, args: &[&str], target: &Path, label: &str) -> Result<(), String> {
    if !Path::new(tool).exists() {
        return Err(format!("{label} failed: missing macOS system tool `{tool}`"));
    }
    let output =
        Command::new(tool).args(args).arg(target).output().map_err(|e| format!("{label} failed to launch `{tool}` for {}: {e}", target.display()))?;
    if !output.status.success() {
        let code = output.status.code().map_or_else(|| String::from("signal"), |c| c.to_string());
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(format!("{label} failed for `{}` (exit {code}): stdout=`{stdout}` stderr=`{stderr}`", target.display()));
    }
    Ok(())
}

#[must_use]
pub fn render_info_plist(template: &str, version: &str, build_sha: &str) -> String {
    let short = version.split('-').next().unwrap_or(version);
    template.replace("@VERSION@", version).replace("@SHORT_VERSION@", short).replace("@BUILD_SHA@", build_sha)
}

fn git_short_sha(root: &Path) -> String {
    Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "--short=9", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| String::from("dev"))
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).map_err(|e| format!("stat {}: {e}", path.display()))?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).map_err(|e| format!("chmod {}: {e}", path.display()))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{adhoc_codesign_and_verify, render_info_plist, validate_info_plist};
    use std::path::{Path, PathBuf};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("curvebook-xtask-bundle-{label}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn substitutes_version_and_sha_placeholders() {
        let rendered = render_info_plist("<string>@VERSION@/@SHORT_VERSION@ (@BUILD_SHA@)</string>", "0.1.0-rc.1", "abc1234");
        assert_eq!(rendered, "<string>0.1.0-rc.1/0.1.0 (abc1234)</string>");
    }

    #[test]
    fn validate_info_plist_accepts_rendered_template_and_rejects_broken_or_unrendered() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let tpl = std::fs::read_to_string(root.join("packaging/macos/Info.plist.in")).unwrap();
        let dir = unique_temp_dir("plist");

        let valid_path = dir.join("ValidInfo.plist");
        std::fs::write(&valid_path, render_info_plist(&tpl, "0.1.0", "deadbeef1")).unwrap();
        validate_info_plist(&valid_path).expect("rendered Info.plist.in must pass plutil validation");

        let unrendered_path = dir.join("UnrenderedInfo.plist");
        std::fs::write(&unrendered_path, &tpl).unwrap();
        let err = validate_info_plist(&unrendered_path).expect_err("unsubstituted placeholders must fail");
        assert!(err.contains("@VERSION@"));

        if cfg!(target_os = "macos") {
            let malformed_path = dir.join("MalformedInfo.plist");
            std::fs::write(&malformed_path, "<?xml version=\"1.0\"?><plist><dict><key>Broken</plist>").unwrap();
            let err = validate_info_plist(&malformed_path).expect_err("malformed XML plist must fail plutil -lint");
            assert!(err.contains("plutil -lint"), "unexpected error: {err}");

            let missing_app = dir.join("Missing.app");
            let sign_err = adhoc_codesign_and_verify(&missing_app).expect_err("missing app must fail codesign");
            assert!(sign_err.contains("codesign"), "unexpected sign error: {sign_err}");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn packaging_metadata_is_consistent_and_free_of_sibling_leftovers() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let plist = std::fs::read_to_string(root.join("packaging/macos/Info.plist.in")).unwrap();
        assert!(plist.contains("org.curvebook.CurveBook"));
        assert!(plist.contains("curvebook"));
        assert!(plist.contains("cbtemplate"));

        let desktop = std::fs::read_to_string(root.join("packaging/linux/org.curvebook.CurveBook.desktop")).unwrap();
        assert!(desktop.contains("Categories=Science;Math;"));
        assert!(desktop.contains("application/vnd.curvebook.project"));

        let mime = std::fs::read_to_string(root.join("packaging/linux/org.curvebook.CurveBook.mime.xml")).unwrap();
        assert!(mime.contains("*.curvebook"));
        assert!(mime.contains("*.cbtemplate"));

        for rel in [
            "packaging/macos/Info.plist.in",
            "packaging/linux/org.curvebook.CurveBook.desktop",
            "packaging/linux/org.curvebook.CurveBook.metainfo.xml.in",
            "packaging/linux/org.curvebook.CurveBook.mime.xml",
            "packaging/windows/curvebook.wxs",
            "packaging/windows/install-policy.wxi",
            "packaging/windows/check-install-policy.ps1",
        ] {
            let content = std::fs::read_to_string(root.join(rel)).unwrap().to_ascii_lowercase();
            for leftover in ["artcraft", "getartcraft", "storytold", "storyteller", "vectorcraft", "drawcraft", "pcraft", "pixelcraft"] {
                assert!(!content.contains(leftover), "{rel} contains leftover `{leftover}`");
            }
        }
    }
}
