//! Permanent regression tests for Round 2 (`R0.19`) S13 (`R2-S13-01..11`) and S11 (`R2-S11-10..11`).

use std::fs;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Command;

use super::common::{TempDirGuard, run_xtask, workspace_root};

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))? {
        let path = entry.map_err(|e| format!("read_dir entry in {}: {e}", dir.display()))?.path();
        if path.is_dir() {
            collect_rs_files(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

#[test]
fn r2_s13_01_release_workflow_pins_license_tools_and_html_includes_full_texts() {
    let root = workspace_root().unwrap();
    let release = fs::read_to_string(root.join(".github/workflows/release.yml")).unwrap();
    assert!(release.contains("cargo install --locked cargo-deny --version 0.20.2"));
    assert!(release.contains("cargo install --locked cargo-about --version 0.9.3 --features cli"));
    assert!(release.contains("cargo deny check licenses"));
    assert!(release.contains("cargo about generate about.hbs -o licenses/THIRD-PARTY-LICENSES.html"));
    assert!(release.contains("CI: \"true\""));
    assert!(!release.contains("out=\"$(cargo xtask licenses --html)\""));

    let out_dir = TempDirGuard::new("lic-html").unwrap();
    let html_path = out_dir.path.join("THIRD-PARTY-LICENSES.html");
    let out = run_xtask(&["licenses", "--html", html_path.to_str().unwrap()], &[("CI", Some("false")), ("CARGO_NET_OFFLINE", Some("true"))]).unwrap();
    assert!(out.status.success(), "stdout={}\nstderr={}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let html = fs::read_to_string(&html_path).unwrap();
    assert!(html.contains("CurveBook — Third-Party Software Licenses"));
    assert!(html.contains("License overview"));
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
    let mut files = fs::read_dir(wf_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("yml"))
        .collect::<Vec<_>>();
    files.push(root.join(".github/actions/ci-setup/action.yml"));
    for path in files {
        checked += 1;
        let text = fs::read_to_string(&path).unwrap();
        for (idx, line) in text.lines().enumerate() {
            let trimmed = line.split('#').next().unwrap_or("").trim();
            let after_dash = trimmed.strip_prefix('-').map_or(trimmed, str::trim_start);
            if let Some(uses_rhs) = after_dash.strip_prefix("uses:") {
                let spec = uses_rhs.trim();
                if spec.starts_with("./") {
                    // This action is pinned by the checkout revision; audit its external
                    // actions and tool checksums in the same loop, rather than bypassing it.
                    assert_eq!(spec, "./.github/actions/ci-setup", "unexpected local action in {}", path.display());
                    assert!(root.join(spec).join("action.yml").is_file());
                    continue;
                }
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
    let setup = fs::read_to_string(root.join(".github/actions/ci-setup/action.yml")).unwrap();
    assert!(ci.contains("uses: ./.github/actions/ci-setup"));
    assert!(!ci.contains("download-actionlint.bash") && !setup.contains("download-actionlint.bash"));
    assert!(setup.contains("8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8"));
    assert!(setup.contains("cargo-nextest-0.9.152-x86_64-unknown-linux-gnu.tar.gz"));
    assert!(setup.contains("d039b43c3e0958441b6ad0a39c8884d018b3d2e8fb9928f209074b1100b5e17a"));
    assert!(setup.contains("rm -f \"$tarball\""));
    assert!(setup.contains("reclaim-disk"));
    let coverage = fs::read_to_string(root.join(".github/workflows/coverage.yml")).unwrap();
    assert!(coverage.contains("d039b43c3e0958441b6ad0a39c8884d018b3d2e8fb9928f209074b1100b5e17a"));
    let release = fs::read_to_string(root.join(".github/workflows/release.yml")).unwrap();
    assert!(!release.contains("nfpm"), "nfpm must be removed from release.yml under R0.28");
    let linux_pkg = fs::read_to_string(root.join("packaging/linux/package.sh")).unwrap();
    assert!(linux_pkg.contains("ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"));
    assert!(linux_pkg.contains("f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158"));
    assert!(linux_pkg.contains("2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d"));
    assert!(linux_pkg.contains("00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444"));
}

#[test]
fn r2_s13_05_and_r2_s11_11_web_headers_coop_coep_csp_and_subpath_rules() {
    let root = workspace_root().unwrap();
    let headers = fs::read_to_string(root.join("packaging/web/_headers")).unwrap();
    let htaccess = fs::read_to_string(root.join("packaging/web/.htaccess")).unwrap();
    let agents = fs::read_to_string(root.join("AGENTS.md")).unwrap();
    let releasing = fs::read_to_string(root.join("docs/release/releasing.md")).unwrap();

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
    let no_linter_path = ("PATH", Some("/nonexistent-empty-bin"));
    for ci in [None, Some("false"), Some("0")] {
        let out = run_xtask(&["workflows"], &[("CI", ci), no_linter_path]).unwrap();
        assert!(!out.status.success(), "expected `xtask workflows` without linters to fail (CI={ci:?})");
        let err_txt = String::from_utf8_lossy(&out.stderr);
        assert!(err_txt.contains("neither `actionlint` nor `shellcheck` is installed"), "CI={ci:?} must take the non-CI path: {err_txt}");
        assert!(!err_txt.contains("is not installed in PATH (required when CI=true)"), "CI={ci:?} must not be treated as CI: {err_txt}");
    }
    let in_ci = run_xtask(&["workflows"], &[("CI", Some("true")), no_linter_path]).unwrap();
    assert!(!in_ci.status.success(), "expected `xtask workflows` without linters to fail when CI=true");
    let err_txt = String::from_utf8_lossy(&in_ci.stderr);
    assert!(err_txt.contains("`actionlint` is not installed in PATH (required when CI=true)"), "{err_txt}");

    let root = workspace_root().unwrap();
    let control_proto = fs::read_to_string(root.join("docs/control-protocol.md")).unwrap();
    let agents = fs::read_to_string(root.join("AGENTS.md")).unwrap();
    assert!(control_proto.contains("--window"), "R2-S11-09 missing --window in control-protocol.md");
    assert!(control_proto.contains("all synced `UI_COMMANDS`") || control_proto.contains("`45` synced `UI_COMMANDS`"));
    assert!(!control_proto.contains("`32` synced `UI_COMMANDS`"));
    assert!(agents.contains("`116` commands") && !agents.contains("once R0 generators land") && agents.contains("cargo xtask workflows"));
    assert!(fs::read_to_string(root.join("xtask/src/main.rs")).unwrap().contains("xtask::run_cli()"));
    assert!(fs::read_to_string(root.join("xtask/src/lib.rs")).unwrap().contains("ci::run(&root, rest)"));

    let mut sources = Vec::new();
    collect_rs_files(&root.join("xtask/src"), &mut sources).unwrap();
    assert!(sources.len() > 1, "expected xtask sources under xtask/src");
    for path in sources.iter().filter(|p| !p.ends_with("workflows.rs")) {
        let text = fs::read_to_string(path).unwrap();
        for raw in ["var_os(\"CI\")", "var(\"CI\")"] {
            assert!(!text.contains(raw), "R2-S13-09: {} reads `{raw}` instead of workflows::is_ci_env()", path.display());
        }
    }
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
    let (bin_dir, dmg_mount) = (fx.path.join("bin"), fx.path.join("mount"));
    fs::create_dir_all(&bin_dir).unwrap();
    fs::create_dir_all(&dmg_mount).unwrap();
    let write_exe = |path: &Path, body: &str| {
        fs::write(path, body).unwrap();
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    };
    let hdiutil = format!(
        "#!/usr/bin/env bash\ncase \"$1\" in\n  attach) cp -R \"{}/.\" \"$4/\" 2>/dev/null || true ; printf '/dev/disk99\\tApple_HFS\\t%s\\n' \"$4\" ;;\n  detach) exit 0 ;;\nesac\n",
        dmg_mount.display()
    );
    for (name, body) in [
        ("hdiutil", hdiutil.as_str()),
        ("plutil", "#!/usr/bin/env bash\nexit 0\n"),
        (
            "codesign",
            "#!/usr/bin/env bash\nif [ \"$1\" = \"-dv\" ]; then\n  if [ \"${MOCK_SIGNED:-0}\" = \"1\" ]; then\n    echo 'Authority=Developer ID Application: Test' >&2\n    echo 'flags=0x10000(runtime)' >&2\n    echo 'Timestamp=Jan 1, 2026' >&2\n  else\n    echo 'Signature=adhoc' >&2\n  fi\n  exit 0\nfi\nif [ \"$1\" = \"--test-requirement==notarized\" ]; then\n  if [ \"${MOCK_CLI_NOTARIZED:-${MOCK_NOTARIZED:-0}}\" = \"1\" ]; then exit 0; fi\n  echo 'rejected (un-notarized executable)' >&2\n  exit 1\nfi\nexit 0\n",
        ),
        ("xcrun", "#!/usr/bin/env bash\nif [ \"${MOCK_NOTARIZED:-0}\" = \"1\" ]; then exit 0; fi\nexit 1\n"),
        (
            "spctl",
            "#!/usr/bin/env bash\nlast_arg=\"${!#}\"\ncase \"$last_arg\" in\n  *.app|*.dmg)\n    if [ \"${MOCK_NOTARIZED:-0}\" = \"1\" ]; then exit 0; fi\n    ;;\n  *)\n    echo 'rejected (the code is valid but does not seem to be an app)' >&2\n    exit 3\n    ;;\nesac\necho 'rejected (not notarized)' >&2\nexit 3\n",
        ),
        ("lipo", "#!/usr/bin/env bash\necho 'x86_64 arm64'\n"),
        ("arch", "#!/usr/bin/env bash\nshift\nexec \"$@\"\n"),
        ("uname", "#!/usr/bin/env bash\ncase \"${1:-}\" in\n  -m) echo 'arm64' ;;\n  *) echo \"${MOCK_UNAME_S:-Darwin}\" ;;\nesac\n"),
    ] {
        write_exe(&bin_dir.join(name), body);
    }

    let dmg = fx.path.join("app.dmg");
    fs::write(&dmg, b"dmg").unwrap();
    let path_env = format!("{}:/usr/bin:/bin", bin_dir.display());
    let run_verify = |extra_args: &[&str], envs: &[(&str, &str)]| {
        let mut cmd = Command::new("bash");
        cmd.arg(&verify_sh).args(["--arch", "universal"]).args(extra_args).arg(dmg.to_str().unwrap()).env("PATH", &path_env);
        for &(k, v) in envs {
            cmd.env(k, v);
        }
        let out = cmd.output().unwrap();
        (out.status.success(), String::from_utf8_lossy(&out.stderr).into_owned())
    };

    let (ok, err) = run_verify(&[], &[("MOCK_UNAME_S", "Linux")]);
    assert!(!ok && err.contains("must run on macOS"), "{err}");
    let (ok, err) = run_verify(&[], &[]);
    assert!(!ok && err.contains("CurveBook.app"), "{err}");

    fs::create_dir_all(dmg_mount.join("CurveBook.app/Contents/MacOS")).unwrap();
    fs::write(dmg_mount.join("CurveBook.app/Contents/Info.plist"), "<plist/>").unwrap();
    write_exe(&dmg_mount.join("CurveBook.app/Contents/MacOS/curvebook"), "#!/usr/bin/env bash\nexit 0\n");
    let (ok, err) = run_verify(&[], &[]);
    assert!(!ok && err.contains("curvebook-cli"), "{err}");

    write_exe(&dmg_mount.join("CurveBook.app/Contents/MacOS/curvebook-cli"), "#!/usr/bin/env bash\necho 'curvebook-cli 0.1.0'\n");
    assert!(!run_verify(&["--require-notarized"], &[("MOCK_SIGNED", "0")]).0);
    let (ok, err) = run_verify(&["--require-notarized"], &[("MOCK_SIGNED", "1"), ("MOCK_NOTARIZED", "1"), ("MOCK_CLI_NOTARIZED", "0")]);
    assert!(!ok && err.contains("un-notarized"), "expected notarization check failure for bundled curvebook-cli, got stderr={err}");
    let (ok, err) = run_verify(&["--require-notarized"], &[("MOCK_SIGNED", "1"), ("MOCK_NOTARIZED", "1"), ("MOCK_CLI_NOTARIZED", "1")]);
    assert!(ok, "stderr={err}");
}
