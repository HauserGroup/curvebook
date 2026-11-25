//! Regression gates for `R0.32`: verify that `xtask` decouples heavy GUI/engine/plot
//! crates behind the opt-in `docs-site` Cargo feature while keeping default light gates,
//! `cargo xtask layers` (22 packages), and example-delegated checks fast (`R0.32`).

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use super::common::{TempDirGuard, run_xtask as run_xtask_env, workspace_root};
use super::zip_helper::ZipWriter;

fn run_xtask(args: &[&str]) -> Result<std::process::Output, String> {
    run_xtask_env(args, &[])
}

fn run_cargo_tree(root: &Path, extra_args: &[&str]) -> Result<BTreeSet<String>, String> {
    let manifest = root.join("Cargo.toml");
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root).arg("tree").arg("--manifest-path").arg(&manifest).args([
        "--locked",
        "--offline",
        "-p",
        "xtask",
        "--edges",
        "normal",
        "--prefix",
        "none",
    ]);
    cmd.args(extra_args);
    let out = cmd.output().map_err(|e| format!("run cargo tree {extra_args:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo tree {extra_args:?} failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut pkgs = BTreeSet::new();
    for line in stdout.lines() {
        if let Some(name) = line.split_whitespace().next()
            && !name.is_empty()
        {
            pkgs.insert(name.to_owned());
        }
    }
    Ok(pkgs)
}

#[test]
fn r032_cargo_toml_defines_default_empty_and_decouples_heavy_crates() -> Result<(), String> {
    let root = workspace_root()?;
    let manifest = fs::read_to_string(root.join("xtask/Cargo.toml")).map_err(|e| format!("read xtask/Cargo.toml: {e}"))?;

    assert!(manifest.contains("default = []"), "xtask/Cargo.toml must default to empty features");
    for removed in ["curvebook-chart", "curvebook-pdf", "curvebook-svg", "curvebook-testkit", "image"] {
        assert!(!manifest.contains(removed), "xtask/Cargo.toml must not depend on `{removed}`");
    }
    let docs_deps = [
        "pulldown-cmark",
        "curvebook-doc",
        "curvebook-engine",
        "curvebook-fit",
        "curvebook-format",
        "curvebook-mcp",
        "curvebook-palette",
        "curvebook-stats",
        "curvebook-ui-egui",
    ];
    for dep in docs_deps {
        assert!(manifest.contains(&format!("\"dep:{dep}\"")), "xtask/Cargo.toml `docs-site` feature must activate `dep:{dep}`");
        let dep_line = manifest.lines().find(|l| l.trim_start().starts_with(&format!("{dep} ="))).ok_or_else(|| format!("missing {dep} line"))?;
        assert!(dep_line.contains("optional = true"), "{dep} must be optional in xtask/Cargo.toml: `{dep_line}`");
    }

    assert!(
        manifest.contains("name = \"r12_docs_site\"\nrequired-features = [\"docs-site\"]"),
        "r12_docs_site must declare `required-features = [\"docs-site\"]`"
    );

    let light_tree = run_cargo_tree(&root, &[])?;
    assert!(light_tree.len() <= 22, "default xtask graph must stay <= 22 unique crates, got {}: {light_tree:?}", light_tree.len());
    for forbidden in [
        "curvebook-ui-egui",
        "curvebook-engine",
        "curvebook-format",
        "curvebook-testkit",
        "curvebook-mcp",
        "curvebook-chart",
        "curvebook-pdf",
        "curvebook-svg",
        "curvebook-fit",
        "curvebook-stats",
        "curvebook-render",
        "curvebook-tools",
        "curvebook-text",
        "curvebook-transform",
        "egui",
        "eframe",
        "epaint",
        "fontdb",
        "vello_cpu",
        "harfrust",
        "skrifa",
        "image",
    ] {
        assert!(!light_tree.contains(forbidden), "default xtask graph must not link `{forbidden}`");
    }
    assert!(light_tree.contains("curvebook-platform"));

    let docs_tree = run_cargo_tree(&root, &["--features", "docs-site"])?;
    for required in ["pulldown-cmark", "curvebook-ui-egui", "curvebook-engine", "curvebook-mcp", "curvebook-fit", "curvebook-stats"] {
        assert!(docs_tree.contains(required), "`docs-site` graph must link `{required}`");
    }
    Ok(())
}

#[test]
fn r032_layers_package_count_unchanged_and_zero_violations() -> Result<(), String> {
    let out = run_xtask(&["layers"])?;
    assert!(out.status.success(), "xtask layers failed:\n{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("layers: all 22 workspace packages satisfy L0..L7 dependency rules and source hygiene"), "unexpected output: {stdout}");
    Ok(())
}

#[test]
fn r032_light_gates_and_feature_parity() -> Result<(), String> {
    for (args, expected_needle) in [
        (&["pending", "--count"][..], "pending:"),
        (&["citations"][..], "citations:"),
        (&["commands", "--check"][..], "commands:"),
        (&["schemas", "--check"][..], "schemas:"),
        (&["status"][..], "status: verified"),
    ] {
        let out = run_xtask(args)?;
        assert!(
            out.status.success(),
            "xtask {args:?} failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains(expected_needle), "expected `{expected_needle}` in stdout for {args:?}, got: {stdout}");
    }

    #[cfg(not(feature = "docs-site"))]
    {
        let out = run_xtask(&["docs-site", "--check"])?;
        assert!(!out.status.success(), "light binary must fail closed on `docs-site --check`");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("docs-site"), "expected `docs-site` error hint, got: {stderr}");
    }

    #[cfg(feature = "docs-site")]
    {
        let out = run_xtask(&["docs-site", "--check"])?;
        assert!(out.status.success(), "xtask docs-site --check failed:\n{}", String::from_utf8_lossy(&out.stderr));
    }

    Ok(())
}

#[test]
fn r032_release_scan_fixture_parity_across_feature_modes() -> Result<(), String> {
    let root = workspace_root()?;
    let fx = TempDirGuard::new("parity")?;

    // Clean container fixture exercises brands + cleanroom + assets in both light and full-gates binaries.
    let clean_dir = fx.path.join("clean");
    fs::create_dir_all(&clean_dir).map_err(|e| e.to_string())?;
    let notice = fs::read(root.join("NOTICE")).map_err(|e| e.to_string())?;
    let mut clean_zw = ZipWriter::new();
    clean_zw
        .add_file("curvebook-demo-0.1.0/index.html", b"<!doctype html><title>CurveBook</title><p>Clean fixture.</p>".to_vec())
        .map_err(|e| e.to_string())?;
    clean_zw.add_file("curvebook-demo-0.1.0/NOTICE", notice).map_err(|e| e.to_string())?;
    fs::write(clean_dir.join("curvebook-demo-0.1.0.zip"), clean_zw.finish().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let report_path = fx.path.join("report.json");
    let clean_out =
        run_xtask(&["release-scan", "--no-checksums", "--report", report_path.to_str().ok_or("bad path")?, clean_dir.to_str().ok_or("bad path")?])?;
    assert!(
        clean_out.status.success(),
        "release-scan clean fixture failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&clean_out.stdout),
        String::from_utf8_lossy(&clean_out.stderr)
    );
    let report: serde_json::Value = serde_json::from_str(&fs::read_to_string(&report_path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    assert_eq!(report["ok"], serde_json::json!(true));
    assert_eq!(report["asset_count"], serde_json::json!(1));
    assert_eq!(report["total_unpacked_files"], serde_json::json!(2));

    // Planted violation fixture must fail closed with identical container attribution in both modes.
    let dirty_dir = fx.path.join("dirty");
    fs::create_dir_all(&dirty_dir).map_err(|e| e.to_string())?;
    let leak_brand = ["Gr", "aph", "Pad"].concat();
    let mut dirty_zw = ZipWriter::new();
    dirty_zw.add_file("curvebook-demo/README.md", format!("Imported from {leak_brand}\n").into_bytes()).map_err(|e| e.to_string())?;
    fs::write(dirty_dir.join("curvebook-demo.zip"), dirty_zw.finish().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let dirty_out = run_xtask(&["release-scan", "--no-checksums", dirty_dir.to_str().ok_or("bad path")?])?;
    assert!(!dirty_out.status.success(), "planted brand violation must fail closed");
    let stderr = String::from_utf8_lossy(&dirty_out.stderr);
    assert!(stderr.contains("curvebook-demo.zip!curvebook-demo/README.md"), "{stderr}");
    Ok(())
}
