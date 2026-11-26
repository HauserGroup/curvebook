//! `cargo xtask bundle`: build release binaries and assemble `dist/CurveBook.app` on macOS.

use std::path::{Path, PathBuf};
use std::time::Duration;

use curvebook_platform::process::ProcessCommand;
use curvebook_platform::write_atomic;

const PLIST_IN: &str = "packaging/macos/Info.plist.in";
const ICON_ICNS: &str = "assets/app-icon/curvebook.icns";
const PLUTIL_BIN: &str = "/usr/bin/plutil";
const CODESIGN_BIN: &str = "/usr/bin/codesign";

pub fn run(root: &Path) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err(String::from("macOS .app bundling requires macOS (plutil + codesign)"));
    }
    let version = crate::version::read_workspace_version(&std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| format!("Cargo.toml: {e}"))?)?;
    let sha = git_short_sha(root);
    build_release_binaries(root, &sha)?;
    let (dist_dir, app_dir) = (root.join("dist"), root.join("dist/CurveBook.app"));
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
    let mut args = vec!["build", "--locked", "--release"];
    if std::env::var("CARGO_NET_OFFLINE").is_ok_and(|v| v == "true" || v == "1") {
        args.push("--offline");
    }
    args.extend(["-p", "curvebook", "-p", "curvebook-cli"]);
    let out = crate::dev_tool_command("cargo")
        .current_dir(root)
        .env("CURVEBOOK_BUILD_SHA", sha)
        .args(args)
        .wall_timeout(Duration::from_secs(1800))
        .idle_timeout(Duration::from_secs(300))
        .run()
        .map_err(|e| format!("cargo build --locked --release: {e}"))?;
    if !out.stdout.is_empty() {
        print!("{}", String::from_utf8_lossy(&out.stdout));
    }
    if !out.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
    }
    if out.success { Ok(()) } else { Err(String::from("release build failed")) }
}

fn assemble_validate_and_sign(root: &Path, app_dir: &Path, version: &str, sha: &str) -> Result<(), String> {
    let (contents, macos_dir, res_dir) = (app_dir.join("Contents"), app_dir.join("Contents/MacOS"), app_dir.join("Contents/Resources"));
    for d in [&macos_dir, &res_dir] {
        std::fs::create_dir_all(d).map_err(|e| format!("mkdir {}: {e}", d.display()))?;
    }
    let plist_tpl = std::fs::read_to_string(root.join(PLIST_IN)).map_err(|e| format!("{PLIST_IN}: {e}"))?;
    let plist_path = contents.join("Info.plist");
    write_atomic(&plist_path, render_info_plist(&plist_tpl, version, sha).as_bytes()).map_err(|e| format!("write Info.plist: {e}"))?;
    validate_info_plist(&plist_path)?;
    let release_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map_or_else(|| root.join("target"), |p| if p.is_absolute() { p } else { root.join(p) })
        .join("release");
    for bin in ["curvebook", "curvebook-cli"] {
        let (src, dst) = (release_dir.join(bin), macos_dir.join(bin));
        std::fs::copy(&src, &dst).map_err(|e| format!("copy {}: {e}", src.display()))?;
        set_executable(&dst)?;
    }
    std::fs::copy(root.join(ICON_ICNS), res_dir.join("curvebook.icns")).map_err(|e| format!("copy {ICON_ICNS}: {e}"))?;
    copy_bundle_legal_docs(root, &res_dir)?;
    adhoc_codesign_and_verify(app_dir)
}

fn copy_bundle_legal_docs(root: &Path, res_dir: &Path) -> Result<(), String> {
    for doc in ["LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "ATTRIBUTION.md", "ASSETS.md"] {
        let src = root.join(doc);
        if !crate::scan_util::is_regular_file(&src) {
            return Err(format!("missing required legal/attribution file `{doc}` in {}", root.display()));
        }
        std::fs::copy(&src, res_dir.join(doc)).map_err(|e| format!("copy {doc}: {e}"))?;
    }
    for doc in ["README.md", "LICENSE", "COPYRIGHT"] {
        let src = root.join(doc);
        if crate::scan_util::is_regular_file(&src) {
            std::fs::copy(&src, res_dir.join(doc)).map_err(|e| format!("copy {doc}: {e}"))?;
        }
    }
    if let Some(rel) =
        ["THIRD-PARTY-LICENSES.html", "licenses/THIRD-PARTY-LICENSES.html"].into_iter().find(|r| crate::scan_util::is_regular_file(&root.join(r)))
    {
        std::fs::copy(root.join(rel), res_dir.join("THIRD-PARTY-LICENSES.html")).map_err(|e| format!("copy {rel}: {e}"))?;
        Ok(())
    } else {
        Err(String::from(
            "missing required `THIRD-PARTY-LICENSES.html` (or `licenses/THIRD-PARTY-LICENSES.html`); run `cargo xtask licenses --html` first",
        ))
    }
}

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

pub fn adhoc_codesign_and_verify(app_dir: &Path) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    run_macos_tool(CODESIGN_BIN, &["--force", "--deep", "--sign", "-"], app_dir, "ad-hoc codesign")?;
    run_macos_tool(CODESIGN_BIN, &["--verify", "--strict", "--deep"], app_dir, "codesign --verify")
}

fn run_macos_tool(tool: &str, args: &[&str], target: &Path, label: &str) -> Result<(), String> {
    if !Path::new(tool).exists() {
        return Err(format!("{label} failed: missing macOS system tool `{tool}`"));
    }
    let target_str = target.to_str().ok_or_else(|| format!("{label}: non-UTF-8 path {}", target.display()))?;
    let output = ProcessCommand::new(tool)
        .args(args.iter().copied())
        .arg(target_str)
        .wall_timeout(Duration::from_secs(120))
        .idle_timeout(Duration::from_secs(30))
        .run()
        .map_err(|e| format!("{label} failed to launch `{tool}` for {}: {e}", target.display()))?;
    if !output.success {
        let code = output.status_code.map_or_else(|| String::from("signal"), |c| c.to_string());
        return Err(format!(
            "{label} failed for `{}` (exit {code}): stdout=`{}` stderr=`{}`",
            target.display(),
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

#[must_use]
pub fn render_info_plist(template: &str, version: &str, build_sha: &str) -> String {
    let short = version.split('-').next().unwrap_or(version);
    template.replace("@VERSION@", version).replace("@SHORT_VERSION@", short).replace("@BUILD_SHA@", build_sha)
}

fn git_short_sha(root: &Path) -> String {
    crate::dev_tool_command("git")
        .current_dir(root)
        .args(["rev-parse", "--short=9", "HEAD"])
        .wall_timeout(Duration::from_secs(30))
        .idle_timeout(Duration::from_secs(10))
        .run()
        .ok()
        .filter(|o| o.success)
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
    use std::path::Path;

    use super::{adhoc_codesign_and_verify, copy_bundle_legal_docs, render_info_plist, validate_info_plist};
    use crate::scan_util::test_support::TestDir;

    #[test]
    fn plist_validation_packaging_metadata_and_legal_docs_bundle_guards() {
        assert_eq!(
            render_info_plist("<string>@VERSION@/@SHORT_VERSION@ (@BUILD_SHA@)</string>", "0.1.0-rc.1", "abc1234"),
            "<string>0.1.0-rc.1/0.1.0 (abc1234)</string>"
        );
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let tpl = std::fs::read_to_string(root.join("packaging/macos/Info.plist.in")).unwrap();
        let dir = TestDir::new("bundle-checks");
        dir.write("ValidInfo.plist", &render_info_plist(&tpl, "0.1.0", "deadbeef1"));
        validate_info_plist(&dir.join("ValidInfo.plist")).unwrap();
        dir.write("UnrenderedInfo.plist", &tpl);
        assert!(validate_info_plist(&dir.join("UnrenderedInfo.plist")).unwrap_err().contains("@VERSION@"));
        if cfg!(target_os = "macos") {
            dir.write("MalformedInfo.plist", "<?xml version=\"1.0\"?><plist><dict><key>Broken</plist>");
            assert!(validate_info_plist(&dir.join("MalformedInfo.plist")).unwrap_err().contains("plutil -lint"));
            assert!(adhoc_codesign_and_verify(&dir.join("Missing.app")).unwrap_err().contains("codesign"));
        }

        assert!(tpl.contains("org.curvebook.CurveBook") && tpl.contains("curvebook") && tpl.contains("cbtemplate"));
        let desktop = std::fs::read_to_string(root.join("packaging/linux/org.curvebook.CurveBook.desktop")).unwrap();
        assert!(desktop.contains("Categories=Science;Math;") && desktop.contains("application/vnd.curvebook.project"));
        let mime = std::fs::read_to_string(root.join("packaging/linux/org.curvebook.CurveBook.mime.xml")).unwrap();
        assert!(mime.contains("*.curvebook") && mime.contains("*.cbtemplate"));
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

        let res_dir = dir.join("Resources");
        std::fs::create_dir_all(&res_dir).unwrap();
        for f in ["LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "ATTRIBUTION.md", "ASSETS.md"] {
            dir.write(f, "ok\n");
        }
        assert!(copy_bundle_legal_docs(dir.path(), &res_dir).unwrap_err().contains("THIRD-PARTY-LICENSES.html"));
        dir.write("licenses/THIRD-PARTY-LICENSES.html", "<html>ok</html>\n");
        copy_bundle_legal_docs(dir.path(), &res_dir).unwrap();
        assert!(res_dir.join("THIRD-PARTY-LICENSES.html").is_file() && res_dir.join("LICENSE-MIT").is_file());
        std::fs::remove_file(dir.join("NOTICE")).unwrap();
        assert!(copy_bundle_legal_docs(dir.path(), &res_dir).unwrap_err().contains("NOTICE"));
    }
}
