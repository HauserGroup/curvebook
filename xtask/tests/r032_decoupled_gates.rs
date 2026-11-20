//! Regression gates for `R0.32`: verify that `xtask` decouples heavy GUI/engine/plot
//! crates behind the default-on `full-gates` Cargo feature while preserving light gates,
//! `cargo xtask layers` (22 packages), and default-feature CI parity.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use curvebook_format::ZipWriter;

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(label: &str) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!("cb-r032-test-{label}-{}-{nanos}-{seq}", std::process::id()));
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

fn run_xtask(args: &[&str]) -> Result<Output, String> {
    let root = workspace_root()?;
    Command::new(env!("CARGO_BIN_EXE_xtask")).current_dir(root).args(args).output().map_err(|e| format!("run xtask {args:?}: {e}"))
}

fn run_cargo_tree(root: &Path, extra_args: &[&str]) -> Result<BTreeSet<String>, String> {
    let manifest = root.join("Cargo.toml");
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root).arg("tree").arg("--manifest-path").arg(&manifest).args(["--locked", "--offline", "-p", "xtask", "--prefix", "none"]);
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
fn r032_cargo_toml_defines_default_full_gates_and_decouples_heavy_crates() -> Result<(), String> {
    let root = workspace_root()?;
    let manifest = fs::read_to_string(root.join("xtask/Cargo.toml")).map_err(|e| format!("read xtask/Cargo.toml: {e}"))?;

    assert!(manifest.contains("default = [\"full-gates\"]"), "xtask/Cargo.toml must enable `full-gates` by default");
    let heavy_deps = [
        "curvebook-chart",
        "curvebook-doc",
        "curvebook-engine",
        "curvebook-fit",
        "curvebook-mcp",
        "curvebook-palette",
        "curvebook-pdf",
        "curvebook-stats",
        "curvebook-svg",
        "curvebook-ui-egui",
    ];
    for dep in heavy_deps {
        assert!(manifest.contains(&format!("\"dep:{dep}\"")), "xtask/Cargo.toml `full-gates` feature must activate `dep:{dep}`");
        let dep_line = manifest.lines().find(|l| l.trim_start().starts_with(&format!("{dep} ="))).ok_or_else(|| format!("missing {dep} line"))?;
        assert!(dep_line.contains("optional = true"), "{dep} must be optional in xtask/Cargo.toml: `{dep_line}`");
    }

    for test_target in ["r021_strict_alpha_branding", "r12_docs_site"] {
        assert!(
            manifest.contains(&format!("name = \"{test_target}\"\nrequired-features = [\"full-gates\"]")),
            "{test_target} must declare `required-features = [\"full-gates\"]`"
        );
    }

    let light_tree = run_cargo_tree(&root, &["--no-default-features"])?;
    for forbidden in [
        "curvebook-ui-egui",
        "curvebook-engine",
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
    ] {
        assert!(!light_tree.contains(forbidden), "`--no-default-features` graph must not link `{forbidden}`");
    }

    let full_tree = run_cargo_tree(&root, &[])?;
    for required in ["curvebook-ui-egui", "curvebook-engine", "curvebook-mcp", "curvebook-fit", "curvebook-stats", "curvebook-chart"] {
        assert!(full_tree.contains(required), "default `full-gates` graph must link `{required}`");
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

    #[cfg(not(feature = "full-gates"))]
    {
        for gated in [&["commands", "--check"][..], &["docs-site", "--check"][..]] {
            let out = run_xtask(gated)?;
            assert!(!out.status.success(), "light binary must fail closed on `{gated:?}`");
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(stderr.contains("full-gates"), "expected `full-gates` error hint for {gated:?}, got: {stderr}");
        }
    }

    #[cfg(feature = "full-gates")]
    {
        let out = run_xtask(&["commands", "--check"])?;
        assert!(out.status.success(), "xtask commands --check failed:\n{}", String::from_utf8_lossy(&out.stderr));
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("commands:"), "unexpected output: {stdout}");
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
