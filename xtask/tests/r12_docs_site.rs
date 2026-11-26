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
        // skipping archival/planning directories (`docs/{history,audit,planning,implementation}`)
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
        } else if p.is_dir() && matches!(name, "images" | "schemas" | "release" | "reference" | "evidence") {
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

fn parse_table_row_asset_path(line: &str) -> Option<(&str, &str, &str)> {
    let inner = line.trim().strip_prefix('|')?.strip_suffix('|')?;
    let cells: Vec<&str> = inner.split('|').map(str::trim).collect();
    let [asset_cell, _, source_cell, licence_cell, ..] = cells.as_slice() else {
        return None;
    };
    let path = asset_cell.strip_prefix('`')?.strip_suffix('`')?.trim();
    if path.is_empty() {
        return None;
    }
    Some((path, source_cell, licence_cell))
}

fn extract_backticked_paths(source: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = source;
    while let Some((_, after)) = rest.split_once('`') {
        let Some((token_raw, tail)) = after.split_once('`') else {
            break;
        };
        let token = token_raw.trim();
        if token.contains('/') {
            out.push(token);
        }
        rest = tail;
    }
    out
}

fn filter_assets_markdown(assets_md: &str, seed: &BTreeSet<String>) -> (String, BTreeSet<String>) {
    let mut refs_by_asset: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in assets_md.lines() {
        if let Some((path, source, licence)) = parse_table_row_asset_path(line) {
            let mut refs = Vec::new();
            refs.extend(extract_backticked_paths(source).into_iter().map(str::to_owned));
            refs.extend(extract_backticked_paths(licence).into_iter().map(str::to_owned));
            refs_by_asset.insert(path.to_owned(), refs);
        }
    }
    let mut needed = seed.clone();
    let mut queue: Vec<String> = seed.iter().cloned().collect();
    while let Some(path) = queue.pop() {
        if let Some(refs) = refs_by_asset.get(&path) {
            for parent in refs {
                if needed.insert(parent.clone()) {
                    queue.push(parent.clone());
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
        if let Some((path, _, _)) = parse_table_row_asset_path(line)
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

fn run_xtask_ok(cwd: &Path, args: &[&str]) -> Result<Output, String> {
    let out = run_xtask(cwd, args)?;
    assert!(
        out.status.success(),
        "xtask {args:?} failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(out)
}

fn run_xtask_fail(cwd: &Path, args: &[&str], needle: &str) -> Result<(), String> {
    let out = run_xtask(cwd, args)?;
    assert!(!out.status.success(), "expected xtask {args:?} to fail");
    if !needle.is_empty() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(needle), "expected `{needle}` in stderr:\n{stderr}");
    }
    Ok(())
}

fn collect_html_map(base: &Path, out: &mut BTreeMap<String, String>) -> Result<(), String> {
    let mut files = Vec::new();
    collect_rel_files_recursive(base, base, &mut files)?;
    for rel in files {
        if rel.extension().and_then(|e| e.to_str()) == Some("html") {
            let text = fs::read_to_string(base.join(&rel)).map_err(|e| format!("read {}: {e}", rel.display()))?;
            out.insert(rel.to_string_lossy().replace('\\', "/"), text);
        }
    }
    Ok(())
}

#[test]
fn docs_site_check_passes_and_output_is_deterministic() -> Result<(), String> {
    let root = workspace_root()?;
    let out_a = WorkspaceScratchDir::new(&root, "det-a")?;
    let out_b = WorkspaceScratchDir::new(&root, "det-b")?;

    run_xtask_ok(&root, &["docs-site", "--check", "--output", out_a.path.to_str().ok_or("non-utf8 path")?])?;
    run_xtask_ok(&root, &["docs-site", "--output", out_b.path.to_str().ok_or("non-utf8 path")?])?;

    let (mut map_a, mut map_b) = (BTreeMap::new(), BTreeMap::new());
    collect_html_map(&out_a.path, &mut map_a)?;
    collect_html_map(&out_b.path, &mut map_b)?;

    assert_eq!(map_a.keys().collect::<Vec<_>>(), map_b.keys().collect::<Vec<_>>());
    for (rel, html_a) in &map_a {
        assert_eq!(html_a, map_b.get(rel).ok_or_else(|| format!("missing {rel}"))?, "non-deterministic HTML output for `{rel}`");
    }

    assert_beta_site_contents(&map_a)
}

fn assert_beta_site_contents(map_a: &BTreeMap<String, String>) -> Result<(), String> {
    for key in [
        "index.html",
        "reference.html",
        "support.html",
        "az-index.html",
        "analyses.html",
        "automation/mcp.html",
        "automation/cli.html",
        "automation/control-and-batch.html",
        "registry/commands.html",
        "registry/mcp-tools.html",
        "registry/cli.html",
        "registry/equations.html",
        "registry/tables.html",
        "registry/graphs.html",
        "registry/formats.html",
    ] {
        assert!(map_a.contains_key(key), "missing generated page `{key}`");
    }

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
        "docs/design-language.md",
        "docs/ci.md",
        "docs/release/releasing.md",
    ] {
        let expected_url = format!("https://github.com/HauserGroup/curvebook/blob/main/{repo_path}");
        assert!(contrib_index.contains(&expected_url), "section-contribute.html missing GitHub main link `{expected_url}`");
    }

    assert_nav_hubs_and_r12_14_fixes(map_a)?;

    // AUD-12-19: Math-containing headings in live docs must produce clean slug IDs and nested TOC text.
    let fitting_html = map_a.get("fitting.html").ok_or("missing fitting.html")?;
    assert!(fitting_html.contains("id=\"hougaard-skewness-g1\""), "fitting.html must render clean heading ID `hougaard-skewness-g1` (AUD-12-19)");
    assert!(
        fitting_html.contains("href=\"#hougaard-skewness-g1\">Hougaard Skewness (g1)</a>"),
        "fitting.html nested TOC must render clean plain math text for Hougaard Skewness (AUD-12-19)"
    );

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

fn assert_nav_hubs_and_r12_14_fixes(map_a: &BTreeMap<String, String>) -> Result<(), String> {
    // Verify grouped Manual Home sections, flat chrome, R12.13 navigation, and R12.14 fixes.
    let home_html = map_a.get("index.html").ok_or("missing index.html")?;
    for group in [
        "class=\"site-shell is-manual\"",
        "class=\"manual-layout\"",
        "class=\"manual-sidebar\"",
        "class=\"section-group\"",
        "Start Here",
        "Scientific &amp; Graphing Reference",
        "Files, Automation &amp; Settings",
        "Support &amp; Contribute",
        "aria-current=\"page\"",
        "Development build · main",
        "1 page",
        "href=\"concepts.html\"",
    ] {
        assert!(home_html.contains(group), "index.html missing `{group}`");
    }
    for forbidden in ["desktop-window", "title-bar", "window-status", "window-statusbar", "1 pages", "class=\"breadcrumbs\"", "class=\"pager\""] {
        assert!(!home_html.contains(forbidden), "index.html contains unexpected `{forbidden}`");
    }
    assert!(map_a.contains_key("section-concepts.html"), "backwards-compatible section-concepts.html must still be emitted");

    let reference_html = map_a.get("reference.html").ok_or("missing reference.html")?;
    for expected in [
        "Core concepts &amp; data",
        "Statistics &amp; curve fitting",
        "Graphs, files &amp; settings",
        "Terminology",
        "href=\"concepts.html\"",
        "href=\"analyses.html\"",
    ] {
        assert!(reference_html.contains(expected), "reference.html missing `{expected}`");
    }
    let support_html = map_a.get("support.html").ok_or("missing support.html")?;
    for expected in ["href=\"troubleshooting.html\"", "href=\"privacy-security.html\"", "href=\"section-contribute.html\""] {
        assert!(support_html.contains(expected), "support.html missing `{expected}`");
    }

    let get_started_sec = map_a.get("section-get-started.html").ok_or("missing section-get-started.html")?;
    assert!(get_started_sec.contains("Getting started</h1>") && get_started_sec.contains("href=\"get-started.html\">Install CurveBook</a>"));

    let az_html = map_a.get("az-index.html").ok_or("missing az-index.html")?;
    assert!(az_html.contains("class=\"az-letters\"") && !az_html.contains("class=\"toc\""), "az-index.html must not emit duplicate vertical TOC");

    let get_started_html = map_a.get("get-started.html").ok_or("missing get-started.html")?;
    let gs_h1 = get_started_html.find("<h1 ").ok_or("get-started.html missing h1")?;
    let gs_toc = get_started_html.find("class=\"toc\"").ok_or("get-started.html missing toc")?;
    assert!(gs_h1 < gs_toc, "TOC must appear below H1 on get-started.html");
    let (_, sidebar_slice) = get_started_html.split_once("class=\"manual-sidebar\"").ok_or("missing sidebar on get-started.html")?;
    let (sidebar_only, _) = sidebar_slice.split_once("</aside>").ok_or("unclosed sidebar on get-started.html")?;
    assert!(
        sidebar_only.contains("href=\"desktop-gui.html\"") && !sidebar_only.contains("href=\"registry/tables.html\""),
        "sidebar must expand only active section on get-started.html"
    );

    let tables_html = map_a.get("tables.html").ok_or("missing tables.html")?;
    assert!(tables_html.contains("<td><code>||</code>, <code>OR</code></td>"), "Disjunction table row must keep intact `||` code span");

    let glossary_html = map_a.get("glossary.html").ok_or("missing glossary.html")?;
    assert!(glossary_html.contains("<span class=\"math\">"), "glossary.html must render math spans");
    assert!(!glossary_html.contains("$\\text{AIC}_c$"), "glossary.html leaked raw TeX");

    let desktop_html = map_a.get("desktop-gui.html").ok_or("missing desktop-gui.html")?;
    assert!(desktop_html.contains("<figure class=\"doc-figure\">"), "desktop-gui.html should wrap standalone screenshots in <figure>");

    let equations_html = map_a.get("registry/equations.html").ok_or("missing registry/equations.html")?;
    assert!(equations_html.contains("<span class=\"math-frac\">"), "registry/equations.html must render static math fractions");
    assert!(equations_html.contains("<code>Y ="), "registry/equations.html must retain copyable TeX inside <code>");
    assert!(
        equations_html.contains("<span class=\"math-text\">Bottom</span> + <span class=\"math-frac\">"),
        "missing left-side space before `+` in math"
    );
    assert!(equations_html.contains("1e-30"), "registry/equations.html missing clean scientific bound `1e-30`");
    assert!(!equations_html.contains("0.000000000000000000000000000001"), "registry/equations.html leaked unformatted decimal bound");

    let graphs_html = map_a.get("registry/graphs.html").ok_or("missing registry/graphs.html")?;
    assert!(graphs_html.contains("id=\"graph-layout-features\""), "registry/graphs.html missing R13 features section");
    assert!(graphs_html.contains("EqualizeScaleMode") || graphs_html.contains("layout.equalizeScaling"));
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
    run_xtask_ok(sim_root, &["docs-site", "--sitemap"])?;

    let absent_out = WorkspaceScratchDir::new(&root, "absent-r12-3-out")?;
    run_xtask_ok(sim_root, &["docs-site", "--check", "--output", absent_out.path.to_str().ok_or("bad path")?])?;

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
    run_xtask_ok(sim_root, &["docs-site", "--sitemap"])?;

    let partial_out = WorkspaceScratchDir::new(&root, "partial-r12-3-out")?;
    run_xtask_ok(sim_root, &["docs-site", "--check", "--output", partial_out.path.to_str().ok_or("bad path")?])?;

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
        (
            "get-started",
            "# Get Started\n\nSee [MCP Setup](../mcp-for-beginners.md).\n\n## Linux {#install-linux}\n\nLinux `.AppImage` install notes.\n\n## Verifying Downloads {#verifying-downloads}\n\nChecksum and attestation verification.\n",
        ),
        ("desktop-gui", "# Desktop GUI Overview\n\nSee [Commands](../commands.md).\n"),
        ("concepts", "# Concepts\n\nSee [Tables](tables.md).\n"),
        ("tables", "# Data Tables\n\nSee [Formats](files.md).\n\n## Table Archetypes {#table-archetypes}\n\nXY, Column, and Grouped tables.\n"),
        ("fitting", "# Curve Fitting\n\nSee [Equations Registry](registry/equations.html#eq-michaelis_menten).\n"),
        (
            "graphs",
            "# Graphs & Page Layouts\n\n![Workspace](../images/workspace-split-fit.png)\n\n## Graph Families {#graph-families}\n\nPlot types.\n\n## Page Layouts {#page-layouts}\n\nMulti-panel figures.\n",
        ),
        ("qol", "# Calculators & Quality-of-Life\n\nSee [Graphs](graphs.md).\n"),
        (
            "files",
            "# Files, Import & Export\n\nSee [Batch Schema](../schemas/batch-v1.schema.json).\n\n## Native Formats {#native-formats}\n\nProject archives.\n",
        ),
        ("settings", "# Settings & Languages\n\nSee [Preferences Schema](../schemas/preferences.schema.json).\n"),
        ("privacy-security", "# Privacy & Security\n\nSee [Security](../../SECURITY.md).\n"),
        (
            "troubleshooting",
            "# Troubleshooting\n\nSee [Privacy & Security](privacy-security.md).\n\n## Known Issues {#known-issues}\n\nBeta known issues.\n\n### MCP Client Cannot Connect {#mcp-connection}\n\nVerify absolute command and `--workdir` paths.\n\n### Linux AppImage FUSE {#linux-fuse}\n\nRun with `--appimage-extract-and-run` if FUSE is unavailable.\n",
        ),
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
        "simple-logistic-regression",
        "fit-spline-lowess",
        "levene-test",
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

    let manual_out = sim_root.join("target/docs-manual");
    run_xtask_ok(sim_root, &["docs-site", "--release", "0.1.0-alpha.2", "--check"])?;
    assert!(!sim_root.join("docs/manual").exists(), "--release must write manual HTML into target/docs-manual, never docs/manual");

    let manifest_text = assert_release_snapshot_outputs(sim_root, &manual_out, &head_sha)?;
    assert_release_negative_gates_and_ci_stage(sim_root, &manual_out, &head_sha, &manifest_text)
}

fn assert_release_snapshot_outputs(sim_root: &Path, manual_out: &Path, head_sha: &str) -> Result<String, String> {
    let manifest_text = fs::read_to_string(manual_out.join("v0.1/manifest.json")).map_err(|e| e.to_string())?;
    let manifest_json: serde_json::Value = serde_json::from_str(&manifest_text).map_err(|e| e.to_string())?;
    assert_eq!(manifest_json["commit"], serde_json::json!(head_sha));
    assert_eq!(manifest_json["minor_dir"], serde_json::json!("v0.1"));
    assert_eq!(manifest_json["source"], serde_json::json!("release"));
    assert_eq!(fs::read_to_string(manual_out.join("latest/manifest.json")).map_err(|e| e.to_string())?, manifest_text);

    for (path, refresh, href) in [
        (manual_out.join("index.html"), "latest/index.html", "v0.1/index.html"),
        (sim_root.join("docs/get-started.html"), "manual/latest/get-started.html", "manual/latest/get-started.html"),
        (
            sim_root.join("docs/troubleshooting.html"),
            "manual/latest/troubleshooting.html#known-issues",
            "manual/latest/troubleshooting.html#known-issues",
        ),
    ] {
        let html = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        assert!(html.contains(&format!("<meta http-equiv=\"refresh\" content=\"0; url={refresh}\">")));
        assert!(html.contains(&format!("href=\"{href}\"")));
        assert!(html.contains("class=\"site-shell\""));
        assert!(!html.contains("desktop-window") && !html.contains("title-bar") && !html.contains("window-statusbar"));
        assert!(!html.contains("<script"));
    }

    let snap_home = fs::read_to_string(manual_out.join("v0.1/index.html")).map_err(|e| e.to_string())?;
    for needle in [
        "Product site</a>",
        "Privacy Policy</a>",
        "Documents v0.1.0-alpha.2",
        "aria-current=\"page\"",
        "<link rel=\"canonical\" href=\"https://curvebook.org/manual/latest/index.html\">",
    ] {
        assert!(snap_home.contains(needle), "missing `{needle}` in release index.html");
    }
    assert_eq!(fs::read_to_string(manual_out.join("latest/index.html")).map_err(|e| e.to_string())?, snap_home);

    let sitemap_xml = fs::read_to_string(manual_out.join("_root/sitemap.xml")).map_err(|e| e.to_string())?;
    assert!(sitemap_xml.contains("https://curvebook.org/manual/latest/index.html"));
    assert!(!sitemap_xml.contains("/manual/v0.1/"));
    assert!(
        fs::read_to_string(manual_out.join("_root/llms.txt")).map_err(|e| e.to_string())?.contains("https://curvebook.org/manual/latest/index.html")
    );
    assert!(fs::read_to_string(manual_out.join("_root/llms-full.txt")).map_err(|e| e.to_string())?.contains("## Quick Facts"));
    for absent in ["docs/sitemap.xml", "docs/llms.txt", "docs/llms-full.txt"] {
        assert!(!sim_root.join(absent).exists(), "{absent} must stay out of docs/");
    }
    Ok(manifest_text)
}

fn assert_release_negative_gates_and_ci_stage(sim_root: &Path, manual_out: &Path, head_sha: &str, manifest_text: &str) -> Result<(), String> {
    let snap_manifest = manual_out.join("v0.1/manifest.json");
    let preview_dir = sim_root.join("target/docs-site-check");
    let manual_str = manual_out.to_str().ok_or("bad path")?;
    let preview_str = preview_dir.to_str().ok_or("bad path")?;

    // Corrupting the manifest commit must make `--check --manual-dir` fail because it no longer matches tag `v0.1.0-alpha.2`.
    fs::write(&snap_manifest, manifest_text.replace(head_sha, "0000000000000000000000000000000000000000")).map_err(|e| e.to_string())?;
    run_xtask_fail(sim_root, &["docs-site", "--check", "--manual-dir", manual_str, "--output", preview_str], "does not match release tag")?;

    // Restoring manifest and planting committed HTML under docs/manual/ must fail --check (R12.10).
    fs::write(&snap_manifest, manifest_text).map_err(|e| e.to_string())?;
    fs::create_dir_all(sim_root.join("docs/manual/latest")).map_err(|e| e.to_string())?;
    fs::write(sim_root.join("docs/manual/latest/index.html"), "<html></html>").map_err(|e| e.to_string())?;
    run_xtask_fail(
        sim_root,
        &["docs-site", "--check", "--manual-dir", manual_str, "--output", preview_str],
        "must not be committed under docs/manual/",
    )?;
    fs::remove_dir_all(sim_root.join("docs/manual")).map_err(|e| e.to_string())?;

    // Planting a committed generated root file under docs/ must also fail --check (R21.4).
    fs::write(sim_root.join("docs/llms-full.txt"), "# stale\n").map_err(|e| e.to_string())?;
    run_xtask_fail(sim_root, &["docs-site", "--check", "--manual-dir", manual_str, "--output", preview_str], "must not be committed under docs/")?;
    fs::remove_file(sim_root.join("docs/llms-full.txt")).map_err(|e| e.to_string())?;

    // Exercise the exact Pages CI sequence: `--build-manual <dir> --check` followed by `pages-stage --manual-dir <dir> _site`.
    let ci_manual = sim_root.join("target/ci-manual");
    let ci_manual_str = ci_manual.to_str().ok_or("bad path")?;
    run_xtask_ok(sim_root, &["docs-site", "--build-manual", ci_manual_str, "--check"])?;
    let staged_site = sim_root.join("_site");
    let stage_res = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .current_dir(sim_root)
        .args(["pages-stage", "--manual-dir", ci_manual_str, staged_site.to_str().ok_or("bad path")?])
        .output()
        .map_err(|e| format!("run pages-stage: {e}"))?;
    assert!(
        stage_res.status.success(),
        "pages-stage --manual-dir failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&stage_res.stdout),
        String::from_utf8_lossy(&stage_res.stderr)
    );
    for present in [
        "CNAME",
        "robots.txt",
        "index.html",
        "manual/index.html",
        "manual/latest/index.html",
        "manual/v0.1/index.html",
        "sitemap.xml",
        "llms.txt",
        "llms-full.txt",
    ] {
        assert!(staged_site.join(present).is_file(), "expected staged `{present}`");
    }
    assert_eq!(fs::read_to_string(staged_site.join("CNAME")).map_err(|e| e.to_string())?, "curvebook.org\n");
    assert!(fs::read_to_string(staged_site.join("robots.txt")).map_err(|e| e.to_string())?.contains("Sitemap: https://curvebook.org/sitemap.xml"));
    for absent in ["manual/latest/manifest.json", "manual/v0.1/manifest.json", "ROADMAP.md"] {
        assert!(!staged_site.join(absent).exists(), "unexpected staged `{absent}`");
    }

    assert_aud12_snapshot_and_guard_regressions(sim_root, manual_out, preview_str)
}

fn assert_aud12_snapshot_and_guard_regressions(sim_root: &Path, manual_out: &Path, preview_str: &str) -> Result<(), String> {
    let manual_str = manual_out.to_str().ok_or("bad path")?;

    // AUD-12-25: `--output` inside `docs/` (including via `..` traversal) must fail closed.
    run_xtask_fail(sim_root, &["docs-site", "--output", "docs"], "strictly outside `docs/`")?;
    run_xtask_fail(sim_root, &["docs-site", "--output", "target/../docs/preview"], "strictly outside `docs/`")?;

    // AUD-12-06: Tampering snapshot HTML (script injection or latest/newest mismatch) must fail `--check --manual-dir`.
    let snap_v01_index = manual_out.join("v0.1/index.html");
    let snap_latest_index = manual_out.join("latest/index.html");
    let orig_snap_html = fs::read_to_string(&snap_v01_index).map_err(|e| e.to_string())?;
    let injected_html = orig_snap_html.replace("</body>", "<script>alert(1)</script></body>");
    fs::write(&snap_v01_index, &injected_html).map_err(|e| e.to_string())?;
    fs::write(&snap_latest_index, &injected_html).map_err(|e| e.to_string())?;
    run_xtask_fail(sim_root, &["docs-site", "--check", "--manual-dir", manual_str, "--output", preview_str], "<script>")?;
    fs::write(&snap_v01_index, &orig_snap_html).map_err(|e| e.to_string())?;
    fs::write(&snap_latest_index, orig_snap_html.replace("CurveBook Reference Manual", "Stale CurveBook Reference Manual"))
        .map_err(|e| e.to_string())?;
    run_xtask_fail(sim_root, &["docs-site", "--check", "--manual-dir", manual_str, "--output", preview_str], "does not match")?;
    fs::write(&snap_latest_index, &orig_snap_html).map_err(|e| e.to_string())?;

    // AUD-12-25: Building a newer minor release (`0.2.0`) then backporting an older minor (`0.1.1`) must keep `latest/` on `v0.2`.
    for tag in ["v0.2.0", "v0.1.1"] {
        let out = Command::new("git").current_dir(sim_root).args(["tag", tag]).output().map_err(|e| e.to_string())?;
        assert!(out.status.success(), "git tag {tag} failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    run_xtask_ok(sim_root, &["docs-site", "--release", "0.2.0", "--check"])?;
    run_xtask_ok(sim_root, &["docs-site", "--release", "0.1.1", "--check"])?;
    let latest_manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(manual_out.join("latest/manifest.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let v01_manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(manual_out.join("v0.1/manifest.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    assert_eq!(latest_manifest["minor_dir"], serde_json::json!("v0.2"));
    assert_eq!(latest_manifest["version"], serde_json::json!("0.2.0"));
    assert_eq!(v01_manifest["version"], serde_json::json!("0.1.1"));
    assert_eq!(
        fs::read_to_string(manual_out.join("latest/index.html")).map_err(|e| e.to_string())?,
        fs::read_to_string(manual_out.join("v0.2/index.html")).map_err(|e| e.to_string())?
    );

    Ok(())
}

#[test]
fn docs_site_renders_math_callouts_figures_toc_and_rejects_leaks_and_broken_root_pages() -> Result<(), String> {
    let root = workspace_root()?;
    let sandbox = WorkspaceScratchDir::new(&root, "math-callout-root")?;
    let sim_root = &sandbox.path;
    SimRepoTemplate::shared()?.materialize_into(sim_root)?;

    let qol_md = sim_root.join("docs/reference/qol.md");
    let orig_qol = fs::read_to_string(&qol_md).map_err(|e| e.to_string())?;
    let mut headings = String::new();
    for i in 1..=9 {
        headings.push_str(&format!("## Section {i}\n\nItem {i} with $\\frac{{{i}}}{{2}}$, $\\sqrt{{x_{i}}}$, $\\bar{{d}}$, $\\hat{{\\varepsilon}}$, $\\tilde{{\\varepsilon}}$, $\\Delta\\text{{AIC}}_c$.\n\n"));
    }
    let test_md = format!(
        "{orig_qol}\n\n\
        ## Synthetic Callout Suite\n\n\
        > [!NOTE]\n> Inline Greek $\\alpha \\chi \\Phi \\kappa \\pi \\tau \\theta \\ge 5.0\\text{{ pt}} \\le 10$ and $\\ln \\log \\arcsin \\exp \\TeX$ and $\\left(\\frac{{p}}{{1 - p}}\\right)\\,\\!$ with $K_{{\\text{{eff}}}}$.\n\n\
        > [!TIP]\n> Tip callout.\n\n\
        > [!IMPORTANT]\n> Important callout.\n\n\
        > [!WARNING]\n> Warning callout.\n\n\
        > [!CAUTION]\n> Caution callout.\n\n\
        Currency & shell: \\$100, $100, $50, between $50 and $100, $50-$100, $HOME, $XDG_CONFIG_HOME, $TMPDIR, $PATH, $HOME/$PATH, $row and $index, `$row`.\n\n\
        $$\n\\chi^2 = \\frac{{(O - E)^2}}{{E}}\n$$\n\n\
        ![Split fit screenshot](../images/workspace-split-fit.png)\n\n\
        {headings}"
    );
    fs::write(&qol_md, &test_md).map_err(|e| e.to_string())?;
    run_xtask_ok(sim_root, &["docs-site", "--sitemap"])?;

    let out_dir = WorkspaceScratchDir::new(&root, "math-callout-out")?;
    let out_str = out_dir.path.to_str().ok_or("bad path")?;
    run_xtask_ok(sim_root, &["docs-site", "--check", "--output", out_str])?;

    let qol_html = fs::read_to_string(out_dir.path.join("qol.html")).map_err(|e| e.to_string())?;
    for expected in [
        "<div class=\"callout callout-note\" role=\"note\">\n<p class=\"callout-title\">Note</p>",
        "<div class=\"callout callout-tip\" role=\"note\">\n<p class=\"callout-title\">Tip</p>",
        "<div class=\"callout callout-important\" role=\"note\">\n<p class=\"callout-title\">Important</p>",
        "<div class=\"callout callout-warning\" role=\"note\">\n<p class=\"callout-title\">Warning</p>",
        "<div class=\"callout callout-caution\" role=\"note\">\n<p class=\"callout-title\">Caution</p>",
        "<details class=\"toc\">\n          <summary class=\"toc-title\">On this page</summary>",
        "<figure class=\"doc-figure\"><img src=\"images/workspace-split-fit.png\" alt=\"Split fit screenshot\" loading=\"lazy\"><figcaption>Split fit screenshot</figcaption></figure>",
        "<div class=\"math-block\"><span class=\"math\">",
        "<span class=\"math-frac\">",
        "√<span class=\"math-rad\">",
        "$100",
        "$50",
        "between $50 and $100",
        "$50-$100",
        "$HOME",
        "$XDG_CONFIG_HOME",
        "$TMPDIR",
        "$PATH",
        "$HOME/$PATH",
        "$row and $index",
        "<code>$row</code>",
    ] {
        assert!(qol_html.contains(expected), "qol.html missing expected `{expected}`:\n{qol_html}");
    }
    let (_, synthetic_tail) =
        qol_html.split_once("<h2 id=\"synthetic-callout-suite\" tabindex=\"-1\">").ok_or("missing synthetic callout suite heading")?;
    assert!(!synthetic_tail.contains("<em>"), "underscores inside math must not become <em>");

    // Negative tests: raw leaked TeX or callout markers outside code must fail --check.
    for (leak, needle) in [("Unwrapped \\frac{1}{2} macro.", "unrendered TeX macro"), ("Plain [!NOTE] outside blockquote.", "callout marker")] {
        fs::write(&qol_md, format!("{orig_qol}\n\n{leak}\n")).map_err(|e| e.to_string())?;
        run_xtask_fail(sim_root, &["docs-site", "--check", "--output", out_str], needle)?;
    }

    // Restore valid qol.md, then test check_root_pages catches broken root HTML and broken root shims.
    fs::write(&qol_md, &orig_qol).map_err(|e| e.to_string())?;
    run_xtask_ok(sim_root, &["docs-site", "--sitemap"])?;
    let root_index = sim_root.join("docs/index.html");
    let orig_root_index = fs::read_to_string(&root_index).map_err(|e| e.to_string())?;
    fs::write(&root_index, orig_root_index.replace("privacy.html", "does-not-exist.html")).map_err(|e| e.to_string())?;
    run_xtask_fail(sim_root, &["docs-site", "--check", "--output", out_str], "does-not-exist.html")?;

    fs::write(&root_index, orig_root_index).map_err(|e| e.to_string())?;
    fs::write(sim_root.join("docs/get-started.html"), "<script>alert(1)</script>").map_err(|e| e.to_string())?;
    run_xtask_fail(sim_root, &["docs-site", "--check", "--output", out_str], "")?;

    Ok(())
}
