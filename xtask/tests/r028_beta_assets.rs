//! Regression gate for `R0.28` (Beta release asset streamlining & Windows `x86` drop),
//! `R0.26` (`release-scan` CI wiring), `R8.10` (fail-closed macOS signing/notarization),
//! `R8.11` bullet 3 (Windows `schedule` secret guard), and `R11.1` bullet 3 (CI web bundle gate).

#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(label: &str) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!("cb-r028-test-{label}-{}-{nanos}-{seq}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).map_err(|e| format!("create_dir_all {}: {e}", path.display()))?;
        Ok(Self { path })
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(Path::to_path_buf).ok_or_else(|| String::from("CARGO_MANIFEST_DIR has no parent"))
}

fn run_release_scan(args: &[&str]) -> Result<std::process::Output, String> {
    let root = workspace_root()?;
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .current_dir(root)
        .arg("release-scan")
        .args(args)
        .output()
        .map_err(|e| format!("run release-scan {args:?}: {e}"))
}

#[test]
fn r028_expect_beta_set_rejects_missing_extra_and_invalid_cli_flags() -> Result<(), String> {
    let ver = "0.1.0-beta.1";
    let expected = [
        format!("curvebook-{ver}-macos-universal.dmg"),
        format!("curvebook-{ver}-windows-x64.msi"),
        format!("curvebook-{ver}-windows-arm64.msi"),
        format!("curvebook-{ver}-linux-x86_64.AppImage"),
        format!("curvebook-{ver}-linux-aarch64.AppImage"),
        format!("curvebook-{ver}-freebsd-x86_64.tar.gz"),
        String::from("THIRD-PARTY-LICENSES.html"),
    ];

    // 1. Missing any expected Beta file fails closed naming the missing asset.
    {
        let fx = TempDirGuard::new("missing")?;
        let dir = fx.path.join("release");
        fs::create_dir_all(&dir).unwrap();
        for name in &expected[..expected.len() - 1] {
            fs::write(dir.join(name), b"clean stub\n").unwrap();
        }
        fs::write(dir.join("SHA256SUMS.txt"), format!("{}  curvebook-{ver}-macos-universal.dmg\n", "a".repeat(64))).unwrap();

        let out = run_release_scan(&["--expect-beta-set", ver, dir.to_str().unwrap()])?;
        assert!(!out.status.success());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("missing expected file(s)") && stderr.contains("THIRD-PARTY-LICENSES.html"), "{stderr}");
    }

    // 2. Every dropped Beta asset (web zip, macOS cli zip, Windows x86/portable, Linux deb/rpm/tar/zsync/flatpak) fails closed as unexpected extra file.
    for dropped_name in [
        format!("curvebook-web-{ver}.zip"),
        format!("curvebook-cli-{ver}-macos-universal.zip"),
        format!("curvebook-{ver}-windows-x86.msi"),
        format!("curvebook-{ver}-windows-x64-portable.zip"),
        format!("curvebook-{ver}-linux-x86_64.deb"),
        format!("curvebook-{ver}-linux-x86_64.rpm"),
        format!("curvebook-{ver}-linux-x86_64.tar.gz"),
        format!("curvebook-{ver}-linux-x86_64.AppImage.zsync"),
        format!("curvebook-{ver}-linux-x86_64.flatpak"),
    ] {
        let fx = TempDirGuard::new("extra")?;
        let dir = fx.path.join("release");
        fs::create_dir_all(&dir).unwrap();
        for name in &expected {
            fs::write(dir.join(name), b"clean stub\n").unwrap();
        }
        fs::write(dir.join(&dropped_name), b"dropped artifact\n").unwrap();
        fs::write(dir.join("SHA256SUMS.txt"), format!("{}  curvebook-{ver}-macos-universal.dmg\n", "a".repeat(64))).unwrap();

        let out = run_release_scan(&["--expect-beta-set", ver, dir.to_str().unwrap()])?;
        assert!(!out.status.success(), "{dropped_name} must fail closed");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("unexpected extra file(s)") && stderr.contains(&dropped_name), "{stderr}");
    }

    // 3. Combining `--expect-beta-set` with `--no-checksums` or invalid versions fails closed.
    {
        let fx = TempDirGuard::new("bad-cli")?;
        let dir = fx.path.join("release");
        fs::create_dir_all(&dir).unwrap();
        let no_cs = run_release_scan(&["--expect-beta-set", ver, "--no-checksums", dir.to_str().unwrap()])?;
        assert!(!no_cs.status.success());

        let bad_ver = run_release_scan(&["--expect-beta-set", "../bad", dir.to_str().unwrap()])?;
        assert!(!bad_ver.status.success());
    }

    Ok(())
}

#[test]
fn r028_packaging_file_presence_and_scripts_enforced() {
    let root = workspace_root().unwrap();

    for removed in [
        "packaging/windows/portable.txt",
        "packaging/linux/nfpm.yaml",
        "packaging/linux/postinst.sh",
        "packaging/linux/flatpak-bundle.sh",
        "packaging/linux/flatpak-oci.sh",
        "packaging/linux/flatpak/org.curvebook.CurveBook.yml",
        "packaging/linux/flatpak/org.curvebook.CurveBook.bundle.yml",
    ] {
        assert!(!root.join(removed).exists(), "{removed} must be deleted under R0.28");
    }

    for retained in [
        "packaging/macos/INSTALLATION.md",
        "packaging/linux/org.curvebook.CurveBook.desktop",
        "packaging/linux/org.curvebook.CurveBook.metainfo.xml.in",
        "packaging/linux/org.curvebook.CurveBook.mime.xml",
        "packaging/linux/apprun-test.sh",
    ] {
        assert!(root.join(retained).is_file(), "missing retained packaging file {retained}");
    }

    let win_pkg = fs::read_to_string(root.join("packaging/windows/package.ps1")).unwrap();
    assert!(!win_pkg.contains("i686-pc-windows-msvc") && !win_pkg.contains("portable"));
    let test_policy = fs::read_to_string(root.join("packaging/windows/test-install-policy.ps1")).unwrap();
    assert!(!test_policy.contains("'x86'"));
    let policy_wxi = fs::read_to_string(root.join("packaging/windows/install-policy.wxi")).unwrap();
    assert!(!policy_wxi.contains("$(sys.BUILDARCH) = x86") && policy_wxi.contains("[%ProgramW6432]"));
    let check_policy = fs::read_to_string(root.join("packaging/windows/check-install-policy.ps1")).unwrap();
    assert!(!check_policy.contains("VersionNT64"));
    let policy_json = fs::read_to_string(root.join("packaging/windows/install-policy.json")).unwrap();
    for guid in ["570875D8-A8A5-4335-8646-C8CE385B27BD", "30074268-38AD-4435-82B7-E2E8948273F9", "5BFAC2A4-683C-40C9-B561-298D76308811"] {
        assert!(policy_json.contains(guid), "install-policy.json must preserve GUID {guid}");
    }

    let linux_pkg = fs::read_to_string(root.join("packaging/linux/package.sh")).unwrap();
    assert!(!linux_pkg.contains("zsync") && !linux_pkg.contains("nfpm") && !linux_pkg.contains("--formats"));
    assert!(linux_pkg.contains("BASENAME=\"curvebook-$VERSION-linux-$ARCH\""));

    let macos_pkg = fs::read_to_string(root.join("packaging/macos/package.sh")).unwrap();
    assert!(!macos_pkg.contains("CLI_ZIP") && !macos_pkg.contains("curvebook-cli-$VERSION"));
    assert!(macos_pkg.contains("--require-signing"));
    assert!(macos_pkg.contains("DMG=\"$DIST/curvebook-$VERSION-macos-$ARCH.dmg\""));
}

#[test]
fn r028_r026_r810_r811_release_workflow_gates_enforced() {
    let root = workspace_root().unwrap();
    let release_yml = fs::read_to_string(root.join(".github/workflows/release.yml")).unwrap();

    for forbidden in ["i686-pc-windows-msvc", "nfpm", "flatpak", "zsync", "portable", "name: release-web"] {
        assert!(!release_yml.contains(forbidden), "release.yml still mentions `{forbidden}`");
    }
    assert!(release_yml.contains("arch: [x64, arm64]"));
    assert!(release_yml.contains("targets: x86_64-pc-windows-msvc, aarch64-pc-windows-msvc"));

    for secret in [
        "WINDOWS_CERTIFICATE",
        "WINDOWS_CERTIFICATE_PASSWORD",
        "AZURE_TENANT_ID",
        "AZURE_CLIENT_ID",
        "AZURE_CLIENT_SECRET",
        "AZURE_SIGNING_ENDPOINT",
        "AZURE_SIGNING_ACCOUNT",
        "AZURE_CERT_PROFILE",
    ] {
        let needle = format!("{secret}: ${{{{ github.event_name != 'schedule' && !inputs.alpha && secrets.{secret} || '' }}}}");
        assert!(release_yml.contains(&needle), "missing schedule guard for `{secret}` in release.yml");
    }

    assert!(!release_yml.contains("No APPLE_CERTIFICATE configured; using ad-hoc signing."));
    for apple_secret in ["APPLE_CERTIFICATE", "APPLE_CERTIFICATE_PASSWORD", "KEYCHAIN_PASSWORD", "APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID"] {
        assert!(release_yml.contains(apple_secret), "release.yml missing {apple_secret}");
    }
    assert!(release_yml.contains("./packaging/macos/package.sh --arch universal --require-signing"));
    assert!(release_yml.contains("./packaging/macos/verify.sh --arch universal --require-notarized"));

    assert!(release_yml.contains("needs: [linux, macos, windows, freebsd, web]"));
    for report_name in [
        "name: scan-report-linux-${{ matrix.arch }}",
        "name: scan-report-macos-universal",
        "name: scan-report-windows-${{ matrix.arch }}",
        "name: scan-report-web",
        "name: scan-report-freebsd-x86_64",
    ] {
        assert!(release_yml.contains(report_name), "release.yml missing `{report_name}` artifact upload");
    }
    assert!(release_yml.contains("--expect-beta-set \"$ver\""));
    assert!(release_yml.contains("--cross-os-reports dist/scan-reports"));
    assert!(release_yml.contains("pattern: scan-report-*"));
}

#[test]
fn r111_ci_workflow_enforces_web_bundle_budget() {
    let root = workspace_root().unwrap();
    let ci_yml = fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    assert!(ci_yml.contains("trunk --version 0.22.1"));
    assert!(ci_yml.contains("./packaging/web/package.sh"));
    assert!(ci_yml.contains("CURVEBOOK_REQUIRE_WEB_BUNDLE"));
}

#[cfg(unix)]
#[test]
fn r810_macos_package_sh_fails_closed_when_signing_required_without_credentials() {
    let root = workspace_root().unwrap();
    let pkg_sh = root.join("packaging/macos/package.sh");

    let missing_id = Command::new("bash")
        .arg(&pkg_sh)
        .args(["--arch", "universal", "--require-signing", "--skip-build"])
        .env_remove("MACOS_SIGN_IDENTITY")
        .env_remove("APPLE_ID")
        .env_remove("APPLE_PASSWORD")
        .env_remove("APPLE_TEAM_ID")
        .output()
        .unwrap();
    assert!(!missing_id.status.success(), "missing MACOS_SIGN_IDENTITY must fail closed");
    assert!(String::from_utf8_lossy(&missing_id.stderr).contains("MACOS_SIGN_IDENTITY"), "stderr={}", String::from_utf8_lossy(&missing_id.stderr));

    let missing_notary = Command::new("bash")
        .arg(&pkg_sh)
        .args(["--arch", "universal", "--require-signing", "--skip-build"])
        .env("MACOS_SIGN_IDENTITY", "Developer ID Application: Test")
        .env("APPLE_ID", "test@example.com")
        .env_remove("APPLE_PASSWORD")
        .env_remove("APPLE_TEAM_ID")
        .output()
        .unwrap();
    assert!(!missing_notary.status.success(), "incomplete notarization credentials must fail closed");
    assert!(String::from_utf8_lossy(&missing_notary.stderr).contains("APPLE_PASSWORD"), "stderr={}", String::from_utf8_lossy(&missing_notary.stderr));
}
