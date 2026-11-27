//! Regression gate for `R0.26` (`cargo xtask release-scan`): Beta/1.0 container fixture synthesis,
//! `SHA256SUMS.txt` integrity checks, planted brand/cleanroom/asset detection with container
//! attribution, path-traversal guards, and fail-closed handling of unknown or retired formats.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::common::{TempDirGuard, build_ustar_bytes, gzip_stored, run_release_scan, workspace_root, write_sha256sums_for_dir};
use super::zip_helper::ZipWriter;

fn join(parts: &[&str]) -> String {
    parts.concat()
}
fn read_repo(root: &Path, rel: &str) -> Result<Vec<u8>, String> {
    fs::read(root.join(rel)).map_err(|e| format!("read {rel}: {e}"))
}

#[cfg(target_os = "macos")]
fn try_create_macos_dmg(src_folder: &Path, dmg_out: &Path) -> bool {
    Command::new("/usr/bin/hdiutil")
        .args([
            "create",
            "-srcfolder",
            &src_folder.to_string_lossy(),
            "-volname",
            "CurveBook",
            "-fs",
            "HFS+",
            "-format",
            "UDRO",
            "-nospotlight",
            "-ov",
            &dmg_out.to_string_lossy(),
        ])
        .output()
        .is_ok_and(|o| o.status.success() && dmg_out.is_file())
}

fn assert_scan_fails_with(dir: &Path, needles: &[&str]) -> Result<(), String> {
    let out = run_release_scan(&[dir.to_str().ok_or("non-utf8 path")?])?;
    assert!(!out.status.success(), "expected release-scan to fail on {}", dir.display());
    let stderr = String::from_utf8_lossy(&out.stderr);
    for &needle in needles {
        assert!(stderr.contains(needle), "expected `{needle}` in stderr:\n{stderr}");
    }
    Ok(())
}

fn seed_single_and_fail(fx: &TempDirGuard, sub: &str, fname: &str, bytes: &[u8], needles: &[&str]) -> Result<(), String> {
    let dir = fx.path.join(sub);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(dir.join(fname), bytes).map_err(|e| e.to_string())?;
    write_sha256sums_for_dir(&dir)?;
    assert_scan_fails_with(&dir, needles)
}

#[test]
fn r026_clean_release_directory_passes_sha256_and_all_container_scanners() -> Result<(), String> {
    let (root, fx) = (workspace_root()?, TempDirGuard::new("clean")?);
    let rel_dir = fx.path.join("release");
    fs::create_dir_all(&rel_dir).unwrap();

    let icon_32 = read_repo(&root, "assets/app-icon/hicolor/32x32/apps/org.curvebook.CurveBook.png")?;
    let icon_svg = read_repo(&root, "assets/ui/toolbar_menus/new_project.svg")?;
    let mime_xml = read_repo(&root, "packaging/linux/org.curvebook.CurveBook.mime.xml")?;
    let desktop_entry = read_repo(&root, "packaging/linux/org.curvebook.CurveBook.desktop")?;
    let assets_md = read_repo(&root, "ASSETS.md")?;
    let notice = read_repo(&root, "NOTICE")?;

    fs::write(
        rel_dir.join("THIRD-PARTY-LICENSES.html"),
        "<!doctype html><html><head><title>CurveBook Third-Party Licenses</title></head><body><p>MIT OR Apache-2.0</p></body></html>\n",
    )
    .unwrap();
    fs::write(
        rel_dir.join("curvebook-0.1.0.cdx.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "bomFormat": "CycloneDX", "specVersion": "1.5", "version": 1,
            "metadata": {"component": {"type": "application", "name": "curvebook", "version": "0.1.0"}},
            "components": [{"type": "application", "name": "curvebook-cli", "version": "0.1.0"}, {"type": "library", "name": "serde", "version": "1.0.219", "purl": "pkg:cargo/serde@1.0.219"}]
        })).unwrap(),
    ).unwrap();

    let mut web_zip = ZipWriter::new();
    for (p, b) in [
        ("curvebook-web-0.1.0/index.html", b"<!doctype html><title>CurveBook</title><input accept=\".curvebook,.pzfx,.prism\">".as_slice()),
        ("curvebook-web-0.1.0/icons/32/org.curvebook.CurveBook.png", &icon_32),
        ("curvebook-web-0.1.0/icons/new_project.svg", &icon_svg),
        ("curvebook-web-0.1.0/ASSETS.md", &assets_md),
        ("curvebook-web-0.1.0/NOTICE", &notice),
    ] {
        web_zip.add_file(p, b.to_vec()).unwrap();
    }
    fs::write(rel_dir.join("curvebook-web-0.1.0.zip"), web_zip.finish().unwrap()).unwrap();

    let tgz_bytes = gzip_stored(&build_ustar_bytes(&[
        ("curvebook-0.1.0-freebsd-x86_64/usr/bin/curvebook", b"\x7fELF_clean_binary_stub"),
        ("curvebook-0.1.0-freebsd-x86_64/usr/share/applications/org.curvebook.CurveBook.desktop", &desktop_entry),
        ("curvebook-0.1.0-freebsd-x86_64/usr/share/mime/packages/org.curvebook.CurveBook.xml", &mime_xml),
        ("curvebook-0.1.0-freebsd-x86_64/usr/share/icons/hicolor/32x32/apps/org.curvebook.CurveBook.png", &icon_32),
        ("curvebook-0.1.0-freebsd-x86_64/usr/share/doc/curvebook/ASSETS.md", &assets_md),
        ("curvebook-0.1.0-freebsd-x86_64/usr/share/doc/curvebook/NOTICE", &notice),
    ])?);
    fs::write(rel_dir.join("curvebook-0.1.0-freebsd-x86_64.tar.gz"), tgz_bytes).unwrap();

    let mut expected_assets = 5usize;
    #[cfg(target_os = "macos")]
    {
        let dmg_src = fx.path.join("dmg-src");
        fs::create_dir_all(dmg_src.join("CurveBook.app/Contents/Resources")).unwrap();
        let plist = fs::read_to_string(root.join("packaging/macos/Info.plist.in"))
            .unwrap()
            .replace("@VERSION@", "0.1.0")
            .replace("@SHORT_VERSION@", "0.1.0")
            .replace("@BUILD_SHA@", "abc1234");
        fs::write(dmg_src.join("CurveBook.app/Contents/Info.plist"), plist).unwrap();
        fs::write(dmg_src.join("CurveBook.app/Contents/Resources/curvebook.icns"), read_repo(&root, "assets/app-icon/curvebook.icns")?).unwrap();
        std::os::unix::fs::symlink("/Applications", dmg_src.join("Applications")).unwrap();
        if try_create_macos_dmg(&dmg_src, &rel_dir.join("curvebook-0.1.0-macos-universal.dmg")) {
            expected_assets += 1;
        }
    }

    write_sha256sums_for_dir(&rel_dir)?;
    let report_path = fx.path.join("scan-report.json");
    let out = run_release_scan(&["--report", report_path.to_str().unwrap(), rel_dir.to_str().unwrap()])?;
    assert!(out.status.success(), "clean release-scan failed:\n{}", String::from_utf8_lossy(&out.stderr));

    let report: serde_json::Value = serde_json::from_str(&fs::read_to_string(&report_path).unwrap()).unwrap();
    assert_eq!(
        (&report["ok"], &report["checksum_verified"], &report["asset_count"]),
        (&serde_json::json!(true), &serde_json::json!(true), &serde_json::json!(expected_assets))
    );
    assert!(report["total_unpacked_files"].as_u64().unwrap() >= 10 && report["total_checked_assets"].as_u64().unwrap() >= 3);
    Ok(())
}

#[test]
fn r026_planted_brand_cleanroom_and_asset_violations_fail_closed_naming_container() -> Result<(), String> {
    let fx = TempDirGuard::new("planted")?;
    let dir = fx.path.join("planted-all");
    fs::create_dir_all(&dir).unwrap();
    let (brand_gp, brand_op, vendor_dm) = (join(&["Gr", "aph", "Pad"]), join(&["Ori", "gin", "Pro"]), join(&["Dot", "mat", "ics"]));

    let mut zw = ZipWriter::new();
    zw.add_file("curvebook-cli/README.md", format!("Compatibility wrapper for {brand_gp}\n").into_bytes()).unwrap();
    zw.add_file("curvebook-cli/fonts/Forbidden.ttf", vec![0x00, 0x01, 0x00, 0x00, 0x00]).unwrap();
    fs::write(dir.join("curvebook-cli-0.1.0.zip"), zw.finish().unwrap()).unwrap();

    let notice_leak = format!("<!-- \u{00a9} 2025 {vendor_dm}. All rights reserved. -->\n");
    let tgz = gzip_stored(&build_ustar_bytes(&[
        ("cb/README.md", format!("Replaces {brand_op} desktop\n").as_bytes()),
        ("cb/sample.xml", notice_leak.as_bytes()),
        ("cb/rogue.svg", b"<svg xmlns=\"http://www.w3.org/2000/svg\"><circle cx=\"8\" cy=\"8\" r=\"4\"/></svg>"),
    ])?);
    fs::write(dir.join("curvebook-0.1.0-freebsd-x86_64.tar.gz"), tgz).unwrap();
    fs::write(dir.join("THIRD-PARTY-LICENSES.html"), format!("<html><title>{brand_gp}</title></html>\n")).unwrap();
    fs::write(dir.join("curvebook-0.1.0.cdx.json"), serde_json::to_vec(&serde_json::json!({"bomFormat": "CycloneDX", "specVersion": "1.5", "components": [{"name": "helper", "description": format!("Imports {brand_gp} files")}]})).unwrap()).unwrap();

    write_sha256sums_for_dir(&dir)?;
    assert_scan_fails_with(
        &dir,
        &[
            "curvebook-cli-0.1.0.zip!curvebook-cli/README.md",
            "zero-bundled-fonts",
            "curvebook-0.1.0-freebsd-x86_64.tar.gz!",
            "[cleanroom]",
            "unattributed asset",
            "THIRD-PARTY-LICENSES.html",
            "curvebook-0.1.0.cdx.json",
        ],
    )
}

#[test]
fn r026_sha256sums_tampered_missing_extra_and_absent_manifest_fail_closed() -> Result<(), String> {
    let fx = TempDirGuard::new("checksums")?;
    let valid_zip = ZipWriter::single("curvebook-cli/README.txt", b"Clean CurveBook CLI\n")?;
    let mk = |sub: &str, name: &str| {
        let d = fx.path.join(sub);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(name), &valid_zip).unwrap();
        d
    };

    let tampered_dir = mk("tampered", "curvebook-cli.zip");
    write_sha256sums_for_dir(&tampered_dir)?;
    let mut tampered = valid_zip.clone();
    tampered[0] ^= 0xff;
    fs::write(tampered_dir.join("curvebook-cli.zip"), tampered).unwrap();
    assert_scan_fails_with(&tampered_dir, &["SHA-256 mismatch"])?;

    let missing_dir = mk("missing-entry", "curvebook-a.zip");
    write_sha256sums_for_dir(&missing_dir)?;
    fs::write(missing_dir.join("curvebook-b.zip"), &valid_zip).unwrap();
    assert_scan_fails_with(&missing_dir, &["curvebook-b.zip", "missing from `SHA256SUMS.txt`"])?;

    let extra_dir = mk("extra-entry", "curvebook-a.zip");
    write_sha256sums_for_dir(&extra_dir)?;
    let mut sums = fs::read_to_string(extra_dir.join("SHA256SUMS.txt")).unwrap();
    sums.push_str(&format!("{}  curvebook-ghost.zip\n", "0".repeat(64)));
    fs::write(extra_dir.join("SHA256SUMS.txt"), sums).unwrap();
    assert_scan_fails_with(&extra_dir, &["curvebook-ghost.zip", "does not exist"])?;

    let absent_dir = mk("absent-sums", "curvebook-a.zip");
    assert_scan_fails_with(&absent_dir, &["SHA256SUMS.txt"])?;
    let opt_out = run_release_scan(&["--no-checksums", absent_dir.to_str().unwrap()])?;
    assert!(opt_out.status.success(), "{}", String::from_utf8_lossy(&opt_out.stderr));
    Ok(())
}

#[test]
fn r026_unknown_retired_empty_traversal_and_unextractable_containers_fail_closed() -> Result<(), String> {
    let fx = TempDirGuard::new("fail-closed")?;
    let empty_dir = fx.path.join("empty");
    fs::create_dir_all(&empty_dir).unwrap();
    assert_scan_fails_with(&empty_dir, &["empty"])?;

    for unknown in [
        "curvebook-mystery.pkg",
        "curvebook-setup.exe",
        "curvebook-archive.rar",
        "curvebook-0.1.0.deb",
        "curvebook-0.1.0.rpm",
        "curvebook-0.1.0.flatpak",
        "curvebook-0.1.0.AppImage.zsync",
    ] {
        seed_single_and_fail(
            &fx,
            &format!("unknown-{unknown}"),
            unknown,
            b"opaque payload",
            &[unknown, "unsupported or unknown release asset format"],
        )?;
    }
    let evil_zip = ZipWriter::single("../escape.txt", b"escaped")?;
    let evil_tgz = gzip_stored(&build_ustar_bytes(&[("../escape.txt", b"escaped")])?);
    for (sub, fname, bytes, needles) in [
        ("zip-slip", "curvebook-slip.zip", evil_zip.as_slice(), &["curvebook-slip.zip"][..]),
        ("tar-slip", "curvebook-slip.tar.gz", evil_tgz.as_slice(), &["curvebook-slip.tar.gz", "../escape.txt"]),
        ("broken-appimage", "curvebook-broken.AppImage", b"\x7fELF_without_squashfs_superblock".as_slice(), &["curvebook-broken.AppImage"]),
    ] {
        seed_single_and_fail(&fx, sub, fname, bytes, needles)?;
    }
    #[cfg(not(windows))]
    seed_single_and_fail(&fx, "non-win-msi", "curvebook-0.1.0-windows-x64.msi", b"msi-placeholder", &["curvebook-0.1.0-windows-x64.msi"])?;
    #[cfg(target_os = "macos")]
    seed_single_and_fail(&fx, "bad-dmg", "curvebook-corrupt.dmg", b"not-a-valid-udif-dmg", &["curvebook-corrupt.dmg"])?;
    Ok(())
}

#[cfg(unix)]
fn write_mock_msiexec(dir: &Path, args_log: &Path) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("mock-msiexec");
    let body = format!(
        "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$1\" \"$2\" \"$3\" \"$4\" > \"{}\"\n[ \"$1\" = \"/a\" ] && [ \"$3\" = \"/qn\" ]\ntarget=\"${{4#TARGETDIR=}}\"\n[ \"$target\" != \"$4\" ]\ncp \"$2\" \"$target/$(basename \"$2\")\"\nmkdir -p \"$target/PFiles/CurveBook\"\ncp \"$2\" \"$target/PFiles/CurveBook/README.txt\"\n",
        args_log.display()
    );
    fs::write(&path, body).map_err(|e| format!("write mock-msiexec: {e}"))?;
    let mut perms = fs::metadata(&path).map_err(|e| e.to_string())?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).map_err(|e| e.to_string())?;
    Ok(path)
}

#[cfg(windows)]
fn write_mock_msiexec(dir: &Path, args_log: &Path) -> Result<PathBuf, String> {
    let path = dir.join("mock-msiexec.cmd");
    let body = format!(
        "@echo off\r\nsetlocal\r\nset \"MSI=%~2\"\r\nif \"%~4\"==\"TARGETDIR\" (set \"TARGET=%~5\") else (set \"TARGET=%~4\")\r\nif \"%TARGET:~0,10%\"==\"TARGETDIR=\" set \"TARGET=%TARGET:~10%\"\r\n> \"{}\" (echo %~1& echo %MSI%& echo %~3& echo TARGETDIR=%TARGET%)\r\ncopy /Y \"%MSI%\" \"%TARGET%\\%~nx2\" >nul\r\nmkdir \"%TARGET%\\PFiles\\CurveBook\"\r\ncopy /Y \"%MSI%\" \"%TARGET%\\PFiles\\CurveBook\\README.txt\" >nul\r\n",
        args_log.display()
    );
    fs::write(&path, body).map_err(|e| format!("write mock-msiexec.cmd: {e}"))?;
    Ok(path)
}

#[test]
fn r026_msi_scan_on_relative_dir_passes_absolute_paths_to_msiexec() -> Result<(), String> {
    let fx = TempDirGuard::new("msi-relative")?;
    let bin_dir = fx.path.join("mock-bin");
    fs::create_dir_all(&bin_dir).map_err(|e| e.to_string())?;
    let args_log = fx.path.join("msiexec-args.log");
    let mock_msiexec = write_mock_msiexec(&bin_dir, &args_log)?;

    let (rel_clean, rel_dirty) = (Path::new("dist/release"), Path::new("dist/dirty-release"));
    let (clean_dir, dirty_dir) = (fx.path.join(rel_clean), fx.path.join(rel_dirty));
    fs::create_dir_all(&clean_dir).map_err(|e| e.to_string())?;
    let clean_msi = clean_dir.join("curvebook-0.1.0-windows-x64.msi");
    fs::write(&clean_msi, b"Clean CurveBook Windows installer payload\n").map_err(|e| e.to_string())?;
    write_sha256sums_for_dir(&clean_dir)?;

    let out = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .current_dir(&fx.path)
        .args(["release-scan", "--report", "scan-report.json", rel_clean.to_str().unwrap()])
        .env("CURVEBOOK_RELEASE_SCAN_MSIEXEC", mock_msiexec.to_str().unwrap())
        .output()
        .map_err(|e| format!("run release-scan: {e}"))?;
    assert!(out.status.success(), "relative msi release-scan failed:\n{}", String::from_utf8_lossy(&out.stderr));

    let logged = fs::read_to_string(&args_log).map_err(|e| e.to_string())?;
    let lines: Vec<&str> = logged.lines().map(str::trim_end).collect();
    let target_arg = lines[3].strip_prefix("TARGETDIR=").unwrap();
    assert_eq!((lines.len(), lines[0], lines[2]), (4, "/a", "/qn"));
    assert!(
        Path::new(lines[1]).is_absolute()
            && Path::new(target_arg).is_absolute()
            && !lines[1].starts_with(r"\\?\")
            && !target_arg.starts_with(r"\\?\")
    );
    assert_eq!(fs::canonicalize(lines[1]).unwrap(), fs::canonicalize(&clean_msi).unwrap());

    fs::create_dir_all(&dirty_dir).map_err(|e| e.to_string())?;
    fs::write(dirty_dir.join("curvebook-0.1.0-windows-x64.msi"), format!("Bundled {} guide\n", join(&["Ori", "gin", "Pro"]))).unwrap();
    write_sha256sums_for_dir(&dirty_dir)?;
    let dirty_out = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .current_dir(&fx.path)
        .args(["release-scan", rel_dirty.to_str().unwrap()])
        .env("CURVEBOOK_RELEASE_SCAN_MSIEXEC", mock_msiexec.to_str().unwrap())
        .output()
        .map_err(|e| e.to_string())?;
    assert!(
        !dirty_out.status.success()
            && String::from_utf8_lossy(&dirty_out.stderr).contains("curvebook-0.1.0-windows-x64.msi!PFiles/CurveBook/README.txt")
    );
    Ok(())
}

#[test]
fn r14_2_cyclonedx_sbom_schema_cleanroom_and_secret_scans_fail_closed() -> Result<(), String> {
    let fx = TempDirGuard::new("r14-2-sbom")?;
    let sbom_name = "curvebook-0.1.0-beta.1.cdx.json";
    for (label, body, expected) in [
        ("invalid-json", "{not valid json\n", "invalid CycloneDX JSON"),
        ("wrong-format", "{\"bomFormat\":\"SPDX\",\"specVersion\":\"2.3\",\"components\":[{\"name\":\"serde\"}]}\n", "bomFormat"),
        ("empty-components", "{\"bomFormat\":\"CycloneDX\",\"specVersion\":\"1.5\",\"components\":[]}\n", "empty `components`"),
    ] {
        seed_single_and_fail(&fx, label, sbom_name, body.as_bytes(), &[sbom_name, expected])?;
    }
    let make_sbom = |extra_comp: serde_json::Value| {
        serde_json::to_vec(&serde_json::json!({
            "bomFormat": "CycloneDX", "specVersion": "1.5", "version": 1,
            "metadata": {"component": {"type": "application", "name": "curvebook", "version": "0.1.0-beta.1"}},
            "components": [{"type": "application", "name": "curvebook-cli", "version": "0.1.0-beta.1"}, extra_comp]
        }))
        .unwrap()
    };
    let (leaked_path, leaked_token, vendor_dm) =
        (join(&["path+file:///", "home/", "runner/work/curvebook/crates/math"]), join(&["ghp_", "AbCdEfGh123456"]), join(&["Dot", "mat", "ics"]));
    for (sub, comp, needle) in [
        ("secret-home", serde_json::json!({"name": "curvebook-math", "bom-ref": leaked_path}), "[secrets]"),
        ("secret-token", serde_json::json!({"name": "curvebook-math", "bom-ref": leaked_token}), "[secrets]"),
        ("cleanroom-sbom", serde_json::json!({"name": "curvebook-doc", "copyright": format!("\u{00a9} 2025 {vendor_dm}")}), "[cleanroom]"),
    ] {
        seed_single_and_fail(&fx, sub, sbom_name, &make_sbom(comp), &[sbom_name, needle])?;
    }

    // 4. The shipped `curvebook-cli` component must appear exactly once with the root version (`R0.36`).
    let cli = |version: Option<&str>| match version {
        Some(v) => serde_json::json!({ "type": "application", "name": "curvebook-cli", "version": v }),
        None => serde_json::json!({ "type": "application", "name": "curvebook-cli" }),
    };
    for (label, cli_components, expected) in [
        ("cli-missing", vec![], "missing shipped binary component `curvebook-cli`"),
        ("cli-no-version", vec![cli(None)], "`curvebook-cli` is missing a non-empty `version`"),
        ("cli-empty-version", vec![cli(Some("  "))], "`curvebook-cli` is missing a non-empty `version`"),
        ("cli-duplicate", vec![cli(Some("0.0.9")), cli(Some("0.1.0-beta.1"))], "`curvebook-cli` 2 times"),
        ("cli-mismatch", vec![cli(Some("0.0.9"))], "does not match root version"),
    ] {
        let dir = fx.path.join(label);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut components = cli_components;
        components.push(serde_json::json!({ "type": "library", "name": "serde", "version": "1.0.0" }));
        let sbom = serde_json::to_vec_pretty(&serde_json::json!({
            "bomFormat": "CycloneDX",
            "specVersion": "1.5",
            "version": 1,
            "metadata": { "component": { "type": "application", "name": "curvebook", "version": "0.1.0-beta.1" } },
            "components": components
        }))
        .map_err(|e| e.to_string())?;
        fs::write(dir.join(sbom_name), sbom).map_err(|e| e.to_string())?;
        write_sha256sums_for_dir(&dir)?;

        let out = run_release_scan(&[dir.to_str().ok_or("non-UTF-8 fixture path")?])?;
        assert!(!out.status.success(), "{label} must fail closed");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(sbom_name) && stderr.contains(expected), "{label}: {stderr}");
    }
    Ok(())
}
