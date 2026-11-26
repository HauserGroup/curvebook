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
    let run_case = |label: &str, files: &[String], needles: &[&str]| -> Result<(), String> {
        let fx = TempDirGuard::new(label)?;
        let dir = fx.path.join("release");
        fs::create_dir_all(&dir).unwrap();
        for name in files {
            fs::write(dir.join(name), b"clean stub\n").unwrap();
        }
        fs::write(dir.join("SHA256SUMS.txt"), format!("{}  curvebook-{ver}-macos-universal.dmg\n", "a".repeat(64))).unwrap();
        let out = run_release_scan(&["--expect-beta-set", ver, dir.to_str().unwrap()])?;
        assert!(!out.status.success(), "{label} must fail closed");
        let stderr = String::from_utf8_lossy(&out.stderr);
        for &n in needles {
            assert!(stderr.contains(n), "{label}: missing `{n}` in {stderr}");
        }
        Ok(())
    };

    for omit in ["THIRD-PARTY-LICENSES.html", &format!("curvebook-{ver}.cdx.json")] {
        let subset: Vec<String> = expected.iter().filter(|n| n.as_str() != omit).cloned().collect();
        run_case("missing", &subset, &["missing expected file(s)", omit])?;
    }
    for dropped in [
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
        let mut with_extra = expected.to_vec();
        with_extra.push(dropped.clone());
        run_case("extra", &with_extra, &["unexpected extra file(s)", &dropped])?;
    }

    let fx = TempDirGuard::new("bad-cli")?;
    let dir = fx.path.join("release");
    fs::create_dir_all(&dir).unwrap();
    assert!(!run_release_scan(&["--expect-beta-set", ver, "--no-checksums", dir.to_str().unwrap()])?.status.success());
    assert!(!run_release_scan(&["--expect-beta-set", "../bad", dir.to_str().unwrap()])?.status.success());
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

    let read = |rel: &str| fs::read_to_string(root.join(rel)).unwrap();
    let win_pkg = read("packaging/windows/package.ps1");
    assert!(!win_pkg.contains("i686-pc-windows-msvc") && !win_pkg.contains("portable"));
    assert!(!read("packaging/windows/test-install-policy.ps1").contains("'x86'"));
    let policy_wxi = read("packaging/windows/install-policy.wxi");
    assert!(!policy_wxi.contains("$(sys.BUILDARCH) = x86") && policy_wxi.contains("[%ProgramW6432]"));
    assert!(!read("packaging/windows/check-install-policy.ps1").contains("VersionNT64"));
    let policy_json = read("packaging/windows/install-policy.json");
    for guid in ["570875D8-A8A5-4335-8646-C8CE385B27BD", "30074268-38AD-4435-82B7-E2E8948273F9", "5BFAC2A4-683C-40C9-B561-298D76308811"] {
        assert!(policy_json.contains(guid), "install-policy.json must preserve GUID {guid}");
    }
    let linux_pkg = read("packaging/linux/package.sh");
    assert!(
        !linux_pkg.contains("zsync")
            && !linux_pkg.contains("nfpm")
            && !linux_pkg.contains("--formats")
            && linux_pkg.contains("BASENAME=\"curvebook-$VERSION-linux-$ARCH\"")
    );
    let macos_pkg = read("packaging/macos/package.sh");
    assert!(
        !macos_pkg.contains("CLI_ZIP")
            && !macos_pkg.contains("curvebook-cli-$VERSION")
            && macos_pkg.contains("--require-signing")
            && macos_pkg.contains("DMG=\"$DIST/curvebook-$VERSION-macos-$ARCH.dmg\"")
    );
}

#[test]
fn r028_r026_r810_r811_release_workflow_gates_enforced() {
    let root = workspace_root().unwrap();
    let release_yml = fs::read_to_string(root.join(".github/workflows/release.yml")).unwrap();
    for forbidden in [
        "i686-pc-windows-msvc",
        "nfpm",
        "flatpak",
        "zsync",
        "portable",
        "name: release-web",
        "No APPLE_CERTIFICATE configured; using ad-hoc signing.",
        "./packaging/web/package.sh",
    ] {
        assert!(!release_yml.contains(forbidden), "release.yml still mentions `{forbidden}`");
    }
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
    for required in [
        "arch: [x64, arm64]",
        "targets: x86_64-pc-windows-msvc, aarch64-pc-windows-msvc",
        "APPLE_CERTIFICATE",
        "APPLE_CERTIFICATE_PASSWORD",
        "KEYCHAIN_PASSWORD",
        "APPLE_ID",
        "APPLE_PASSWORD",
        "APPLE_TEAM_ID",
        "./packaging/macos/package.sh --arch universal --require-signing",
        "./packaging/macos/verify.sh --arch universal --require-notarized",
        "needs: [linux, macos, windows, freebsd]",
        "name: scan-report-linux-${{ matrix.arch }}",
        "name: scan-report-macos-universal",
        "name: scan-report-windows-${{ matrix.arch }}",
        "name: scan-report-freebsd-x86_64",
        "--expect-beta-set \"$ver\"",
        "--cross-os-reports dist/scan-reports",
        "pattern: scan-report-*",
        "cargo install --locked cargo-cyclonedx --version 0.5.10",
        "./packaging/sbom.sh sbom",
        "release-scan --no-checksums sbom",
        "name: cyclonedx-sbom",
        "uses: actions/attest-sbom@c604332985a26aa8cf1bdc465b92731239ec6b9e",
        "--predicate-type https://cyclonedx.org/bom",
    ] {
        assert!(release_yml.contains(required), "release.yml missing `{required}`");
    }
    assert!(!fs::read_to_string(root.join("Cargo.toml")).unwrap().contains("cargo-cyclonedx"));
    assert!(!fs::read_to_string(root.join("Cargo.lock")).unwrap().contains("name = \"cargo-cyclonedx\""));
}

#[cfg(unix)]
#[test]
fn r14_2_sbom_script_merges_desktop_and_cli_normalizes_paths_and_passes_release_scan() -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let root = workspace_root()?;
    let sbom_sh = root.join("packaging/sbom.sh");
    assert!(sbom_sh.is_file(), "missing packaging/sbom.sh");

    let fx = TempDirGuard::new("r14-2-sbom-script")?;
    let (fake_repo, bin_dir, out_dir) = (fx.path.join("repo"), fx.path.join("mock-bin"), fx.path.join("sbom-out"));
    for d in ["packaging", "apps/curvebook", "apps/curvebook-cli", "crates/math"] {
        fs::create_dir_all(fake_repo.join(d)).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(&bin_dir).map_err(|e| e.to_string())?;
    fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let staged_sbom_sh = fake_repo.join("packaging/sbom.sh");
    fs::copy(&sbom_sh, &staged_sbom_sh).map_err(|e| e.to_string())?;
    fs::copy(root.join("packaging/env.sh"), fake_repo.join("packaging/env.sh")).map_err(|e| e.to_string())?;
    fs::write(fake_repo.join("Cargo.toml"), "[workspace]\n[workspace.package]\nversion = \"0.1.0-beta.1\"\n").map_err(|e| e.to_string())?;

    let ws_posix = fs::canonicalize(&fake_repo).map_err(|e| e.to_string())?.to_string_lossy().into_owned();
    let mock_cargo = bin_dir.join("cargo");
    let desk_json = serde_json::json!({
        "bomFormat": "CycloneDX", "specVersion": "1.5", "version": 1,
        "metadata": {"timestamp": "2026-11-25T00:00:00Z", "component": {"type": "application", "bom-ref": format!("path+file://{ws_posix}/apps/curvebook#0.1.0-beta.1"), "name": "curvebook", "version": "0.1.0-beta.1", "purl": "pkg:cargo/curvebook@0.1.0-beta.1?download_url=file://."}},
        "components": [
            {"type": "library", "bom-ref": format!("path+file://{ws_posix}/crates/math#curvebook-math@0.1.0-beta.1"), "name": "curvebook-math", "version": "0.1.0-beta.1", "purl": "pkg:cargo/curvebook-math@0.1.0-beta.1?download_url=file://../../crates/math"},
            {"type": "library", "bom-ref": "registry+https://github.com/rust-lang/crates.io-index#serde@1.0.219", "name": "serde", "version": "1.0.219", "purl": "pkg:cargo/serde@1.0.219"}
        ],
        "dependencies": [{"ref": format!("path+file://{ws_posix}/apps/curvebook#0.1.0-beta.1"), "dependsOn": [format!("path+file://{ws_posix}/crates/math#curvebook-math@0.1.0-beta.1")]}]
    });
    let cli_json = serde_json::json!({
        "bomFormat": "CycloneDX", "specVersion": "1.5", "version": 1,
        "metadata": {"timestamp": "2026-11-25T00:00:00Z", "component": {"type": "application", "bom-ref": format!("path+file://{ws_posix}/apps/curvebook-cli#0.1.0-beta.1"), "name": "curvebook-cli", "version": "0.1.0-beta.1", "purl": "pkg:cargo/curvebook-cli@0.1.0-beta.1?download_url=file://."}},
        "components": [
            {"type": "library", "bom-ref": format!("path+file://{ws_posix}/crates/mcp#curvebook-mcp@0.1.0-beta.1"), "name": "curvebook-mcp", "version": "0.1.0-beta.1", "purl": "pkg:cargo/curvebook-mcp@0.1.0-beta.1?download_url=file://../../crates/mcp"},
            {"type": "library", "bom-ref": "registry+https://github.com/rust-lang/crates.io-index#serde@1.0.219", "name": "serde", "version": "1.0.219", "purl": "pkg:cargo/serde@1.0.219"}
        ],
        "dependencies": [{"ref": format!("path+file://{ws_posix}/apps/curvebook-cli#0.1.0-beta.1"), "dependsOn": [format!("path+file://{ws_posix}/crates/mcp#curvebook-mcp@0.1.0-beta.1")]}]
    });
    let mock_body = format!(
        "#!/usr/bin/env bash\nset -euo pipefail\nmanifest=\"\"; override=\"\"\nwhile [[ $# -gt 0 ]]; do\n  case \"$1\" in --manifest-path) manifest=\"$2\"; shift 2 ;; --override-filename) override=\"$2\"; shift 2 ;; *) shift ;; esac\ndone\nws=\"$(dirname \"$manifest\")\"\ncat > \"$ws/apps/curvebook/$override.json\" <<'JSON'\n{desk_json}\nJSON\ncat > \"$ws/apps/curvebook-cli/$override.json\" <<'JSON'\n{cli_json}\nJSON\nprintf '{{}}\\n' > \"$ws/crates/math/$override.json\"\n"
    );
    fs::write(&mock_cargo, mock_body).map_err(|e| e.to_string())?;
    let mut perms = fs::metadata(&mock_cargo).map_err(|e| e.to_string())?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&mock_cargo, perms).map_err(|e| e.to_string())?;

    let path_env = format!("{}:{}", bin_dir.display(), std::env::var("PATH").unwrap_or_default());
    let run = Command::new("bash")
        .args([staged_sbom_sh.to_str().unwrap(), out_dir.to_str().unwrap()])
        .current_dir(&fake_repo)
        .env("PATH", &path_env)
        .env("CURVEBOOK_VERSION", "0.1.0-beta.1")
        .env("SOURCE_DATE_EPOCH", "1764076800")
        .output()
        .map_err(|e| format!("run packaging/sbom.sh: {e}"))?;
    assert!(run.status.success(), "sbom.sh failed:\n{}\n{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr));

    for dir in ["apps/curvebook", "apps/curvebook-cli", "crates/math"] {
        assert!(fs::read_dir(fake_repo.join(dir)).unwrap().next().is_none(), "leftover scratch files in {dir}");
    }
    let raw = fs::read_to_string(out_dir.join("curvebook-0.1.0-beta.1.cdx.json")).map_err(|e| e.to_string())?;
    assert!(!raw.contains(&ws_posix) && !raw.contains("download_url=file://"), "SBOM leaked local path: {raw}");
    let parsed: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    assert_eq!(
        (&parsed["bomFormat"], &parsed["specVersion"], &parsed["metadata"]["component"]["bom-ref"]),
        (&serde_json::json!("CycloneDX"), &serde_json::json!("1.5"), &serde_json::json!("workspace:apps/curvebook#0.1.0-beta.1"))
    );
    let comp_names: Vec<&str> = parsed["components"].as_array().unwrap().iter().filter_map(|c| c["name"].as_str()).collect();
    assert_eq!(comp_names, vec!["curvebook-cli", "curvebook-math", "curvebook-mcp", "serde"]);

    let scan = run_release_scan(&["--no-checksums", out_dir.to_str().unwrap()])?;
    assert!(scan.status.success(), "release-scan rejected generated SBOM:\n{}", String::from_utf8_lossy(&scan.stderr));
    Ok(())
}

#[cfg(unix)]
#[test]
fn r810_macos_package_sh_fails_closed_when_signing_required_without_credentials() {
    let pkg_sh = workspace_root().unwrap().join("packaging/macos/package.sh");
    let run_pkg = |extra: &[(&str, &str)]| {
        let mut cmd = Command::new("bash");
        cmd.arg(&pkg_sh).args(["--arch", "universal", "--require-signing", "--skip-build"]);
        for k in ["MACOS_SIGN_IDENTITY", "APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID"] {
            cmd.env_remove(k);
        }
        for &(k, v) in extra {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    };
    let missing_id = run_pkg(&[]);
    assert!(!missing_id.status.success() && String::from_utf8_lossy(&missing_id.stderr).contains("MACOS_SIGN_IDENTITY"));
    let missing_notary = run_pkg(&[("MACOS_SIGN_IDENTITY", "Developer ID Application: Test"), ("APPLE_ID", "test@example.com")]);
    assert!(!missing_notary.status.success() && String::from_utf8_lossy(&missing_notary.stderr).contains("APPLE_PASSWORD"));
}
