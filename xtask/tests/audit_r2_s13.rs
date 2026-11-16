//! Permanent regression tests for Round 2 (`R0.19`) S13 (`R2-S13-01..11`) and S11 (`R2-S11-10..11`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(label: &str) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!("cb-audit-r2-s13-{label}-{}-{nanos}-{seq}", std::process::id()));
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

fn run_xtask(args: &[&str], extra_env: &[(&str, Option<&str>)]) -> Result<std::process::Output, String> {
    let root = workspace_root()?;
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xtask"));
    cmd.current_dir(root).args(args);
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

#[test]
fn r2_s13_01_release_workflow_pins_license_tools_and_html_includes_full_texts() {
    let root = workspace_root().unwrap();
    let release = fs::read_to_string(root.join(".github/workflows/release.yml")).unwrap();
    assert!(release.contains("cargo install --locked cargo-deny --version 0.20.2"));
    assert!(release.contains("cargo install --locked cargo-about --version 0.9.3"));
    assert!(release.contains("cargo deny check licenses"));
    assert!(release.contains("cargo about generate about.hbs -o licenses/THIRD-PARTY-LICENSES.html"));
    assert!(release.contains("CI: \"true\""));
    assert!(!release.contains("out=\"$(cargo xtask licenses --html)\""));

    let out_dir = TempDirGuard::new("lic-html").unwrap();
    let html_path = out_dir.path.join("THIRD-PARTY-LICENSES.html");
    let out = run_xtask(&["licenses", "--html", html_path.to_str().unwrap()], &[("CI", Some("false")), ("CARGO_NET_OFFLINE", Some("true"))]).unwrap();
    assert!(out.status.success(), "stdout={}\nstderr={}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let html = fs::read_to_string(&html_path).unwrap();
    assert!(html.contains("SPDX License"));
    assert!(html.contains("Full Third-Party License Texts"));
    assert!(html.contains("Permission is hereby granted, free of charge"));
}

#[test]
fn r2_s13_02_release_workflow_freebsd_and_all_jobs_avoid_script_injection() {
    let root = workspace_root().unwrap();
    let release = fs::read_to_string(root.join(".github/workflows/release.yml")).unwrap();
    assert!(release.contains("envs: 'INPUT_VERSION GITHUB_REF_TYPE GITHUB_REF_NAME'"));
    assert!(!release.contains("CURVEBOOK_VERSION=\"${{ inputs.version }}\""));
    assert!(!release.contains("nfpm_arch=\"${{"));

    let out = run_xtask(&["telemetry"], &[]).unwrap();
    assert!(out.status.success(), "telemetry failed: {}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn r2_s13_03_release_workflow_enforces_semver_parity_and_draft_guard() {
    let root = workspace_root().unwrap();
    let release = fs::read_to_string(root.join(".github/workflows/release.yml")).unwrap();
    assert!(release.contains("Validate release version parity"));
    assert!(release.contains("disagrees with Cargo.toml version"));
    assert!(release.contains("gh release view \"$tag\" --json isDraft --jq .isDraft"));
    assert!(release.contains("refusing to overwrite already-published release"));
}

#[test]
fn r2_s13_04_all_workflows_pin_40_char_shas_and_verify_tool_checksums() {
    let root = workspace_root().unwrap();
    let wf_dir = root.join(".github/workflows");
    let mut checked = 0usize;
    for entry in fs::read_dir(wf_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("yml") {
            continue;
        }
        checked += 1;
        let text = fs::read_to_string(&path).unwrap();
        for (idx, line) in text.lines().enumerate() {
            let trimmed = line.split('#').next().unwrap_or("").trim();
            let after_dash = trimmed.strip_prefix('-').map_or(trimmed, str::trim_start);
            if let Some(uses_rhs) = after_dash.strip_prefix("uses:") {
                let spec = uses_rhs.trim();
                let split = spec.split_once('@');
                assert!(split.is_some(), "{}:{}: missing `@sha` in `{spec}`", path.display(), idx + 1);
                let sha = split.unwrap().1;
                assert_eq!(sha.len(), 40, "{}:{}: `{spec}` not 40-char SHA", path.display(), idx + 1);
                assert!(sha.bytes().all(|b| b.is_ascii_hexdigit()), "{}:{}: invalid hex in `{spec}`", path.display(), idx + 1);
            }
        }
    }
    assert!(checked >= 6);

    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    assert!(!ci.contains("download-actionlint.bash"));
    assert!(ci.contains("8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8"));
    let release = fs::read_to_string(root.join(".github/workflows/release.yml")).unwrap();
    assert!(release.contains("17350a838c8e2c422c6e573ed379b18424565d2de8a2b1cb1b20211976124eb5"));
    assert!(release.contains("b3cf95aa6dabed836d09ad7f0c190a13c74c5b1304db60846f0f702ee407f430"));
}

#[test]
fn r2_s13_05_and_r2_s11_11_web_headers_coop_coep_csp_and_subpath_rules() {
    let root = workspace_root().unwrap();
    let headers = fs::read_to_string(root.join("packaging/web/_headers")).unwrap();
    let htaccess = fs::read_to_string(root.join("packaging/web/.htaccess")).unwrap();
    let agents = fs::read_to_string(root.join("AGENTS.md")).unwrap();
    let releasing = fs::read_to_string(root.join("docs/releasing.md")).unwrap();

    for expected in [
        "Cross-Origin-Opener-Policy: same-origin",
        "Cross-Origin-Embedder-Policy: require-corp",
        "connect-src 'self'",
        "/*",
        "/*.wasm",
        "/*.js",
        "/fonts/*",
        "/\n",
        "/index.html",
        "/curvebook/",
        "/curvebook/index.html",
        "/curvebook/*.wasm",
        "/curvebook/*.js",
        "/curvebook/fonts/*",
    ] {
        assert!(headers.contains(expected), "_headers missing `{expected}`");
    }
    assert!(htaccess.contains("Cross-Origin-Opener-Policy \"same-origin\""));
    assert!(htaccess.contains("Cross-Origin-Embedder-Policy \"require-corp\""));
    assert!(htaccess.contains("connect-src 'self'"));
    assert!(agents.contains("connect-src 'self'"));
    assert!(releasing.contains("connect-src 'self'"));
}

#[test]
fn r2_s13_08_09_10_11_xtask_cli_gates_and_ci_falsiness() {
    for gate in ["layers", "secrets", "licenses", "telemetry", "cleanroom", "status"] {
        let out = run_xtask(&[gate], &[("CI", Some("false")), ("CARGO_NET_OFFLINE", Some("true"))]).unwrap();
        assert!(out.status.success(), "`xtask {gate}` with CI=false failed: {}", String::from_utf8_lossy(&out.stderr));
    }

    // With empty PATH (zero external linters), `xtask workflows` must fail even when CI is unset.
    let no_linters = run_xtask(&["workflows"], &[("CI", None), ("PATH", Some("/nonexistent-empty-bin"))]).unwrap();
    assert!(!no_linters.status.success(), "expected `xtask workflows` without linters to fail");
    let err_txt = String::from_utf8_lossy(&no_linters.stderr);
    assert!(err_txt.contains("actionlint") && err_txt.contains("shellcheck"), "{err_txt}");

    let root = workspace_root().unwrap();
    let control_proto = fs::read_to_string(root.join("docs/control-protocol.md")).unwrap();
    let agents = fs::read_to_string(root.join("AGENTS.md")).unwrap();
    assert!(control_proto.contains("--window"), "R2-S11-09 missing --window in control-protocol.md");
    assert!(control_proto.contains("all synced `UI_COMMANDS`") || control_proto.contains("`45` synced `UI_COMMANDS`"));
    assert!(!control_proto.contains("`32` synced `UI_COMMANDS`"));
    assert!(agents.contains("`97` commands"));
    assert!(!agents.contains("once R0 generators land"));
    assert!(agents.contains("cargo xtask workflows"));

    let main_rs = fs::read_to_string(root.join("xtask/src/main.rs")).unwrap();
    assert!(main_rs.contains("workflows::is_ci_env()"), "R2-S13-09: cmd_ci must use workflows::is_ci_env()");
    assert!(!main_rs.contains("var_os(\"CI\")"), "R2-S13-09: raw var_os(\"CI\") check must not remain in xtask/src/main.rs");
}

#[cfg(unix)]
#[test]
fn r2_s11_10_macos_verify_sh_checks_dmg_app_and_require_notarized_cli() {
    use std::os::unix::fs::PermissionsExt;

    let root = workspace_root().unwrap();
    let verify_sh = root.join("packaging/macos/verify.sh");
    let syntax = Command::new("bash").arg("-n").arg(&verify_sh).output().unwrap();
    assert!(syntax.status.success(), "{}", String::from_utf8_lossy(&syntax.stderr));

    let fx = TempDirGuard::new("verify-sh").unwrap();
    let bin_dir = fx.path.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let dmg_mount = fx.path.join("mount");
    let cli_bin_dir = fx.path.join("cli-stage");
    fs::create_dir_all(&dmg_mount).unwrap();
    fs::create_dir_all(&cli_bin_dir).unwrap();

    let write_exe = |name: &str, body: &str| {
        let p = bin_dir.join(name);
        fs::write(&p, body).unwrap();
        let mut perms = fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&p, perms).unwrap();
    };

    write_exe(
        "hdiutil",
        &format!(
            "#!/usr/bin/env bash\ncase \"$1\" in\n  attach) cp -R \"{}/.\" \"$4/\" 2>/dev/null || true ; printf '/dev/disk99\\tApple_HFS\\t%s\\n' \"$4\" ;;\n  detach) exit 0 ;;\nesac\n",
            dmg_mount.display()
        ),
    );
    write_exe("plutil", "#!/usr/bin/env bash\nexit 0\n");
    write_exe(
        "codesign",
        "#!/usr/bin/env bash\nif [ \"$1\" = \"-dv\" ]; then\n  if [ \"${MOCK_SIGNED:-0}\" = \"1\" ]; then\n    echo 'Authority=Developer ID Application: Test' >&2\n    echo 'flags=0x10000(runtime)' >&2\n    echo 'Timestamp=Jan 1, 2026' >&2\n  else\n    echo 'Signature=adhoc' >&2\n  fi\n  exit 0\nfi\nif [ \"$1\" = \"--test-requirement==notarized\" ]; then\n  if [ \"${MOCK_NOTARIZED:-0}\" = \"1\" ]; then exit 0; fi\n  echo 'rejected (un-notarized executable)' >&2\n  exit 1\nfi\nexit 0\n",
    );
    write_exe("xcrun", "#!/usr/bin/env bash\nif [ \"${MOCK_NOTARIZED:-0}\" = \"1\" ]; then exit 0; fi\nexit 1\n");
    write_exe(
        "spctl",
        "#!/usr/bin/env bash\nlast_arg=\"${!#}\"\ncase \"$last_arg\" in\n  *.app|*.dmg)\n    if [ \"${MOCK_NOTARIZED:-0}\" = \"1\" ]; then exit 0; fi\n    ;;\n  *)\n    echo 'rejected (the code is valid but does not seem to be an app)' >&2\n    exit 3\n    ;;\nesac\necho 'rejected (not notarized)' >&2\nexit 3\n",
    );
    write_exe("lipo", "#!/usr/bin/env bash\necho 'x86_64 arm64'\n");
    write_exe("arch", "#!/usr/bin/env bash\nshift\nexec \"$@\"\n");
    write_exe("ditto", &format!("#!/usr/bin/env bash\ncp \"{}/curvebook-cli\" \"$4/curvebook-cli\"\n", cli_bin_dir.display()));

    let mock_cli = cli_bin_dir.join("curvebook-cli");
    fs::write(&mock_cli, "#!/usr/bin/env bash\necho 'curvebook-cli 0.1.0'\n").unwrap();
    let mut perms = fs::metadata(&mock_cli).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&mock_cli, perms.clone()).unwrap();

    let dmg = fx.path.join("app.dmg");
    let zip = fx.path.join("cli.zip");
    fs::write(&dmg, b"dmg").unwrap();
    fs::write(&zip, b"zip").unwrap();

    let path_env = format!("{}:/usr/bin:/bin", bin_dir.display());
    // 1. Missing CurveBook.app inside mounted DMG must fail.
    let missing_app = Command::new("bash")
        .arg(&verify_sh)
        .args(["--arch", "universal", dmg.to_str().unwrap(), zip.to_str().unwrap()])
        .env("PATH", &path_env)
        .output()
        .unwrap();
    assert!(!missing_app.status.success());
    assert!(String::from_utf8_lossy(&missing_app.stderr).contains("CurveBook.app"), "{}", String::from_utf8_lossy(&missing_app.stderr));

    // Create CurveBook.app inside mounted DMG.
    fs::create_dir_all(dmg_mount.join("CurveBook.app/Contents/MacOS")).unwrap();
    fs::write(dmg_mount.join("CurveBook.app/Contents/Info.plist"), "<plist/>").unwrap();
    let app_bin = dmg_mount.join("CurveBook.app/Contents/MacOS/curvebook");
    fs::write(&app_bin, "#!/usr/bin/env bash\nexit 0\n").unwrap();
    fs::set_permissions(&app_bin, perms).unwrap();

    // 2. Ad-hoc CLI under `--require-notarized` must fail.
    let adhoc_notarize = Command::new("bash")
        .arg(&verify_sh)
        .args(["--arch", "universal", "--require-notarized", dmg.to_str().unwrap(), zip.to_str().unwrap()])
        .env("PATH", &path_env)
        .env("MOCK_SIGNED", "0")
        .output()
        .unwrap();
    assert!(!adhoc_notarize.status.success());

    // 3. Developer-ID signed but un-notarized standalone CLI `.zip` must fail
    // `codesign --test-requirement="=notarized"` inside `verify_cli_zip`.
    let signed_unnotarized = Command::new("bash")
        .arg(&verify_sh)
        .args(["--arch", "universal", "--require-notarized", zip.to_str().unwrap()])
        .env("PATH", &path_env)
        .env("MOCK_SIGNED", "1")
        .env("MOCK_NOTARIZED", "0")
        .output()
        .unwrap();
    assert!(!signed_unnotarized.status.success());
    assert!(
        String::from_utf8_lossy(&signed_unnotarized.stderr).contains("un-notarized"),
        "expected notarization check failure in verify_cli_zip, got stderr={}",
        String::from_utf8_lossy(&signed_unnotarized.stderr)
    );

    // 4. Properly signed + notarized `.dmg` & standalone CLI `.zip` pass `--require-notarized`
    // without calling `spctl` on the bare CLI executable.
    let pass = Command::new("bash")
        .arg(&verify_sh)
        .args(["--arch", "universal", "--require-notarized", dmg.to_str().unwrap(), zip.to_str().unwrap()])
        .env("PATH", &path_env)
        .env("MOCK_SIGNED", "1")
        .env("MOCK_NOTARIZED", "1")
        .output()
        .unwrap();
    assert!(pass.status.success(), "stderr={}", String::from_utf8_lossy(&pass.stderr));
}
