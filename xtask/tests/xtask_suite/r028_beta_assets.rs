//! Regression gate for `R0.28` (Beta release asset streamlining & Windows `x86` drop),
//! `R0.26` (`release-scan` CI wiring), `R8.10` (fail-closed macOS signing/notarization),
//! `R8.11` bullet 3 (Windows `schedule` secret guard), `R11.1` bullet 3 (CI web bundle gate),
//! and `R14.2` Part A (release CycloneDX SBOM generation, scanning, and attestation).

use std::fs;
#[cfg(unix)]
use std::process::Command;

use super::common::{TempDirGuard, run_release_scan, workspace_root};

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
        format!("curvebook-{ver}.cdx.json"),
        String::from("THIRD-PARTY-LICENSES.html"),
    ];

    // 1. Missing any expected Beta file (including THIRD-PARTY-LICENSES.html or the CycloneDX SBOM) fails closed naming the missing asset.
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
    {
        let fx = TempDirGuard::new("missing-sbom")?;
        let dir = fx.path.join("release");
        fs::create_dir_all(&dir).unwrap();
        let sbom_name = format!("curvebook-{ver}.cdx.json");
        for name in expected.iter().filter(|n| **n != sbom_name) {
            fs::write(dir.join(name), b"clean stub\n").unwrap();
        }
        fs::write(dir.join("SHA256SUMS.txt"), format!("{}  curvebook-{ver}-macos-universal.dmg\n", "a".repeat(64))).unwrap();

        let out = run_release_scan(&["--expect-beta-set", ver, dir.to_str().unwrap()])?;
        assert!(!out.status.success());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("missing expected file(s)") && stderr.contains(&sbom_name), "{stderr}");
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
        "packaging/macos/INSTALLATION.md",
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

    assert!(release_yml.contains("needs: [linux, macos, windows, freebsd]"));
    assert!(!release_yml.contains("./packaging/web/package.sh"));
    for report_name in [
        "name: scan-report-linux-${{ matrix.arch }}",
        "name: scan-report-macos-universal",
        "name: scan-report-windows-${{ matrix.arch }}",
        "name: scan-report-freebsd-x86_64",
    ] {
        assert!(release_yml.contains(report_name), "release.yml missing `{report_name}` artifact upload");
    }
    assert!(release_yml.contains("--expect-beta-set \"$ver\""));
    assert!(release_yml.contains("--cross-os-reports dist/scan-reports"));
    assert!(release_yml.contains("pattern: scan-report-*"));

    // R14.2 Part A: CI-only CycloneDX SBOM generation, artifact wiring, release-scan, and SBOM attestation.
    assert!(release_yml.contains("cargo install --locked cargo-cyclonedx --version 0.5.10"));
    assert!(release_yml.contains("./packaging/sbom.sh sbom"));
    assert!(release_yml.contains("release-scan --no-checksums sbom"));
    assert!(release_yml.contains("name: cyclonedx-sbom"));
    assert!(release_yml.contains("uses: actions/attest-sbom@c604332985a26aa8cf1bdc465b92731239ec6b9e"));
    assert!(release_yml.contains("--predicate-type https://cyclonedx.org/bom"));

    let root_cargo = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let cargo_lock = fs::read_to_string(root.join("Cargo.lock")).unwrap();
    assert!(!root_cargo.contains("cargo-cyclonedx"), "cargo-cyclonedx must remain CI-only (never in Cargo.toml)");
    assert!(!cargo_lock.contains("name = \"cargo-cyclonedx\""), "cargo-cyclonedx must remain CI-only (never in Cargo.lock)");
}

#[cfg(unix)]
#[test]
fn r14_2_sbom_script_merges_desktop_and_cli_normalizes_paths_and_passes_release_scan() -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let root = workspace_root()?;
    let sbom_sh = root.join("packaging/sbom.sh");
    assert!(sbom_sh.is_file(), "missing packaging/sbom.sh");

    let fx = TempDirGuard::new("r14-2-sbom-script")?;
    let fake_repo = fx.path.join("repo");
    let bin_dir = fx.path.join("mock-bin");
    let out_dir = fx.path.join("sbom-out");
    fs::create_dir_all(fake_repo.join("packaging")).map_err(|e| e.to_string())?;
    fs::create_dir_all(fake_repo.join("apps/curvebook")).map_err(|e| e.to_string())?;
    fs::create_dir_all(fake_repo.join("apps/curvebook-cli")).map_err(|e| e.to_string())?;
    fs::create_dir_all(fake_repo.join("crates/math")).map_err(|e| e.to_string())?;
    fs::create_dir_all(&bin_dir).map_err(|e| e.to_string())?;
    fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;

    let staged_sbom_sh = fake_repo.join("packaging/sbom.sh");
    fs::copy(&sbom_sh, &staged_sbom_sh).map_err(|e| e.to_string())?;
    fs::copy(root.join("packaging/env.sh"), fake_repo.join("packaging/env.sh")).map_err(|e| e.to_string())?;
    fs::write(fake_repo.join("Cargo.toml"), "[workspace]\n[workspace.package]\nversion = \"0.1.0-beta.1\"\n").map_err(|e| e.to_string())?;

    let ws_root_canon = fs::canonicalize(&fake_repo).map_err(|e| e.to_string())?;
    let ws_posix = ws_root_canon.to_string_lossy();
    let mock_cargo = bin_dir.join("cargo");
    let mock_body = format!(
        "#!/usr/bin/env bash\n\
set -euo pipefail\n\
manifest=\"\"\n\
override=\"\"\n\
while [[ $# -gt 0 ]]; do\n\
  case \"$1\" in\n\
    --manifest-path) manifest=\"$2\"; shift 2 ;;\n\
    --override-filename) override=\"$2\"; shift 2 ;;\n\
    *) shift ;;\n\
  esac\n\
done\n\
ws=\"$(dirname \"$manifest\")\"\n\
cat > \"$ws/apps/curvebook/$override.json\" <<'JSON'\n\
{{\n\
  \"bomFormat\": \"CycloneDX\",\n\
  \"specVersion\": \"1.5\",\n\
  \"version\": 1,\n\
  \"metadata\": {{\n\
    \"timestamp\": \"2026-11-25T00:00:00Z\",\n\
    \"component\": {{\n\
      \"type\": \"application\",\n\
      \"bom-ref\": \"path+file://{ws_posix}/apps/curvebook#0.1.0-beta.1\",\n\
      \"name\": \"curvebook\",\n\
      \"version\": \"0.1.0-beta.1\",\n\
      \"purl\": \"pkg:cargo/curvebook@0.1.0-beta.1?download_url=file://.\"\n\
    }}\n\
  }},\n\
  \"components\": [\n\
    {{\n\
      \"type\": \"library\",\n\
      \"bom-ref\": \"path+file://{ws_posix}/crates/math#curvebook-math@0.1.0-beta.1\",\n\
      \"name\": \"curvebook-math\",\n\
      \"version\": \"0.1.0-beta.1\",\n\
      \"purl\": \"pkg:cargo/curvebook-math@0.1.0-beta.1?download_url=file://../../crates/math\"\n\
    }},\n\
    {{\n\
      \"type\": \"library\",\n\
      \"bom-ref\": \"registry+https://github.com/rust-lang/crates.io-index#serde@1.0.219\",\n\
      \"name\": \"serde\",\n\
      \"version\": \"1.0.219\",\n\
      \"purl\": \"pkg:cargo/serde@1.0.219\"\n\
    }}\n\
  ],\n\
  \"dependencies\": [\n\
    {{\n\
      \"ref\": \"path+file://{ws_posix}/apps/curvebook#0.1.0-beta.1\",\n\
      \"dependsOn\": [\"path+file://{ws_posix}/crates/math#curvebook-math@0.1.0-beta.1\"]\n\
    }}\n\
  ]\n\
}}\n\
JSON\n\
cat > \"$ws/apps/curvebook-cli/$override.json\" <<'JSON'\n\
{{\n\
  \"bomFormat\": \"CycloneDX\",\n\
  \"specVersion\": \"1.5\",\n\
  \"version\": 1,\n\
  \"metadata\": {{\n\
    \"timestamp\": \"2026-11-25T00:00:00Z\",\n\
    \"component\": {{\n\
      \"type\": \"application\",\n\
      \"bom-ref\": \"path+file://{ws_posix}/apps/curvebook-cli#0.1.0-beta.1\",\n\
      \"name\": \"curvebook-cli\",\n\
      \"version\": \"0.1.0-beta.1\",\n\
      \"purl\": \"pkg:cargo/curvebook-cli@0.1.0-beta.1?download_url=file://.\"\n\
    }}\n\
  }},\n\
  \"components\": [\n\
    {{\n\
      \"type\": \"library\",\n\
      \"bom-ref\": \"path+file://{ws_posix}/crates/mcp#curvebook-mcp@0.1.0-beta.1\",\n\
      \"name\": \"curvebook-mcp\",\n\
      \"version\": \"0.1.0-beta.1\",\n\
      \"purl\": \"pkg:cargo/curvebook-mcp@0.1.0-beta.1?download_url=file://../../crates/mcp\"\n\
    }},\n\
    {{\n\
      \"type\": \"library\",\n\
      \"bom-ref\": \"registry+https://github.com/rust-lang/crates.io-index#serde@1.0.219\",\n\
      \"name\": \"serde\",\n\
      \"version\": \"1.0.219\",\n\
      \"purl\": \"pkg:cargo/serde@1.0.219\"\n\
    }}\n\
  ],\n\
  \"dependencies\": [\n\
    {{\n\
      \"ref\": \"path+file://{ws_posix}/apps/curvebook-cli#0.1.0-beta.1\",\n\
      \"dependsOn\": [\"path+file://{ws_posix}/crates/mcp#curvebook-mcp@0.1.0-beta.1\"]\n\
    }}\n\
  ]\n\
}}\n\
JSON\n\
printf '{{}}\n' > \"$ws/crates/math/$override.json\"\n"
    );
    fs::write(&mock_cargo, mock_body).map_err(|e| e.to_string())?;
    let mut perms = fs::metadata(&mock_cargo).map_err(|e| e.to_string())?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&mock_cargo, perms).map_err(|e| e.to_string())?;

    let path_env = format!("{}:{}", bin_dir.display(), std::env::var("PATH").unwrap_or_default());
    let run = Command::new("bash")
        .arg(&staged_sbom_sh)
        .arg(&out_dir)
        .current_dir(&fake_repo)
        .env("PATH", &path_env)
        .env("CURVEBOOK_VERSION", "0.1.0-beta.1")
        .env("SOURCE_DATE_EPOCH", "1764076800")
        .output()
        .map_err(|e| format!("run packaging/sbom.sh: {e}"))?;
    assert!(
        run.status.success(),
        "packaging/sbom.sh failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );

    // Every temporary per-crate SBOM file across apps/ and crates/ must be cleaned up automatically.
    for dir in ["apps/curvebook", "apps/curvebook-cli", "crates/math"] {
        let leftovers: Vec<_> = fs::read_dir(fake_repo.join(dir)).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert!(leftovers.is_empty(), "leftover scratch files in {dir}: {leftovers:?}");
    }

    let out_file = out_dir.join("curvebook-0.1.0-beta.1.cdx.json");
    let raw = fs::read_to_string(&out_file).map_err(|e| e.to_string())?;
    assert!(!raw.contains(&*ws_posix), "SBOM leaked local workspace path: {raw}");
    assert!(!raw.contains("download_url=file://"), "SBOM leaked relative file:// purl: {raw}");

    let parsed: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    assert_eq!(parsed["bomFormat"], serde_json::json!("CycloneDX"));
    assert_eq!(parsed["specVersion"], serde_json::json!("1.5"));
    assert_eq!(parsed["metadata"]["component"]["bom-ref"], serde_json::json!("workspace:apps/curvebook#0.1.0-beta.1"));
    let comp_names: Vec<&str> = parsed["components"].as_array().unwrap().iter().filter_map(|c| c["name"].as_str()).collect();
    assert_eq!(comp_names, vec!["curvebook-cli", "curvebook-math", "curvebook-mcp", "serde"]);

    // Merged SBOM must pass `cargo xtask release-scan --no-checksums`.
    let scan = run_release_scan(&["--no-checksums", out_dir.to_str().unwrap()])?;
    assert!(
        scan.status.success(),
        "release-scan rejected generated SBOM:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&scan.stdout),
        String::from_utf8_lossy(&scan.stderr)
    );
    Ok(())
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
