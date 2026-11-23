//! Permanent regression tests for Phase 9 Stage 9.1 (`FA-SEC23-01..02`, `FA-SEC23-04..16`,
//! `FA-X3-03`, and `FA-X3-05`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_SEQ: AtomicU64 = AtomicU64::new(0);

struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    fn new(label: &str) -> Result<Self, String> {
        let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!("cb-audit-p9-sec23-{label}-{}-{nanos}-{seq}", std::process::id()));
        fs::create_dir_all(&path).map_err(|e| format!("create_dir_all {}: {e}", path.display()))?;
        Ok(Self { path })
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(Path::to_path_buf).ok_or_else(|| String::from("CARGO_MANIFEST_DIR has no parent"))
}

fn run_xtask(cwd: &Path, args: &[&str], extra_env: &[(&str, Option<&str>)]) -> Result<Output, String> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xtask"));
    cmd.current_dir(cwd).args(args);
    for &(k, v) in extra_env {
        match v {
            Some(val) => {
                cmd.env(k, val);
            }
            None => {
                cmd.env_remove(k);
            }
        }
    }
    cmd.output().map_err(|e| format!("run xtask {args:?}: {e}"))
}

#[cfg(unix)]
fn build_ustar_header(path: &str, typeflag: u8, size: usize, linkname: &str) -> [u8; 512] {
    let mut hdr = [0u8; 512];
    let p_bytes = path.as_bytes();
    let p_len = p_bytes.len().min(100);
    hdr[..p_len].copy_from_slice(&p_bytes[..p_len]);
    hdr[100..108].copy_from_slice(b"0000644\0");
    hdr[108..116].copy_from_slice(b"0000000\0");
    hdr[116..124].copy_from_slice(b"0000000\0");
    hdr[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
    hdr[136..148].copy_from_slice(b"00000000000\0");
    hdr[148..156].fill(b' ');
    hdr[156] = typeflag;
    let l_bytes = linkname.as_bytes();
    let l_len = l_bytes.len().min(100);
    hdr[157..157 + l_len].copy_from_slice(&l_bytes[..l_len]);
    hdr[257..263].copy_from_slice(b"ustar\0");
    hdr[263..265].copy_from_slice(b"00");
    let chksum: u32 = hdr.iter().map(|&b| u32::from(b)).sum();
    hdr[148..156].copy_from_slice(format!("{chksum:06o}\0 ").as_bytes());
    hdr
}

#[cfg(unix)]
fn gzip_stored(data: &[u8]) -> Vec<u8> {
    let mut gz = vec![0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03];
    let chunks: Vec<&[u8]> = if data.is_empty() { vec![&[]] } else { data.chunks(65_535).collect() };
    for (idx, chunk) in chunks.iter().enumerate() {
        let is_last = u8::from(idx + 1 == chunks.len());
        let len = chunk.len() as u16;
        let nlen = !len;
        gz.push(is_last);
        gz.extend_from_slice(&len.to_le_bytes());
        gz.extend_from_slice(&nlen.to_le_bytes());
        gz.extend_from_slice(chunk);
    }
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320u32 & mask);
        }
    }
    gz.extend_from_slice(&(!crc).to_le_bytes());
    gz.extend_from_slice(&(data.len() as u32).to_le_bytes());
    gz
}

#[cfg(unix)]
#[test]
fn fa_sec23_01_release_scan_rejects_tar_symlink_hardlink_and_never_executes_elf_appimage() -> Result<(), String> {
    let root = workspace_root()?;
    let fx = ScratchDir::new("sec23-01")?;

    // 1. Symlink inside `.tar.gz` must be rejected by verbose listing BEFORE extraction.
    let sym_dir = fx.path.join("sym-tar");
    fs::create_dir_all(&sym_dir).map_err(|e| e.to_string())?;
    let mut tar_sym = Vec::new();
    tar_sym.extend_from_slice(&build_ustar_header("usr/bin/curvebook", b'0', 5, ""));
    let mut payload = b"clean".to_vec();
    payload.resize(512, 0);
    tar_sym.extend_from_slice(&payload);
    tar_sym.extend_from_slice(&build_ustar_header("usr/bin/link", b'2', 0, "/etc/passwd"));
    tar_sym.resize(tar_sym.len() + 1024, 0);
    let sym_tgz = gzip_stored(&tar_sym);
    fs::write(sym_dir.join("curvebook-0.1.0-linux-x86_64.tar.gz"), &sym_tgz).map_err(|e| e.to_string())?;
    let sym_sums = format!("{}  curvebook-0.1.0-linux-x86_64.tar.gz\n", curvebook_testkit::sha256_hex(&sym_tgz));
    fs::write(sym_dir.join("SHA256SUMS.txt"), sym_sums).map_err(|e| e.to_string())?;

    let sym_out = run_xtask(&root, &["release-scan", sym_dir.to_str().ok_or("utf8")?], &[])?;
    assert!(!sym_out.status.success(), "symlink inside .tar.gz must fail release-scan");
    let sym_err = String::from_utf8_lossy(&sym_out.stderr);
    assert!(sym_err.contains("symbolic link"), "expected verbose symlink rejection, got: {sym_err}");

    // 2. Hardlink inside `.tar.gz` must also be rejected BEFORE extraction.
    let hard_dir = fx.path.join("hard-tar");
    fs::create_dir_all(&hard_dir).map_err(|e| e.to_string())?;
    let mut tar_hard = Vec::new();
    tar_hard.extend_from_slice(&build_ustar_header("usr/bin/curvebook", b'0', 5, ""));
    tar_hard.extend_from_slice(&payload);
    tar_hard.extend_from_slice(&build_ustar_header("usr/bin/hard", b'1', 0, "/etc/passwd"));
    tar_hard.resize(tar_hard.len() + 1024, 0);
    let hard_tgz = gzip_stored(&tar_hard);
    fs::write(hard_dir.join("curvebook-0.1.0-linux-x86_64.tar.gz"), &hard_tgz).map_err(|e| e.to_string())?;
    let hard_sums = format!("{}  curvebook-0.1.0-linux-x86_64.tar.gz\n", curvebook_testkit::sha256_hex(&hard_tgz));
    fs::write(hard_dir.join("SHA256SUMS.txt"), hard_sums).map_err(|e| e.to_string())?;

    let hard_out = run_xtask(&root, &["release-scan", hard_dir.to_str().ok_or("utf8")?], &[])?;
    assert!(!hard_out.status.success(), "hardlink inside .tar.gz must fail release-scan");
    let hard_err = String::from_utf8_lossy(&hard_out.stderr);
    assert!(hard_err.contains("hard link"), "expected verbose hardlink rejection, got: {hard_err}");

    // 3. Untrusted ELF `.AppImage` without embedded SquashFS must fail closed WITHOUT executing the binary!
    let app_dir = fx.path.join("elf-appimage");
    fs::create_dir_all(&app_dir).map_err(|e| e.to_string())?;
    let app_bytes = b"\x7fELF\x02\x01\x01\x00#!/bin/sh\necho SHOULD_NEVER_RUN >&2\nexit 0\n";
    fs::write(app_dir.join("curvebook-0.1.0-linux-x86_64.AppImage"), app_bytes).map_err(|e| e.to_string())?;
    let app_sums = format!("{}  curvebook-0.1.0-linux-x86_64.AppImage\n", curvebook_testkit::sha256_hex(app_bytes));
    fs::write(app_dir.join("SHA256SUMS.txt"), app_sums).map_err(|e| e.to_string())?;

    let app_out = run_xtask(&root, &["release-scan", app_dir.to_str().ok_or("utf8")?], &[])?;
    assert!(!app_out.status.success(), "ELF AppImage without SquashFS must fail closed");
    let app_err = String::from_utf8_lossy(&app_out.stderr);
    assert!(app_err.contains("execution is disabled"), "expected disabled AppImage execution error, got: {app_err}");

    Ok(())
}

#[test]
fn fa_sec23_06_07_09_15_16_packaging_and_manifest_contracts_hold() -> Result<(), String> {
    let root = workspace_root()?;

    // FA-SEC23-16: Root Cargo.toml members must not duplicate crates/platform.
    let root_cargo = fs::read_to_string(root.join("Cargo.toml")).map_err(|e| e.to_string())?;
    let members_line = root_cargo.lines().find(|l| l.trim_start().starts_with("members =")).ok_or("missing members line in Cargo.toml")?;
    assert!(
        !members_line.contains("\"crates/platform\""),
        "Cargo.toml must not redundantly list \"crates/platform\" alongside \"crates/*\": {members_line}"
    );

    // FA-SEC23-06: ATTRIBUTION.md staged in packaging/env.sh, package.ps1, and bundle.rs (+ THIRD-PARTY-LICENSES.html in bundle.rs).
    let env_sh = fs::read_to_string(root.join("packaging/env.sh")).map_err(|e| e.to_string())?;
    assert!(env_sh.contains("ATTRIBUTION.md"), "packaging/env.sh missing ATTRIBUTION.md");
    let win_pkg = fs::read_to_string(root.join("packaging/windows/package.ps1")).map_err(|e| e.to_string())?;
    assert!(win_pkg.contains("ATTRIBUTION.md"), "package.ps1 missing ATTRIBUTION.md");
    let bundle_rs = fs::read_to_string(root.join("xtask/src/bundle.rs")).map_err(|e| e.to_string())?;
    assert!(bundle_rs.contains("ATTRIBUTION.md"), "bundle.rs missing ATTRIBUTION.md");
    assert!(bundle_rs.contains("THIRD-PARTY-LICENSES.html"), "bundle.rs missing THIRD-PARTY-LICENSES.html");
    assert!(bundle_rs.contains("\"--locked\""), "bundle.rs must pass --locked to cargo build");

    // FA-SEC23-07: linux package.sh verifies APPIMAGETOOL_SHA256 unconditionally & curl uses --proto-redir '=https'.
    let linux_pkg = fs::read_to_string(root.join("packaging/linux/package.sh")).map_err(|e| e.to_string())?;
    assert!(linux_pkg.contains("--proto-redir '=https'"));
    let verify_line = linux_pkg.find("verify_sha256 \"$APPIMAGETOOL_SHA256\" \"$TOOL\"").ok_or("missing APPIMAGETOOL verify_sha256")?;
    let if_start = linux_pkg.find("if [[ -z \"$TOOL\" ]]").ok_or("missing TOOL download guard")?;
    let if_block_end = linux_pkg.get(if_start..).and_then(|s| s.find("\nfi\n")).ok_or("missing fi for TOOL download")? + if_start;
    assert!(verify_line > if_block_end, "APPIMAGETOOL SHA256 verification must run unconditionally outside `if [[ -z \"$TOOL\" ]]`");

    let import_cert = fs::read_to_string(root.join("packaging/macos/import-cert.sh")).map_err(|e| e.to_string())?;
    assert!(import_cert.contains("trap 'rm -f \"$CERT_P12\"' EXIT"), "import-cert.sh must clean CERT_P12 on EXIT");

    // FA-SEC23-16: macOS package.sh guards $# -ge 2 on --arch.
    let macos_pkg = fs::read_to_string(root.join("packaging/macos/package.sh")).map_err(|e| e.to_string())?;
    assert!(macos_pkg.contains("$# -ge 2"), "macos/package.sh must check $# -ge 2 for --arch");

    // FA-SEC23-09: sign.ps1 has RequireSigning and avoids `/p` argv exposure; package.ps1 validates MZ/PE and +build.
    let win_sign = fs::read_to_string(root.join("packaging/windows/sign.ps1")).map_err(|e| e.to_string())?;
    assert!(win_sign.contains("$RequireSigning"), "sign.ps1 must support -RequireSigning");
    assert!(win_sign.contains("CURVEBOOK_REQUIRE_WINDOWS_SIGNING"));
    assert!(!win_sign.contains("'/p'"), "sign.ps1 must not expose PFX password via `/p` argv");
    assert!(win_sign.contains("'/sha1'"), "sign.ps1 must sign via cert store thumbprint");
    assert!(win_pkg.contains("($Version -split '[-+]')[0]"), "package.ps1 must strip both -prerelease and +build metadata");
    assert!(win_pkg.contains("0x4D") && win_pkg.contains("0x5A") && win_pkg.contains("0x50") && win_pkg.contains("0x45"));

    // FA-SEC23-10: icons.sh uses --locked.
    let icons_sh = fs::read_to_string(root.join("packaging/icons.sh")).map_err(|e| e.to_string())?;
    assert!(icons_sh.contains("cargo run --locked -p xtask -- ico"));

    Ok(())
}

#[test]
fn fa_sec23_10_16_x3_03_05_ci_workflows_hardening_holds() -> Result<(), String> {
    let root = workspace_root()?;
    let setup = fs::read_to_string(root.join(".github/actions/ci-setup/action.yml")).map_err(|e| e.to_string())?;
    assert!(setup.contains("--proto '=https' --proto-redir '=https'"));
    assert!(setup.contains("poppler-utils"));
    assert!(setup.contains("install-pdf-qa"));

    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).map_err(|e| e.to_string())?;
    assert!(ci.contains("install-pdf-qa: ${{ matrix.stage == 'tests' && 'true' || 'false' }}"));

    let cov = fs::read_to_string(root.join(".github/workflows/coverage.yml")).map_err(|e| e.to_string())?;
    assert!(cov.contains("--proto '=https' --proto-redir '=https'"));
    assert!(cov.contains("cargo install --locked cargo-llvm-cov --version "));

    let rel = fs::read_to_string(root.join(".github/workflows/release.yml")).map_err(|e| e.to_string())?;
    assert!(rel.contains("cargo run --locked -p xtask -- contributors"));
    assert!(rel.contains("r15_2_gonogo_1m_column_matched_benchmark_release -- --ignored"));
    assert!(rel.contains("-RequireSigning"));
    assert!(rel.contains("gh release delete-asset"));

    Ok(())
}

#[test]
fn fa_sec23_11_web_csp_and_macos_entitlements_hardened() -> Result<(), String> {
    let root = workspace_root()?;

    // macOS entitlements drop network.client on zero-telemetry desktop app.
    let ent = fs::read_to_string(root.join("packaging/macos/entitlements.plist")).map_err(|e| e.to_string())?;
    assert!(!ent.contains("com.apple.security.network.client"));

    // Web HTML/CSS extracted and style-src removes 'unsafe-inline' everywhere.
    let index = fs::read_to_string(root.join("apps/curvebook-web/index.html")).map_err(|e| e.to_string())?;
    assert!(index.contains("rel=\"css\" href=\"styles.css\""));
    assert!(!index.contains("<style>"));
    assert!(!index.contains(" style=\""));
    assert!(index.contains("style-src 'self';"));
    assert!(!index.contains("style-src 'self' 'unsafe-inline'"));
    assert!(root.join("apps/curvebook-web/styles.css").is_file());

    let headers = fs::read_to_string(root.join("packaging/web/_headers")).map_err(|e| e.to_string())?;
    assert!(headers.contains("style-src 'self';"));
    assert!(!headers.contains("style-src 'self' 'unsafe-inline'"));

    let htaccess = fs::read_to_string(root.join("packaging/web/.htaccess")).map_err(|e| e.to_string())?;
    assert!(htaccess.contains("style-src 'self';"));
    assert!(!htaccess.contains("style-src 'self' 'unsafe-inline'"));

    let web_pkg_sh = fs::read_to_string(root.join("packaging/web/package.sh")).map_err(|e| e.to_string())?;
    assert!(web_pkg_sh.contains("CARGO_NET_LOCKED=true"));
    assert!(web_pkg_sh.contains("sha256-"));

    // Simulate `cargo xtask web-fonts` rewriting an index.html inside TRUNK_STAGING_DIR.
    let fx = ScratchDir::new("webfonts-postbuild")?;
    let stage = fx.path.join("stage");
    fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let built_index = stage.join("index.html");
    fs::copy(root.join("apps/curvebook-web/index.html"), &built_index).map_err(|e| e.to_string())?;
    let out = run_xtask(&root, &["web-fonts"], &[("TRUNK_STAGING_DIR", Some(stage.to_str().ok_or("utf8")?)), ("CURVEBOOK_FONTS_DIR", None)])?;
    assert!(out.status.success(), "web-fonts failed: {}", String::from_utf8_lossy(&out.stderr));
    let stamped = fs::read_to_string(&built_index).map_err(|e| e.to_string())?;
    assert!(!stamped.contains("'unsafe-inline'"), "built index.html must have zero 'unsafe-inline': {stamped}");
    assert!(stamped.contains("'sha256-"), "built index.html missing script SHA-256 CSP hash");

    Ok(())
}

#[cfg(feature = "full-gates")]
#[test]
fn fa_sec23_08_docs_site_fails_on_missing_image_and_mcp_broken_link() -> Result<(), String> {
    let root = workspace_root()?;

    // Build a minimal workspace copy mirroring docs/ enough to trigger broken link/image in docs-site.
    let fx = ScratchDir::new("docs-site-sec23-08")?;
    let sim = &fx.path;
    for file in ["AGENTS.md", "NOTICE", "README.md", "SECURITY.md", "Cargo.toml", "ASSETS.md", "ATTRIBUTION.md"] {
        fs::copy(root.join(file), sim.join(file)).map_err(|e| format!("copy {file}: {e}"))?;
    }
    copy_dir_tree(&root.join("docs"), &sim.join("docs"))?;
    copy_dir_tree(&root.join("assets"), &sim.join("assets"))?;
    copy_dir_tree(&root.join("crates"), &sim.join("crates"))?;

    // 1. Missing image in markdown must fail (never silently substitute curvebook.png).
    let graphs_md = sim.join("docs/reference/graphs.md");
    let orig_graphs = fs::read_to_string(&graphs_md).map_err(|e| e.to_string())?;
    fs::write(&graphs_md, format!("{orig_graphs}\n![Broken](../images/does-not-exist-404.png)\n")).map_err(|e| e.to_string())?;
    let out_dir_1 = sim.join("out1");
    let res1 = run_xtask(
        sim,
        &["docs-site", "--check", "--output", out_dir_1.to_str().ok_or("utf8")?],
        &[("CURVEBOOK_DOCS_SITE_ROOT", Some(sim.to_str().ok_or("utf8")?))],
    )?;
    assert!(!res1.status.success(), "missing markdown image must fail docs-site");
    let err1 = String::from_utf8_lossy(&res1.stderr);
    assert!(err1.contains("does-not-exist-404.png"), "unexpected error: {err1}");
    fs::write(&graphs_md, orig_graphs).map_err(|e| e.to_string())?;

    // 2. Broken relative link inside docs/mcp-for-beginners.md must fail.
    let mcp_md = sim.join("docs/mcp-for-beginners.md");
    let orig_mcp = fs::read_to_string(&mcp_md).map_err(|e| e.to_string())?;
    fs::write(&mcp_md, format!("{orig_mcp}\nSee [Broken](../MISSING_REPO_DOC.md).\n")).map_err(|e| e.to_string())?;
    let out_dir_2 = sim.join("out2");
    let res2 = run_xtask(
        sim,
        &["docs-site", "--check", "--output", out_dir_2.to_str().ok_or("utf8")?],
        &[("CURVEBOOK_DOCS_SITE_ROOT", Some(sim.to_str().ok_or("utf8")?))],
    )?;
    assert!(!res2.status.success(), "broken relative link in mcp-for-beginners.md must fail docs-site");
    let err2 = String::from_utf8_lossy(&res2.stderr);
    assert!(err2.contains("MISSING_REPO_DOC.md"), "unexpected error: {err2}");

    Ok(())
}

#[cfg(feature = "full-gates")]
fn copy_dir_tree(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        let s = entry.path();
        let d = dst.join(entry.file_name());
        if ft.is_dir() {
            if entry.file_name() == "target" {
                continue;
            }
            copy_dir_tree(&s, &d)?;
        } else if ft.is_file() {
            fs::copy(&s, &d).map_err(|e| format!("copy {}: {e}", s.display()))?;
        }
    }
    Ok(())
}

#[test]
fn fa_sec23_12_14_stats_exact_and_remediated_xtask_platform_hygiene() -> Result<(), String> {
    let root = workspace_root()?;

    // FA-SEC23-12: `cargo xtask stats --exact` produces exact `sloc` column.
    let out = run_xtask(&root, &["stats", "--exact"], &[])?;
    assert!(out.status.success(), "stats --exact failed: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("sloc"), "stats --exact should output `sloc` column: {stdout}");

    // FA-SEC23-14: remediated xtask modules avoid raw std::fs::write and std::process::Command in non-test code.
    for rel in [
        "xtask/src/bundle.rs",
        "xtask/src/commands.rs",
        "xtask/src/contributors.rs",
        "xtask/src/corpus.rs",
        "xtask/src/coverage.rs",
        "xtask/src/ico.rs",
        "xtask/src/schemas.rs",
        "xtask/src/screenshots.rs",
        "xtask/src/ui_commands.rs",
        "xtask/src/docs_site/mod.rs",
        "xtask/src/docs_site/verify.rs",
    ] {
        let src = fs::read_to_string(root.join(rel)).map_err(|e| format!("read {rel}: {e}"))?;
        let non_test = src.split("#[cfg(test)]").next().unwrap_or(&src);
        assert!(!non_test.contains("std::fs::write("), "{rel} still has non-test std::fs::write(");
        assert!(!non_test.contains("fs::write("), "{rel} still has non-test fs::write(");
        assert!(!non_test.contains("std::process::Command"), "{rel} still uses std::process::Command");
        let has_bare_cmd_new = non_test
            .match_indices("Command::new(")
            .any(|(idx, _)| idx == 0 || (!non_test.as_bytes()[idx - 1].is_ascii_alphanumeric() && non_test.as_bytes()[idx - 1] != b'_'));
        assert!(!has_bare_cmd_new, "{rel} still has bare Command::new(");
    }

    Ok(())
}
