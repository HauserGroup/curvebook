//! End-to-end regression tests for `cargo xtask docs-site` (`R12.1`, `R12.4`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_SEQ: AtomicU64 = AtomicU64::new(0);
static SIM_REPO_TEMPLATE: OnceLock<Result<SimRepoTemplate, String>> = OnceLock::new();

struct SimRepoTemplate {
    files: Vec<(PathBuf, Vec<u8>)>,
}

impl SimRepoTemplate {
    fn shared() -> Result<&'static Self, String> {
        match SIM_REPO_TEMPLATE.get_or_init(Self::build_from_workspace) {
            Ok(tpl) => Ok(tpl),
            Err(err) => Err(err.clone()),
        }
    }

    fn build_from_workspace() -> Result<Self, String> {
        let root = workspace_root()?;
        let mut files = Vec::new();

        for file in ["AGENTS.md", "NOTICE", "README.md", "SECURITY.md", "Cargo.toml"] {
            let bytes = fs::read(root.join(file)).map_err(|e| format!("read {file}: {e}"))?;
            files.push((PathBuf::from(file), bytes));
        }

        // Collect only the docs subtrees that `docs-site` reads or resolves links against,
        // skipping archival/planning directories (`docs/{history,audit,planning,evidence,implementation}`)
        // and internal tracking tables (`ROADMAP.md`, `STATUS.md`, `gui-workflows.md`).
        let mut docs_paths = Vec::new();
        collect_docs_subset(&root, &mut docs_paths)?;
        let mut needed_assets = BTreeSet::new();
        for rel in &docs_paths {
            let bytes = fs::read(root.join(rel)).map_err(|e| format!("read {}: {e}", rel.display()))?;
            let rel_slash = rel.to_string_lossy().replace('\\', "/");
            if rel_slash.starts_with("docs/images/") {
                needed_assets.insert(rel_slash);
            }
            files.push((rel.clone(), bytes));
        }

        // Filter `ASSETS.md` and `ATTRIBUTION.md` down to `docs/images/*` and their transitive
        // parent-source chain so `assets::load_repo_asset_catalog` validates genuine provenance
        // without copying or SHA-256 hashing unrelated megabyte icons in `assets/`.
        let assets_md = fs::read_to_string(root.join("ASSETS.md")).map_err(|e| format!("read ASSETS.md: {e}"))?;
        let (filtered_assets_md, transitive_assets) = filter_assets_markdown(&assets_md, &needed_assets);
        files.push((PathBuf::from("ASSETS.md"), filtered_assets_md.into_bytes()));

        let attr_md = fs::read_to_string(root.join("ATTRIBUTION.md")).map_err(|e| format!("read ATTRIBUTION.md: {e}"))?;
        let filtered_attr_md = filter_attribution_table(&attr_md, &transitive_assets);
        files.push((PathBuf::from("ATTRIBUTION.md"), filtered_attr_md.into_bytes()));

        for asset_rel in transitive_assets {
            if asset_rel.starts_with("assets/") {
                let bytes = fs::read(root.join(&asset_rel)).map_err(|e| format!("read {asset_rel}: {e}"))?;
                files.push((PathBuf::from(asset_rel), bytes));
            }
        }

        Ok(Self { files })
    }

    fn materialize_into(&self, dst_root: &Path) -> Result<(), String> {
        for (rel, bytes) in &self.files {
            let dst = dst_root.join(rel);
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
            }
            fs::write(&dst, bytes).map_err(|e| format!("write {}: {e}", dst.display()))?;
        }
        Ok(())
    }
}

fn collect_docs_subset(root: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let docs_dir = root.join("docs");
    let mut entries: Vec<_> = fs::read_dir(&docs_dir).map_err(|e| format!("read_dir {}: {e}", docs_dir.display()))?.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let p = entry.path();
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if p.is_file() {
            if !matches!(name, "STATUS.md" | "gui-workflows.md") {
                out.push(Path::new("docs").join(name));
            }
        } else if p.is_dir() && matches!(name, "images" | "schemas" | "release" | "manual" | "reference") {
            collect_rel_files_recursive(root, &p, out)?;
        }
    }
    Ok(())
}

fn collect_rel_files_recursive(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let p = entry.path();
        if p.is_dir() {
            collect_rel_files_recursive(root, &p, out)?;
        } else if p.is_file() {
            let rel = p.strip_prefix(root).map_err(|e| e.to_string())?.to_path_buf();
            out.push(rel);
        }
    }
    Ok(())
}

fn parse_table_row_asset_path(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') || !trimmed.ends_with('|') {
        return None;
    }
    let cells: Vec<&str> = trimmed[1..trimmed.len() - 1].split('|').map(str::trim).collect();
    let [asset_cell, _, source_cell, ..] = cells.as_slice() else {
        return None;
    };
    let path = asset_cell.strip_prefix('`')?.strip_suffix('`')?.trim();
    if path.is_empty() {
        return None;
    }
    Some((path, source_cell))
}

fn extract_backticked_paths(source: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = source;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else {
            break;
        };
        let token = after[..end].trim();
        if token.contains('/') {
            out.push(token);
        }
        rest = &after[end + 1..];
    }
    out
}

fn filter_assets_markdown(assets_md: &str, seed: &BTreeSet<String>) -> (String, BTreeSet<String>) {
    let mut source_by_asset = BTreeMap::new();
    for line in assets_md.lines() {
        if let Some((path, source)) = parse_table_row_asset_path(line) {
            source_by_asset.insert(path.to_owned(), source.to_owned());
        }
    }
    let mut needed = seed.clone();
    let mut queue: Vec<String> = seed.iter().cloned().collect();
    while let Some(path) = queue.pop() {
        if let Some(src) = source_by_asset.get(&path) {
            for parent in extract_backticked_paths(src) {
                if needed.insert(parent.to_owned()) {
                    queue.push(parent.to_owned());
                }
            }
        }
    }
    let filtered = filter_attribution_table(assets_md, &needed);
    (filtered, needed)
}

fn filter_attribution_table(md: &str, keep: &BTreeSet<String>) -> String {
    let mut out = String::with_capacity(md.len());
    for line in md.lines() {
        if let Some((path, _)) = parse_table_row_asset_path(line)
            && !keep.contains(path)
        {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

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
    SimRepoTemplate::shared()?.materialize_into(sim_root)?;

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
    SimRepoTemplate::shared()?.materialize_into(sim_root)?;

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
