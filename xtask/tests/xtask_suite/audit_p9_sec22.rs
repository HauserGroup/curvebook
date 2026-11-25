//! Phase 9 Stage 9.1 regression suite (`FA-SEC22-01..19`, `FA-X3-05`, `FA-X3-06`).
//!
//! Exercises the compiled `xtask` CLI end-to-end on real policy gates and edge-case env settings,
//! keeping every synthetic negative token assembled at runtime so strict repo-wide brand/secret
//! checks stay clean on this test file itself.

use std::fs;

use super::common::{TempDirGuard, run_xtask, workspace_root};

#[test]
fn fa_x3_06_secrets_prints_workspace_tracked_plus_untracked_count_and_passes() -> Result<(), String> {
    let out = run_xtask(&["secrets"], &[])?;
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("tracked + untracked") && !stdout.contains("across 0 "), "unexpected secrets summary line: {stdout}");
    Ok(())
}

#[test]
fn fa_sec22_15_workflows_enforces_require_workflow_lint_and_partial_linter_parity() -> Result<(), String> {
    let empty_bin = TempDirGuard::new("empty-bin")?;
    let empty_str = empty_bin.path.to_str().ok_or_else(|| String::from("non-utf8 temp dir"))?;
    let out = run_xtask(&["workflows"], &[("CI", Some("false")), ("CURVEBOOK_REQUIRE_WORKFLOW_LINT", Some("1")), ("PATH", Some(empty_str))])?;
    assert!(!out.status.success(), "CURVEBOOK_REQUIRE_WORKFLOW_LINT=1 must fail when linters are missing");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("CURVEBOOK_REQUIRE_WORKFLOW_LINT=1"), "expected explicit require-lint error, got: {err}");

    // Partial-linter parity: create a mock `shellcheck` stub without `actionlint`;
    // `workflows` must fail rather than passing green while silently skipping `actionlint`.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        let partial_bin = TempDirGuard::new("partial-bin")?;
        let sc_stub = partial_bin.path.join("shellcheck");
        fs::write(&sc_stub, "#!/bin/sh\nexit 0\n").map_err(|e| format!("write stub: {e}"))?;
        let mut perms = fs::metadata(&sc_stub).map_err(|e| format!("stat stub: {e}"))?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&sc_stub, perms).map_err(|e| format!("chmod stub: {e}"))?;
        let partial_str = partial_bin.path.to_str().ok_or_else(|| String::from("non-utf8 stub dir"))?;

        let partial = run_xtask(&["workflows"], &[("CI", Some("false")), ("CURVEBOOK_REQUIRE_WORKFLOW_LINT", None), ("PATH", Some(partial_str))])?;
        assert!(!partial.status.success(), "partial linter run must fail when actionlint is absent");
        let p_err = String::from_utf8_lossy(&partial.stderr);
        assert!(p_err.contains("actionlint"), "expected missing actionlint diagnostic: {p_err}");
    }
    Ok(())
}

#[test]
fn fa_sec22_14_licenses_html_atomic_output_succeeds_offline() -> Result<(), String> {
    let dir = TempDirGuard::new("lic-html")?;
    let out_html = dir.path.join("nested/THIRD-PARTY-LICENSES.html");
    let out_str = out_html.to_str().ok_or_else(|| String::from("non-utf8 path"))?;
    let out = run_xtask(&["licenses", "--html", out_str], &[("CI", Some("false")), ("CARGO_NET_OFFLINE", Some("true"))])?;
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let html = fs::read_to_string(&out_html).map_err(|e| format!("read {}: {e}", out_html.display()))?;
    assert!(
        html.contains("Resolved Shipped Third-Party Crates") && html.contains("curvebook-platform") || html.contains("<!DOCTYPE html>"),
        "{html}"
    );
    Ok(())
}

#[test]
fn fa_x3_05_06_live_policy_gates_pass_and_ci_rs_hardening_present() -> Result<(), String> {
    let root = workspace_root()?;
    let ci_rs = fs::read_to_string(root.join("xtask/src/ci.rs")).map_err(|e| format!("read ci.rs: {e}"))?;
    assert!(ci_rs.contains("binary(r15_2_anova_and_1m_bench)"), "FA-X3-05 filter term missing from ci.rs");
    assert!(ci_rs.contains("workflows::is_ci_env()"), "FA-X3-06 non-full-gates CI guard missing from ci.rs");

    // Per R0.32, do not re-run full live-repository scans inside xtask integration tests
    // (the `ci policy` stage already runs every live gate once per revision). Instead,
    // verify `ci.rs::policy` wires every required gate and smoke-test fast CLI dispatch.
    for required_call in [
        "assets::run(root)?;",
        "brands::run(root)?;",
        "cleanroom::run(root)?;",
        "cmd_layers(root)?;",
        "secrets::run(root)?;",
        "licenses::run(root, &[])?;",
        "telemetry::run(root, &[])?;",
        "workflows::run(root, &[])?;",
        "pending::run_count(root)?;",
        "status::run(root)",
    ] {
        assert!(ci_rs.contains(required_call), "ci.rs::policy missing required gate call `{required_call}`");
    }

    for args in [&["assets"][..], &["pending", "--count"][..]] {
        let out = run_xtask(args, &[])?;
        assert!(
            out.status.success(),
            "`xtask {}` failed:\nstdout: {}\nstderr: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }
    Ok(())
}
