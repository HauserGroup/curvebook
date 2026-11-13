//! `cargo xtask bundle`: build release binaries and assemble `dist/LumenCraft.app` on macOS.

use std::path::Path;
use std::process::Command;

const PLIST_IN: &str = "packaging/macos/Info.plist.in";
const ICON_ICNS: &str = "assets/app-icon/lumencraft.icns";

pub fn run(root: &Path) -> Result<(), String> {
    let cargo_toml = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| format!("Cargo.toml: {e}"))?;
    let version = crate::version::read_workspace_version(&cargo_toml)?;
    let sha = git_short_sha(root);

    let status = Command::new("cargo")
        .current_dir(root)
        .env("LUMENCRAFT_BUILD_SHA", &sha)
        .args(["build", "--release", "-p", "lumencraft", "-p", "lumencraft-cli"])
        .status()
        .map_err(|e| format!("cargo build --release: {e}"))?;
    if !status.success() {
        return Err(String::from("release build failed"));
    }

    let app_dir = root.join("dist/LumenCraft.app");
    let contents = app_dir.join("Contents");
    let macos_dir = contents.join("MacOS");
    let res_dir = contents.join("Resources");
    if app_dir.exists() {
        std::fs::remove_dir_all(&app_dir).map_err(|e| format!("clean {}: {e}", app_dir.display()))?;
    }
    std::fs::create_dir_all(&macos_dir).map_err(|e| format!("mkdir {}: {e}", macos_dir.display()))?;
    std::fs::create_dir_all(&res_dir).map_err(|e| format!("mkdir {}: {e}", res_dir.display()))?;

    let plist_tpl = std::fs::read_to_string(root.join(PLIST_IN)).map_err(|e| format!("{PLIST_IN}: {e}"))?;
    let plist = render_info_plist(&plist_tpl, &version, &sha);
    std::fs::write(contents.join("Info.plist"), plist).map_err(|e| format!("write Info.plist: {e}"))?;

    let target_dir = std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), std::path::PathBuf::from);
    let release_dir = target_dir.join("release");
    for bin in ["lumencraft", "lumencraft-cli"] {
        let src = release_dir.join(bin);
        let dst = macos_dir.join(bin);
        std::fs::copy(&src, &dst).map_err(|e| format!("copy {}: {e}", src.display()))?;
        set_executable(&dst)?;
    }

    let icns = root.join(ICON_ICNS);
    std::fs::copy(&icns, res_dir.join("lumencraft.icns")).map_err(|e| format!("copy {ICON_ICNS}: {e}"))?;

    if cfg!(target_os = "macos") && Path::new("/usr/bin/codesign").exists() {
        let sign = Command::new("/usr/bin/codesign")
            .args(["--force", "--deep", "--sign", "-"])
            .arg(&app_dir)
            .status()
            .map_err(|e| format!("codesign: {e}"))?;
        if !sign.success() {
            return Err(String::from("ad-hoc codesign failed"));
        }
    }

    println!("bundle: assembled {} (v{version}, {sha})", app_dir.display());
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
    use super::render_info_plist;
    use std::path::Path;

    #[test]
    fn substitutes_version_and_sha_placeholders() {
        let rendered = render_info_plist("<string>@VERSION@ (@BUILD_SHA@)</string>", "0.1.0", "abc1234");
        assert_eq!(rendered, "<string>0.1.0 (abc1234)</string>");
    }

    #[test]
    fn packaging_metadata_is_consistent_and_free_of_sibling_leftovers() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let plist = std::fs::read_to_string(root.join("packaging/macos/Info.plist.in")).unwrap();
        assert!(plist.contains("org.lumencraft.LumenCraft"));
        assert!(plist.contains("lumencraft"));
        assert!(plist.contains("lctemplate"));

        let desktop = std::fs::read_to_string(root.join("packaging/linux/org.lumencraft.LumenCraft.desktop")).unwrap();
        assert!(desktop.contains("Categories=Science;Math;"));
        assert!(desktop.contains("application/vnd.lumencraft.project"));

        let mime = std::fs::read_to_string(root.join("packaging/linux/org.lumencraft.LumenCraft.mime.xml")).unwrap();
        assert!(mime.contains("*.lumencraft"));
        assert!(mime.contains("*.lctemplate"));

        for rel in [
            "packaging/macos/Info.plist.in",
            "packaging/linux/org.lumencraft.LumenCraft.desktop",
            "packaging/linux/org.lumencraft.LumenCraft.metainfo.xml.in",
            "packaging/linux/org.lumencraft.LumenCraft.mime.xml",
            "packaging/linux/flatpak/org.lumencraft.LumenCraft.yml",
            "packaging/linux/flatpak/org.lumencraft.LumenCraft.bundle.yml",
            "packaging/windows/lumencraft.wxs",
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
