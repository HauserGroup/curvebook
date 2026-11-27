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
        SIM_REPO_TEMPLATE.get_or_init(Self::build_from_workspace).as_ref().map_err(Clone::clone)
    }

    fn build_from_workspace() -> Result<Self, String> {
        let root = workspace_root()?;
        let mut files = Vec::new();
        for file in ["AGENTS.md", "CITATION.cff", "NOTICE", "README.md", "SECURITY.md", "Cargo.toml"] {
            files.push((PathBuf::from(file), fs::read(root.join(file)).map_err(|e| format!("read {file}: {e}"))?));
        }
        let mut docs_paths = Vec::new();
        for entry in sorted_entries(&root.join("docs"))? {
            let name = entry.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if entry.is_file() && !matches!(name, "STATUS.md" | "gui-workflows.md") {
                docs_paths.push(Path::new("docs").join(name));
            } else if entry.is_dir() && matches!(name, "images" | "schemas" | "release" | "reference" | "evidence") {
                collect_rel_files(&root, &entry, &mut docs_paths)?;
            }
        }
        let mut needed_assets = BTreeSet::new();
        for rel in &docs_paths {
            let s = rel.to_string_lossy().replace('\\', "/");
            if s.starts_with("docs/images/") {
                needed_assets.insert(s);
            }
            files.push((rel.clone(), fs::read(root.join(rel)).map_err(|e| format!("read {}: {e}", rel.display()))?));
        }
        let assets_md = fs::read_to_string(root.join("ASSETS.md")).map_err(|e| e.to_string())?;
        let (filtered_assets, transitive) = filter_assets_md(&assets_md, &needed_assets);
        files.push((PathBuf::from("ASSETS.md"), filtered_assets.into_bytes()));
        let attr_md = fs::read_to_string(root.join("ATTRIBUTION.md")).map_err(|e| e.to_string())?;
        files.push((PathBuf::from("ATTRIBUTION.md"), filter_table(&attr_md, &transitive).into_bytes()));
        for rel in transitive.into_iter().filter(|r| r.starts_with("assets/")) {
            files.push((PathBuf::from(&rel), fs::read(root.join(&rel)).map_err(|e| format!("read {rel}: {e}"))?));
        }
        Ok(Self { files })
    }

    fn materialize_into(&self, dst_root: &Path) -> Result<(), String> {
        for (rel, bytes) in &self.files {
            let dst = dst_root.join(rel);
            if let Some(p) = dst.parent() {
                fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            fs::write(&dst, bytes).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

fn sorted_entries(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out: Vec<_> = fs::read_dir(dir).map_err(|e| e.to_string())?.filter_map(Result::ok).map(|e| e.path()).collect();
    out.sort();
    Ok(out)
}

fn collect_rel_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for p in sorted_entries(dir)? {
        if p.is_dir() {
            collect_rel_files(root, &p, out)?;
        } else if p.is_file() {
            out.push(p.strip_prefix(root).map_err(|e| e.to_string())?.to_path_buf());
        }
    }
    Ok(())
}

fn parse_table_row(line: &str) -> Option<(&str, &str, &str)> {
    let cells: Vec<&str> = line.trim().strip_prefix('|')?.strip_suffix('|')?.split('|').map(str::trim).collect();
    let [asset, _, src, lic, ..] = cells.as_slice() else { return None };
    let path = asset.strip_prefix('`')?.strip_suffix('`')?.trim();
    (!path.is_empty()).then_some((path, src, lic))
}

fn filter_assets_md(assets_md: &str, seed: &BTreeSet<String>) -> (String, BTreeSet<String>) {
    let mut refs_by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in assets_md.lines() {
        if let Some((path, src, lic)) = parse_table_row(line) {
            let mut refs = Vec::new();
            for cell in [src, lic] {
                let mut rest = cell;
                while let Some((_, a)) = rest.split_once('`') {
                    let Some((tok, tail)) = a.split_once('`') else { break };
                    if tok.trim().contains('/') {
                        refs.push(tok.trim().to_owned());
                    }
                    rest = tail;
                }
            }
            refs_by.insert(path.to_owned(), refs);
        }
    }
    let (mut needed, mut q) = (seed.clone(), seed.iter().cloned().collect::<Vec<_>>());
    while let Some(p) = q.pop() {
        for parent in refs_by.get(&p).into_iter().flatten() {
            if needed.insert(parent.clone()) {
                q.push(parent.clone());
            }
        }
    }
    (filter_table(assets_md, &needed), needed)
}

fn filter_table(md: &str, keep: &BTreeSet<String>) -> String {
    md.lines().filter(|l| parse_table_row(l).is_none_or(|(p, _, _)| keep.contains(p))).map(|l| format!("{l}\n")).collect()
}

fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(Path::to_path_buf).ok_or_else(|| String::from("no workspace root"))
}

struct ScratchDir {
    path: PathBuf,
}
impl ScratchDir {
    fn new(root: &Path, label: &str) -> Result<Self, String> {
        let dir =
            root.join("target/r12-docs-site-tests").join(format!("{label}-{}-{}", std::process::id(), TEST_SEQ.fetch_add(1, Ordering::Relaxed)));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Ok(Self { path: dir })
    }
}
impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn run_xtask(cwd: &Path, args: &[&str]) -> Result<Output, String> {
    Command::new(env!("CARGO_BIN_EXE_xtask")).current_dir(cwd).env("CURVEBOOK_DOCS_SITE_ROOT", cwd).args(args).output().map_err(|e| e.to_string())
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
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(needle.is_empty() || stderr.contains(needle), "expected `{needle}` in stderr:\n{stderr}");
    Ok(())
}

fn collect_html_map(base: &Path) -> Result<BTreeMap<String, String>, String> {
    let (mut files, mut map) = (Vec::new(), BTreeMap::new());
    collect_rel_files(base, base, &mut files)?;
    for rel in files.into_iter().filter(|r| r.extension().and_then(|e| e.to_str()) == Some("html")) {
        map.insert(rel.to_string_lossy().replace('\\', "/"), fs::read_to_string(base.join(&rel)).map_err(|e| e.to_string())?);
    }
    Ok(map)
}

#[test]
fn docs_site_check_passes_and_output_is_deterministic() -> Result<(), String> {
    let root = workspace_root()?;
    let (out_a, out_b) = (ScratchDir::new(&root, "det-a")?, ScratchDir::new(&root, "det-b")?);
    run_xtask_ok(&root, &["docs-site", "--check", "--output", out_a.path.to_str().ok_or("utf8")?])?;
    run_xtask_ok(&root, &["docs-site", "--output", out_b.path.to_str().ok_or("utf8")?])?;
    let (map_a, map_b) = (collect_html_map(&out_a.path)?, collect_html_map(&out_b.path)?);
    assert_eq!(map_a, map_b, "docs-site HTML must be byte-for-byte deterministic");

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
        assert!(map_a.contains_key(key), "missing `{key}`");
    }
    assert!(!map_a.keys().any(|k| k.starts_with("contribute/")) && !map_a.contains_key("ROADMAP.html") && !map_a.contains_key("STATUS.html"));

    let contrib = &map_a["section-contribute.html"];
    for p in ["AGENTS.md", "docs/architecture.md", "docs/development.md", "docs/extending.md", "docs/ci.md", "docs/release/releasing.md"] {
        assert!(contrib.contains(&format!("https://github.com/HauserGroup/curvebook/blob/main/{p}")));
    }
    let home = &map_a["index.html"];
    for s in
        ["class=\"site-shell is-manual\"", "class=\"manual-sidebar\"", "Start Here", "Development build · main", "1 page", "href=\"concepts.html\""]
    {
        assert!(home.contains(s), "index.html missing `{s}`");
    }
    for bad in ["desktop-window", "title-bar", "1 pages", "class=\"breadcrumbs\"", "class=\"pager\""] {
        assert!(!home.contains(bad), "index.html contains `{bad}`");
    }
    assert!(map_a["fitting.html"].contains("href=\"#hougaard-skewness-g1\">Hougaard Skewness (g1)</a>"));
    assert!(map_a["automation/control-and-batch.html"].contains("https://github.com/HauserGroup/curvebook/blob/main/docs/command-ids.md"));
    assert!(map_a["az-index.html"].contains("class=\"az-letters\"") && !map_a["az-index.html"].contains("class=\"toc\""));
    assert!(map_a["tables.html"].contains("<td><code>||</code>, <code>OR</code></td>"));
    assert!(map_a["glossary.html"].contains("<span class=\"math\">") && !map_a["glossary.html"].contains("$\\text{AIC}_c$"));
    assert!(map_a["desktop-gui.html"].contains("<figure class=\"doc-figure\">"));
    let eq = &map_a["registry/equations.html"];
    assert!(
        eq.contains("<span class=\"math-frac\">")
            && eq.contains("<span class=\"math-text\">Bottom</span> + <span class=\"math-frac\">")
            && eq.contains("1e-30")
    );
    assert!(map_a["registry/graphs.html"].contains("id=\"graph-layout-features\""));
    Ok(())
}

#[test]
fn docs_site_check_passes_with_absent_and_partial_analyses_md() -> Result<(), String> {
    let root = workspace_root()?;
    let sb = ScratchDir::new(&root, "partial")?;
    SimRepoTemplate::shared()?.materialize_into(&sb.path)?;
    for f in ["analyses.md", "fitting.md"] {
        let p = sb.path.join("docs/reference").join(f);
        if p.exists() {
            fs::remove_file(p).map_err(|e| e.to_string())?;
        }
    }
    run_xtask_ok(&sb.path, &["docs-site", "--sitemap"])?;
    let out1 = ScratchDir::new(&root, "absent-out")?;
    run_xtask_ok(&sb.path, &["docs-site", "--check", "--output", out1.path.to_str().ok_or("utf8")?])?;
    let az1 = fs::read_to_string(out1.path.join("az-index.html")).map_err(|e| e.to_string())?;
    assert!(az1.contains("registry/tables.html#analysis-kind-two-group-comparison") && !az1.contains("analyses.html#analysis-two-group-comparison"));

    fs::write(
        sb.path.join("docs/reference/analyses.md"),
        "# Statistical Analyses\n\n## Two-Group Comparison {#analysis-two-group-comparison}\n\nDetails.\n",
    )
    .map_err(|e| e.to_string())?;
    run_xtask_ok(&sb.path, &["docs-site", "--sitemap"])?;
    let out2 = ScratchDir::new(&root, "partial-out")?;
    run_xtask_ok(&sb.path, &["docs-site", "--check", "--output", out2.path.to_str().ok_or("utf8")?])?;
    let az2 = fs::read_to_string(out2.path.join("az-index.html")).map_err(|e| e.to_string())?;
    assert!(az2.contains("analyses.html#analysis-two-group-comparison") && az2.contains("registry/tables.html#analysis-kind-survival"));
    Ok(())
}

#[test]
fn docs_site_release_snapshot_pages_stage_and_negative_gates() -> Result<(), String> {
    let root = workspace_root()?;
    let sb = ScratchDir::new(&root, "snapshot-sim")?;
    let sim = &sb.path;
    SimRepoTemplate::shared()?.materialize_into(sim)?;
    let ref_dir = sim.join("docs/reference");
    fs::create_dir_all(&ref_dir).map_err(|e| e.to_string())?;
    let cite_ref = curvebook_doc::CURVEBOOK_CITATION.plain_text_reference(None);
    let cite_bib = curvebook_doc::CURVEBOOK_CITATION.bibtex(None);
    let get_started_stub = format!(
        "# Get Started\n\nSee [MCP](../mcp-for-beginners.md).\n\n## Linux {{#install-linux}}\n\nNotes.\n\n## Verify {{#verifying-downloads}}\n\nChecksums.\n\n## Cite {{#how-to-cite}}\n\nSee CITATION.cff.\n\n> {cite_ref}\n\n```bibtex\n{cite_bib}\n```\n"
    );
    for (name, body) in [
        ("get-started", get_started_stub.as_str()),
        ("desktop-gui", "# Desktop GUI\n\nSee [Commands](../commands.md).\n"),
        ("concepts", "# Concepts\n\nSee [Tables](tables.md).\n"),
        ("tables", "# Tables\n\nSee [Files](files.md).\n\n## Archetypes {#table-archetypes}\n\nXY.\n"),
        ("fitting", "# Fitting\n\nSee [Equations](registry/equations.html#eq-michaelis_menten).\n"),
        (
            "graphs",
            "# Graphs\n\n![Shot](../images/workspace-split-fit.png)\n\n## Families {#graph-families}\n\nPlots.\n\n## Layouts {#page-layouts}\n\nPanels.\n",
        ),
        ("qol", "# QoL\n\nSee [Graphs](graphs.md).\n"),
        ("files", "# Files\n\nSee [Batch](../schemas/batch-v1.schema.json).\n\n## Native {#native-formats}\n\nArchives.\n"),
        ("settings", "# Settings\n\nSee [Prefs](../schemas/preferences.schema.json).\n"),
        ("privacy-security", "# Privacy\n\nSee [Security](../../SECURITY.md).\n"),
        (
            "troubleshooting",
            "# Troubleshooting\n\n## Known Issues {#known-issues}\n\n### MCP {#mcp-connection}\n\nCheck.\n\n### FUSE {#linux-fuse}\n\nExtract.\n",
        ),
        ("glossary", "# Glossary\n\n## Reactive DAG {#term-reactive-dag}\n\nRecomputes.\n"),
    ] {
        fs::write(ref_dir.join(format!("{name}.md")), body).map_err(|e| e.to_string())?;
    }
    let mut analyses = String::from("# Analyses\n\n");
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
        analyses.push_str(&format!("## {slug} {{#analysis-{slug}}}\n\nRef.\n\n"));
    }
    fs::write(ref_dir.join("analyses.md"), analyses).map_err(|e| e.to_string())?;

    for args in [
        &["init"][..],
        &["config", "user.email", "t@cb.invalid"],
        &["config", "user.name", "CB"],
        &["add", "."],
        &["commit", "-m", "init"],
        &["tag", "v0.1.0-alpha.2"],
    ] {
        assert!(Command::new("git").current_dir(sim).args(args).output().map_err(|e| e.to_string())?.status.success());
    }
    let head_sha =
        String::from_utf8_lossy(&Command::new("git").current_dir(sim).args(["rev-parse", "HEAD"]).output().map_err(|e| e.to_string())?.stdout)
            .trim()
            .to_owned();

    let manual_out = sim.join("target/docs-manual");
    run_xtask_ok(sim, &["docs-site", "--release", "0.1.0-alpha.2", "--check"])?;
    assert!(!sim.join("docs/manual").exists());
    let manifest = fs::read_to_string(manual_out.join("v0.1/manifest.json")).map_err(|e| e.to_string())?;
    assert!(manifest.contains(&head_sha) && fs::read_to_string(manual_out.join("latest/manifest.json")).map_err(|e| e.to_string())? == manifest);
    assert!(fs::read_to_string(manual_out.join("v0.1/index.html")).map_err(|e| e.to_string())?.contains("Documents v0.1.0-alpha.2"));
    assert!(fs::read_to_string(manual_out.join("_root/sitemap.xml")).map_err(|e| e.to_string())?.contains("/manual/latest/index.html"));
    assert!(!sim.join("docs/sitemap.xml").exists() && !sim.join("docs/llms-full.txt").exists());

    let (m_str, p_str) = (manual_out.to_str().ok_or("utf8")?, sim.join("target/check-p").to_str().ok_or("utf8")?.to_owned());
    fs::write(manual_out.join("v0.1/manifest.json"), manifest.replace(&head_sha, &"0".repeat(40))).map_err(|e| e.to_string())?;
    run_xtask_fail(sim, &["docs-site", "--check", "--manual-dir", m_str, "--output", &p_str], "does not match release tag")?;
    fs::write(manual_out.join("v0.1/manifest.json"), &manifest).map_err(|e| e.to_string())?;

    fs::create_dir_all(sim.join("docs/manual/latest")).map_err(|e| e.to_string())?;
    fs::write(sim.join("docs/manual/latest/index.html"), "<html></html>").map_err(|e| e.to_string())?;
    run_xtask_fail(sim, &["docs-site", "--check", "--manual-dir", m_str, "--output", &p_str], "must not be committed under docs/manual/")?;
    fs::remove_dir_all(sim.join("docs/manual")).map_err(|e| e.to_string())?;

    fs::write(sim.join("docs/llms-full.txt"), "# stale\n").map_err(|e| e.to_string())?;
    run_xtask_fail(sim, &["docs-site", "--check", "--manual-dir", m_str, "--output", &p_str], "must not be committed under docs/")?;
    fs::remove_file(sim.join("docs/llms-full.txt")).map_err(|e| e.to_string())?;

    for (path, refresh, href, canonical) in [
        (manual_out.join("index.html"), "latest/index.html", "v0.1/index.html", "https://curvebook.org/manual/latest/index.html"),
        (
            sim.join("docs/get-started.html"),
            "manual/latest/get-started.html",
            "manual/latest/get-started.html",
            "https://curvebook.org/manual/latest/get-started.html",
        ),
        (
            sim.join("docs/troubleshooting.html"),
            "manual/latest/troubleshooting.html#known-issues",
            "manual/latest/troubleshooting.html#known-issues",
            "https://curvebook.org/manual/latest/troubleshooting.html",
        ),
    ] {
        let html = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        assert!(!html.contains("name=\"robots\""), "redirect shims must not pair noindex with a cross-URL canonical");
        assert!(html.contains(&format!("<link rel=\"canonical\" href=\"{canonical}\">")));
        assert!(html.contains(&format!("<meta http-equiv=\"refresh\" content=\"0; url={refresh}\">")));
        assert!(html.contains(&format!("href=\"{href}\"")));
    }

    let ci_manual = sim.join("target/ci-manual");
    let ci_str = ci_manual.to_str().ok_or("utf8")?;
    run_xtask_ok(sim, &["docs-site", "--build-manual", ci_str, "--check"])?;
    let staged = sim.join("_site");
    let st = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .current_dir(sim)
        .args(["pages-stage", "--manual-dir", ci_str, staged.to_str().ok_or("utf8")?])
        .output()
        .map_err(|e| e.to_string())?;
    assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stderr));
    for f in ["CNAME", "robots.txt", "index.html", "manual/index.html", "manual/latest/index.html", "sitemap.xml", "llms.txt", "llms-full.txt"] {
        assert!(staged.join(f).is_file(), "missing staged `{f}`");
    }
    assert!(!staged.join("manual/latest/manifest.json").exists() && !staged.join("ROADMAP.md").exists());

    run_xtask_fail(sim, &["docs-site", "--output", "target/../docs/preview"], "strictly outside `docs/`")?;
    let orig = fs::read_to_string(manual_out.join("v0.1/index.html")).map_err(|e| e.to_string())?;
    for p in ["v0.1/index.html", "latest/index.html"] {
        fs::write(manual_out.join(p), orig.replace("</body>", "<script>x()</script></body>")).map_err(|e| e.to_string())?;
    }
    run_xtask_fail(sim, &["docs-site", "--check", "--manual-dir", m_str, "--output", &p_str], "<script>")?;
    fs::write(manual_out.join("v0.1/index.html"), &orig).map_err(|e| e.to_string())?;
    fs::write(manual_out.join("latest/index.html"), orig.replace("CurveBook Reference Manual", "Stale")).map_err(|e| e.to_string())?;
    run_xtask_fail(sim, &["docs-site", "--check", "--manual-dir", m_str, "--output", &p_str], "does not match")?;
    fs::write(manual_out.join("latest/index.html"), &orig).map_err(|e| e.to_string())?;

    for tag in ["v0.2.0", "v0.1.1"] {
        assert!(Command::new("git").current_dir(sim).args(["tag", tag]).output().map_err(|e| e.to_string())?.status.success());
    }
    run_xtask_ok(sim, &["docs-site", "--release", "0.2.0", "--check"])?;
    run_xtask_ok(sim, &["docs-site", "--release", "0.1.1", "--check"])?;
    let latest_m: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(manual_out.join("latest/manifest.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    assert_eq!(latest_m["version"], serde_json::json!("0.2.0"));
    Ok(())
}

#[test]
fn docs_site_math_callouts_figures_leaks_and_broken_root_pages() -> Result<(), String> {
    let root = workspace_root()?;
    let sb = ScratchDir::new(&root, "math-callout")?;
    let sim = &sb.path;
    SimRepoTemplate::shared()?.materialize_into(sim)?;

    let qol_md = sim.join("docs/reference/qol.md");
    let orig_qol = fs::read_to_string(&qol_md).map_err(|e| e.to_string())?;
    let mut headings = String::new();
    for i in 1..=9 {
        headings.push_str(&format!("## Section {i}\n\n$\\frac{{{i}}}{{2}}$ $\\sqrt{{x_{i}}}$ $\\bar{{d}}$ $\\Delta\\text{{AIC}}_c$.\n\n"));
    }
    let test_md = format!(
        "{orig_qol}\n\n## Synthetic Suite\n\n> [!NOTE]\n> $\\alpha \\chi \\ge 5\\text{{ pt}}$ $K_{{\\text{{eff}}}}$.\n\n> [!WARNING]\n> Warn.\n\n$100 $HOME `$row`.\n\n$$\n\\chi^2 = \\frac{{(O - E)^2}}{{E}}\n$$\n\n![Shot](../images/workspace-split-fit.png)\n\n{headings}"
    );
    fs::write(&qol_md, test_md).map_err(|e| e.to_string())?;
    run_xtask_ok(sim, &["docs-site", "--sitemap"])?;

    let out = ScratchDir::new(&root, "math-out")?;
    let out_str = out.path.to_str().ok_or("utf8")?;
    run_xtask_ok(sim, &["docs-site", "--check", "--output", out_str])?;
    let html = fs::read_to_string(out.path.join("qol.html")).map_err(|e| e.to_string())?;
    for s in [
        "callout-note",
        "callout-warning",
        "<details class=\"toc\">",
        "<figure class=\"doc-figure\">",
        "<div class=\"math-block\">",
        "<span class=\"math-frac\">",
        "$100",
        "$HOME",
        "<code>$row</code>",
    ] {
        assert!(html.contains(s), "qol.html missing `{s}`");
    }
    for (leak, needle) in [("Raw \\frac{1}{2}.", "unrendered TeX macro"), ("Plain [!NOTE].", "callout marker")] {
        fs::write(&qol_md, format!("{orig_qol}\n\n{leak}\n")).map_err(|e| e.to_string())?;
        run_xtask_fail(sim, &["docs-site", "--check", "--output", out_str], needle)?;
    }
    fs::write(&qol_md, &orig_qol).map_err(|e| e.to_string())?;
    run_xtask_ok(sim, &["docs-site", "--sitemap"])?;
    let idx = sim.join("docs/index.html");
    let orig_idx = fs::read_to_string(&idx).map_err(|e| e.to_string())?;
    fs::write(&idx, orig_idx.replace("privacy.html", "does-not-exist.html")).map_err(|e| e.to_string())?;
    run_xtask_fail(sim, &["docs-site", "--check", "--output", out_str], "does-not-exist.html")?;
    fs::write(&idx, orig_idx.replace("No tracking", "Easy to cite")).map_err(|e| e.to_string())?;
    run_xtask_fail(sim, &["docs-site", "--check", "--output", out_str], "easy to cite")?;
    fs::write(&idx, &orig_idx).map_err(|e| e.to_string())?;
    let cff = sim.join("CITATION.cff");
    let orig_cff = fs::read_to_string(&cff).map_err(|e| e.to_string())?;
    fs::write(&cff, orig_cff.replace("version: ", "version: 9.9.9-")).map_err(|e| e.to_string())?;
    run_xtask_fail(sim, &["docs-site", "--check", "--output", out_str], "CITATION.cff drifted")?;
    fs::write(&cff, orig_cff).map_err(|e| e.to_string())?;
    fs::write(sim.join("docs/get-started.html"), "<script>alert(1)</script>").map_err(|e| e.to_string())?;
    run_xtask_fail(sim, &["docs-site", "--check", "--output", out_str], "")
}
