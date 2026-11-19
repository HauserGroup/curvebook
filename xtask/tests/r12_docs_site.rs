//! End-to-end regression tests for `cargo xtask docs-site` (`R12.1`, `R12.4`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_SEQ: AtomicU64 = AtomicU64::new(0);

fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(Path::to_path_buf).ok_or_else(|| String::from("CARGO_MANIFEST_DIR has no parent"))
}

struct WorkspaceScratchDir {
    path: PathBuf,
}

impl WorkspaceScratchDir {
    fn new(root: &Path, label: &str) -> Result<Self, String> {
        let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = root.join("target").join("r12-docs-site-tests").join(format!("{label}-{}-{seq}", std::process::id()));
        if dir.exists() {
            let _ = fs::remove_dir_all(&dir);
        }
        fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        Ok(Self { path: dir })
    }
}

impl Drop for WorkspaceScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn run_xtask(cwd: &Path, args: &[&str]) -> Result<Output, String> {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .current_dir(cwd)
        .env("CURVEBOOK_DOCS_SITE_ROOT", cwd)
        .args(args)
        .output()
        .map_err(|e| format!("run xtask {args:?}: {e}"))
}

fn collect_html_map(base: &Path, dir: &Path, out: &mut BTreeMap<String, String>) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let p = entry.path();
        if p.is_dir() {
            collect_html_map(base, &p, out)?;
        } else if p.extension().and_then(|e| e.to_str()) == Some("html") {
            let rel = p.strip_prefix(base).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
            let text = fs::read_to_string(&p).map_err(|e| format!("read {}: {e}", p.display()))?;
            out.insert(rel, text);
        }
    }
    Ok(())
}

#[test]
fn docs_site_check_passes_and_output_is_deterministic() -> Result<(), String> {
    let root = workspace_root()?;
    let out_a = WorkspaceScratchDir::new(&root, "det-a")?;
    let out_b = WorkspaceScratchDir::new(&root, "det-b")?;

    let a_str = out_a.path.to_str().ok_or("non-utf8 path")?;
    let b_str = out_b.path.to_str().ok_or("non-utf8 path")?;

    let res_a = run_xtask(&root, &["docs-site", "--check", "--output", a_str])?;
    assert!(
        res_a.status.success(),
        "docs-site --check failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&res_a.stdout),
        String::from_utf8_lossy(&res_a.stderr)
    );

    let res_b = run_xtask(&root, &["docs-site", "--output", b_str])?;
    assert!(res_b.status.success(), "second build failed: {}", String::from_utf8_lossy(&res_b.stderr));

    let mut map_a = BTreeMap::new();
    let mut map_b = BTreeMap::new();
    collect_html_map(&out_a.path, &out_a.path, &mut map_a)?;
    collect_html_map(&out_b.path, &out_b.path, &mut map_b)?;

    assert_eq!(map_a.keys().collect::<Vec<_>>(), map_b.keys().collect::<Vec<_>>());
    for (rel, html_a) in &map_a {
        let html_b = map_b.get(rel).ok_or_else(|| format!("missing {rel}"))?;
        assert_eq!(html_a, html_b, "non-deterministic HTML output for `{rel}`");
    }

    // Spot-check critical Beta site map sections, registry content, direct GitHub contributor links, and CSP.
    assert!(map_a.contains_key("index.html"));
    assert!(map_a.contains_key("az-index.html"));
    assert!(map_a.contains_key("analyses.html"));
    assert!(map_a.contains_key("automation/mcp.html"));
    assert!(map_a.contains_key("automation/cli.html"));
    assert!(map_a.contains_key("automation/control-and-batch.html"));
    assert!(map_a.contains_key("registry/commands.html"));
    assert!(map_a.contains_key("registry/mcp-tools.html"));
    assert!(map_a.contains_key("registry/cli.html"));
    assert!(map_a.contains_key("registry/equations.html"));
    assert!(map_a.contains_key("registry/tables.html"));
    assert!(map_a.contains_key("registry/graphs.html"));
    assert!(map_a.contains_key("registry/formats.html"));

    assert!(
        !map_a.keys().any(|k| k.starts_with("contribute/")),
        "contributor guides must link to GitHub main rather than rendering contribute/*.html"
    );
    let contrib_index = map_a.get("section-contribute.html").ok_or("missing section-contribute.html")?;
    for repo_path in [
        "AGENTS.md",
        "docs/architecture.md",
        "docs/development.md",
        "docs/extending.md",
        "docs/pitfalls.md",
        "docs/pitfalls-engineering.md",
        "docs/command-ids.md",
        "docs/file-formats.md",
        "docs/licences.md",
        "docs/localization.md",
        "docs/ci.md",
        "docs/release/releasing.md",
    ] {
        let expected_url = format!("https://github.com/HauserGroup/curvebook/blob/main/{repo_path}");
        assert!(contrib_index.contains(&expected_url), "section-contribute.html missing GitHub main link `{expected_url}`");
    }

    // Contributor docs linked from automation pages must rewrite directly to GitHub blob URLs.
    let batch_html = map_a.get("automation/control-and-batch.html").ok_or("missing automation/control-and-batch.html")?;
    assert!(
        batch_html.contains("https://github.com/HauserGroup/curvebook/blob/main/docs/command-ids.md"),
        "expected GitHub main link for docs/command-ids.md in automation/control-and-batch.html"
    );
    assert!(!map_a.contains_key("ROADMAP.html"));
    assert!(!map_a.contains_key("STATUS.html"));

    Ok(())
}

#[test]
fn docs_site_check_passes_without_r12_3_and_with_partial_analyses_md() -> Result<(), String> {
    let root = workspace_root()?;
    let sandbox = WorkspaceScratchDir::new(&root, "partial-r12-3")?;
    let sim_root = &sandbox.path;

    for file in ["AGENTS.md", "ASSETS.md", "ATTRIBUTION.md", "NOTICE"] {
        fs::copy(root.join(file), sim_root.join(file)).map_err(|e| format!("copy {file}: {e}"))?;
    }
    copy_dir_recursive(&root.join("assets"), &sim_root.join("assets"))?;
    copy_dir_recursive(&root.join("docs"), &sim_root.join("docs"))?;

    // Explicitly remove R12.3 files if present to exercise absent-methods behavior.
    for omitted in ["analyses.md", "fitting.md"] {
        let p = sim_root.join("docs/reference").join(omitted);
        if p.exists() {
            fs::remove_file(&p).map_err(|e| format!("remove {}: {e}", p.display()))?;
        }
    }

    let absent_out = WorkspaceScratchDir::new(&root, "absent-r12-3-out")?;
    let absent_res = run_xtask(sim_root, &["docs-site", "--check", "--output", absent_out.path.to_str().ok_or("bad path")?])?;
    assert!(
        absent_res.status.success(),
        "docs-site --check without R12.3 failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&absent_res.stdout),
        String::from_utf8_lossy(&absent_res.stderr)
    );

    let absent_az = fs::read_to_string(absent_out.path.join("az-index.html")).map_err(|e| e.to_string())?;
    assert!(absent_az.contains("registry/tables.html#analysis-kind-two-group-comparison"));
    assert!(!absent_az.contains("analyses.html#analysis-two-group-comparison"));

    // Now test a partial `docs/reference/analyses.md` covering only one analysis anchor.
    fs::create_dir_all(sim_root.join("docs/reference")).map_err(|e| e.to_string())?;
    fs::write(
        sim_root.join("docs/reference/analyses.md"),
        "# Statistical Analyses\n\n## Two-Group Comparison {#analysis-two-group-comparison}\n\nParametric and non-parametric two-group comparisons.\n",
    )
    .map_err(|e| e.to_string())?;

    let partial_out = WorkspaceScratchDir::new(&root, "partial-r12-3-out")?;
    let partial_res = run_xtask(sim_root, &["docs-site", "--check", "--output", partial_out.path.to_str().ok_or("bad path")?])?;
    assert!(
        partial_res.status.success(),
        "docs-site --check with partial analyses.md failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&partial_res.stdout),
        String::from_utf8_lossy(&partial_res.stderr)
    );

    let partial_az = fs::read_to_string(partial_out.path.join("az-index.html")).map_err(|e| e.to_string())?;
    assert!(partial_az.contains("analyses.html#analysis-two-group-comparison"));
    assert!(partial_az.contains("registry/tables.html#analysis-kind-survival"));
    let partial_tables = fs::read_to_string(partial_out.path.join("registry/tables.html")).map_err(|e| e.to_string())?;
    assert!(partial_tables.contains("../analyses.html#analysis-two-group-comparison"));
    assert!(partial_tables.contains("#analysis-kind-survival"));

    Ok(())
}

#[test]
fn docs_site_ingests_populated_reference_markdown_and_verifies_release_snapshot() -> Result<(), String> {
    let root = workspace_root()?;
    let sandbox = WorkspaceScratchDir::new(&root, "snapshot-sim")?;
    let sim_root = &sandbox.path;

    // Copy minimal workspace metadata/assets/docs into isolated sandbox git repo.
    for file in ["AGENTS.md", "ASSETS.md", "ATTRIBUTION.md", "NOTICE"] {
        fs::copy(root.join(file), sim_root.join(file)).map_err(|e| format!("copy {file}: {e}"))?;
    }
    copy_dir_recursive(&root.join("assets"), &sim_root.join("assets"))?;
    copy_dir_recursive(&root.join("docs"), &sim_root.join("docs"))?;

    // Populate all 13 `docs/reference/*.md` files simulating post-merge `@r12-methods-ref`.
    let ref_dir = sim_root.join("docs/reference");
    fs::create_dir_all(&ref_dir).map_err(|e| e.to_string())?;
    for (name, body) in [
        ("get-started", "# Get Started\n\nSee [MCP Setup](../mcp-for-beginners.md).\n"),
        ("desktop-gui", "# Desktop GUI Overview\n\nSee [Commands](../commands.md).\n"),
        ("concepts", "# Concepts\n\nSee [Tables](tables.md).\n"),
        ("tables", "# Data Tables\n\nSee [Formats](files.md).\n"),
        ("fitting", "# Curve Fitting\n\nSee [Equations Registry](registry/equations.html#eq-michaelis_menten).\n"),
        ("graphs", "# Graphs & Page Layouts\n\n![Workspace](../images/workspace-split-fit.png)\n"),
        ("qol", "# Calculators & Quality-of-Life\n\nSee [Graphs](graphs.md).\n"),
        ("files", "# Files, Import & Export\n\nSee [Batch Schema](../schemas/batch-v1.schema.json).\n"),
        ("settings", "# Settings & Languages\n\nSee [Preferences Schema](../schemas/preferences.schema.json).\n"),
        ("privacy-security", "# Privacy & Security\n\nSee [Security](../../SECURITY.md).\n"),
        ("troubleshooting", "# Troubleshooting\n\nSee [Privacy & Security](privacy-security.md).\n"),
        ("glossary", "# Glossary\n\n## Reactive DAG {#term-reactive-dag}\n\nRecomputes sheets.\n"),
    ] {
        fs::write(ref_dir.join(format!("{name}.md")), body).map_err(|e| e.to_string())?;
    }
    let mut analyses_md = String::from("# Analyses\n\n");
    for slug in [
        "transform",
        "descriptive-stats",
        "outlier-detection",
        "two-group-comparison",
        "bland-altman",
        "multiple-row-t-tests",
        "passing-bablok",
        "one-way-anova",
        "factorial-anova",
        "nested-comparison",
        "contingency",
        "survival",
        "roc-curve",
        "generalized-linear-model",
        "multivariate",
        "linear-regression",
        "nonlinear-regression",
        "p-value-stack",
        "power-analysis",
    ] {
        analyses_md.push_str(&format!("## {slug} {{#analysis-{slug}}}\n\nReference details.\n\n"));
    }
    fs::write(ref_dir.join("analyses.md"), analyses_md).map_err(|e| e.to_string())?;

    // Initialize clean git history + release tag so `--release` and manifest tag check execute end-to-end.
    for git_args in [
        &["init"][..],
        &["config", "user.email", "test@curvebook.invalid"][..],
        &["config", "user.name", "CurveBook Test"][..],
        &["add", "."][..],
        &["commit", "-m", "initial snapshot base"][..],
        &["tag", "v0.1.0-alpha.2"][..],
    ] {
        let out = Command::new("git").current_dir(sim_root).args(git_args).output().map_err(|e| e.to_string())?;
        assert!(out.status.success(), "git {git_args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    }

    let head_out = Command::new("git").current_dir(sim_root).args(["rev-parse", "HEAD"]).output().map_err(|e| e.to_string())?;
    let head_sha = String::from_utf8_lossy(&head_out.stdout).trim().to_owned();

    let rel_res = run_xtask(sim_root, &["docs-site", "--release", "0.1.0-alpha.2", "--check"])?;
    assert!(
        rel_res.status.success(),
        "docs-site --release --check failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&rel_res.stdout),
        String::from_utf8_lossy(&rel_res.stderr)
    );

    let snap_manifest = sim_root.join("docs/manual/v0.1/manifest.json");
    let manifest_text = fs::read_to_string(&snap_manifest).map_err(|e| e.to_string())?;
    let manifest_json: serde_json::Value = serde_json::from_str(&manifest_text).map_err(|e| e.to_string())?;
    assert_eq!(manifest_json["commit"], serde_json::json!(head_sha));
    assert_eq!(manifest_json["minor_dir"], serde_json::json!("v0.1"));

    let redirect_html = fs::read_to_string(sim_root.join("docs/manual/index.html")).map_err(|e| e.to_string())?;
    assert!(redirect_html.contains("<meta http-equiv=\"refresh\" content=\"0; url=v0.1/index.html\">"));
    assert!(!redirect_html.contains("<script"));

    // Corrupting the manifest commit must make `--check` fail because it no longer matches tag `v0.1.0-alpha.2`.
    let bad_manifest = manifest_text.replace(&head_sha, "0000000000000000000000000000000000000000");
    fs::write(&snap_manifest, bad_manifest).map_err(|e| e.to_string())?;
    let preview_dir = sim_root.join("target/docs-site-check");
    let fail_res = run_xtask(sim_root, &["docs-site", "--check", "--output", preview_dir.to_str().ok_or("bad path")?])?;
    assert!(!fail_res.status.success(), "mismatched tag commit in manifest must fail --check");
    assert!(String::from_utf8_lossy(&fail_res.stderr).contains("does not match release tag"));

    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("create {}: {e}", dst.display()))?;
    for entry in fs::read_dir(src).map_err(|e| format!("read_dir {}: {e}", src.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        let out_p = dst.join(entry.file_name());
        if p.is_dir() {
            copy_dir_recursive(&p, &out_p)?;
        } else if p.is_file() {
            fs::copy(&p, &out_p).map_err(|e| format!("copy {} -> {}: {e}", p.display(), out_p.display()))?;
        }
    }
    Ok(())
}
