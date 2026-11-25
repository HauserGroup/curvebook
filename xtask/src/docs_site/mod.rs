//! Static GitHub Pages documentation site generator (`cargo xtask docs-site`).
//!
//! Renders hand-written reference guides (`docs/reference/*.md`, `docs/reference/automation/*.md`,
//! `docs/mcp-for-beginners.md`), direct GitHub contributor links, and live typed Rust
//! registries (`cmd::registry`, `UI_COMMANDS`, `curvebook-mcp`, `cli_catalog`, `curvebook-fit`,
//! `curvebook-doc`, `curvebook-palette`, `curvebook-format`, `docs/schemas/*.schema.json`)
//! into a deterministic, script-free Classic coral HTML manual.

#![allow(dead_code)]

pub mod llms;
pub mod markdown;
pub mod registries;
pub mod seo;
pub mod sitemap;
pub mod template;
pub mod verify;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use curvebook_platform::write_atomic;

use sitemap::{GeneratedSite, assemble_site};
use template::{SiteMeta, render_release_redirect_index, render_root_redirect_shim};
use verify::{parse_minor_dir_version, verify_manual_build, verify_site};

const USAGE: &str = "usage: cargo xtask docs-site [--check] [--output <dir>] [--build-manual [<dir>]] [--manual-dir <dir>] [--release <ver> [--refresh-screenshots]] [--sitemap]";

#[derive(Debug, Default)]
struct CliOptions {
    check: bool,
    output: Option<PathBuf>,
    build_manual: bool,
    manual_dir: Option<PathBuf>,
    release: Option<String>,
    refresh_screenshots: bool,
    sitemap: bool,
}

/// CLI entrypoint for `cargo xtask docs-site`.
pub fn run_cli(root: &Path, args: &[String]) -> Result<(), String> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return Ok(());
    }
    let root_buf = std::env::var_os("CURVEBOOK_DOCS_SITE_ROOT").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| root.to_path_buf());
    let root = root_buf.as_path();
    let opts = parse_args(args)?;
    if opts.sitemap {
        if opts.check || opts.output.is_some() || opts.build_manual || opts.manual_dir.is_some() || opts.release.is_some() || opts.refresh_screenshots
        {
            return Err(format!("`--sitemap` cannot be combined with other options\n{USAGE}"));
        }
        return run_sitemap_sync(root);
    }
    if let Some(ref version) = opts.release {
        if opts.build_manual {
            return Err(format!("`--release` and `--build-manual` are mutually exclusive\n{USAGE}"));
        }
        if opts.output.is_some() && opts.manual_dir.is_some() {
            return Err(format!("specify only one of `--output` or `--manual-dir` with `--release`\n{USAGE}"));
        }
        if opts.refresh_screenshots {
            crate::screenshots::run(root, &[])?;
        }
        let manual_dir = resolve_opt_path(root, opts.manual_dir.or(opts.output)).unwrap_or_else(|| default_manual_dir(root));
        return run_release_snapshot(root, version, &manual_dir, opts.check);
    }
    if opts.refresh_screenshots {
        return Err(format!("`--refresh-screenshots` requires `--release <ver>`\n{USAGE}"));
    }
    if opts.build_manual {
        if opts.output.is_some() && opts.manual_dir.is_some() {
            return Err(format!("specify only one of `--build-manual <dir>`, `--manual-dir <dir>`, or `--output <dir>`\n{USAGE}"));
        }
        let manual_dir = resolve_opt_path(root, opts.manual_dir.or(opts.output)).unwrap_or_else(|| default_manual_dir(root));
        return run_build_manual(root, &manual_dir, opts.check);
    }

    let custom_output = opts.output.is_some();
    let out_dir = resolve_opt_path(root, opts.output).unwrap_or_else(|| default_output_dir(root));
    if opts.check {
        let explicit_manual = resolve_opt_path(root, opts.manual_dir);
        run_check_into(root, &out_dir, explicit_manual.as_deref(), custom_output)
    } else {
        if opts.manual_dir.is_some() {
            return Err(format!("`--manual-dir` requires `--check`, `--build-manual`, or `--release`\n{USAGE}"));
        }
        let meta = preview_meta(root);
        let site = assemble_site(root, meta)?;
        write_preview_site(root, &out_dir, &site)?;
        println!("docs-site: generated {} HTML pages into {}", site.html_pages.len(), out_dir.display());
        Ok(())
    }
}

/// Gate entrypoint called from `cargo xtask ci` (`policy` stage).
pub fn run_check(root: &Path) -> Result<(), String> {
    let out_dir = default_output_dir(root);
    run_check_into(root, &out_dir, None, false)
}

fn parse_args(args: &[String]) -> Result<CliOptions, String> {
    let mut opts = CliOptions::default();
    let mut idx = 0usize;
    while idx < args.len() {
        let arg = args[idx].as_str();
        if arg == "--check" {
            opts.check = true;
        } else if arg == "--sitemap" {
            opts.sitemap = true;
        } else if arg == "--refresh-screenshots" {
            opts.refresh_screenshots = true;
        } else if let Some(val) = arg.strip_prefix("--build-manual=") {
            if val.trim().is_empty() || opts.manual_dir.is_some() {
                return Err(format!("--build-manual= requires a single directory path\n{USAGE}"));
            }
            opts.build_manual = true;
            opts.manual_dir = Some(PathBuf::from(val));
        } else if arg == "--build-manual" {
            opts.build_manual = true;
            if let Some(next) = args.get(idx + 1)
                && !next.is_empty()
                && !next.starts_with('-')
            {
                if opts.manual_dir.is_some() {
                    return Err(format!("duplicate manual directory argument\n{USAGE}"));
                }
                opts.manual_dir = Some(PathBuf::from(next));
                idx += 1;
            }
        } else if let Some(val) = arg.strip_prefix("--manual-dir=") {
            if val.trim().is_empty() || opts.manual_dir.is_some() {
                return Err(format!("--manual-dir= requires a single directory path\n{USAGE}"));
            }
            opts.manual_dir = Some(PathBuf::from(val));
        } else if arg == "--manual-dir" {
            idx += 1;
            let val = args
                .get(idx)
                .filter(|v| !v.is_empty() && !v.starts_with('-'))
                .ok_or_else(|| format!("--manual-dir requires a directory path\n{USAGE}"))?;
            if opts.manual_dir.is_some() {
                return Err(format!("duplicate `--manual-dir` argument\n{USAGE}"));
            }
            opts.manual_dir = Some(PathBuf::from(val));
        } else if let Some(val) = arg.strip_prefix("--output=") {
            if val.trim().is_empty() {
                return Err(format!("--output= requires a directory path\n{USAGE}"));
            }
            opts.output = Some(PathBuf::from(val));
        } else if arg == "--output" {
            idx += 1;
            let val = args.get(idx).ok_or_else(|| format!("--output requires a directory path\n{USAGE}"))?;
            opts.output = Some(PathBuf::from(val));
        } else if let Some(val) = arg.strip_prefix("--release=") {
            if val.trim().is_empty() {
                return Err(format!("--release= requires a semantic version\n{USAGE}"));
            }
            opts.release = Some(val.to_owned());
        } else if arg == "--release" {
            idx += 1;
            let val = args.get(idx).ok_or_else(|| format!("--release requires a semantic version\n{USAGE}"))?;
            opts.release = Some(val.to_owned());
        } else {
            return Err(format!("unknown argument `{arg}`\n{USAGE}"));
        }
        idx += 1;
    }
    Ok(opts)
}

fn resolve_opt_path(root: &Path, opt: Option<PathBuf>) -> Option<PathBuf> {
    opt.map(|p| if p.is_absolute() { p } else { root.join(p) })
}

fn default_output_dir(root: &Path) -> PathBuf {
    root.join("target/docs-site")
}

fn default_manual_dir(root: &Path) -> PathBuf {
    crate::pages_site::default_manual_dir(root)
}

fn workspace_version(root: &Path) -> String {
    std::fs::read_to_string(root.join("Cargo.toml"))
        .ok()
        .and_then(|toml| crate::version::read_workspace_version(&toml).ok())
        .unwrap_or_else(|| String::from(env!("CARGO_PKG_VERSION")))
}

fn preview_meta(root: &Path) -> SiteMeta {
    SiteMeta {
        version_label: format!("v{} (local preview)", workspace_version(root)),
        commit_sha: git_head_sha(root),
        release_date: git_head_date(root),
        is_release_snapshot: false,
    }
}

fn reject_committed_manual_dir(root: &Path, manual_dir: &Path) -> Result<(), String> {
    let forbidden = root.join("docs/manual");
    if manual_dir == forbidden || manual_dir.starts_with(&forbidden) {
        return Err(format!(
            "refusing to write generated manual HTML into `{}` (R12.10 keeps manual HTML in a build directory outside docs/manual/)",
            manual_dir.display()
        ));
    }
    Ok(())
}

struct TempDirCleanup(Option<PathBuf>);

impl Drop for TempDirCleanup {
    fn drop(&mut self) {
        if let Some(ref dir) = self.0 {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

fn run_sitemap_sync(root: &Path) -> Result<(), String> {
    seo::write_not_found_page(root)?;
    let manual_dir = default_manual_dir(root);
    let version = workspace_version(root);
    let site = build_manual_into(root, &version, &manual_dir, "workspace", false, false)?;
    println!(
        "docs-site: wrote {} and refreshed {} manual pages + _root/{{sitemap.xml,llms.txt,llms-full.txt}} in {}",
        seo::NOT_FOUND_REL_PATH,
        site.html_pages.len(),
        manual_dir.display()
    );
    Ok(())
}

fn run_check_into(root: &Path, out_dir: &Path, explicit_manual_dir: Option<&Path>, custom_output: bool) -> Result<(), String> {
    let p_meta = preview_meta(root);
    let site_a = assemble_site(root, p_meta.clone())?;
    let site_b = assemble_site(root, p_meta)?;
    if site_a.html_pages != site_b.html_pages {
        return Err(String::from("docs-site generation is non-deterministic across consecutive passes"));
    }

    write_preview_site(root, out_dir, &site_a)?;

    if let Some(manual_dir) = explicit_manual_dir {
        verify_site(root, out_dir, &site_a, manual_dir, None)?;
    } else {
        let (manual_dir, _cleanup) = if custom_output {
            let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
            let dir = root.join("target").join(format!(".docs-manual-check-{}-{nanos}", std::process::id()));
            (dir.clone(), TempDirCleanup(Some(dir)))
        } else {
            (default_manual_dir(root), TempDirCleanup(None))
        };
        let version = workspace_version(root);
        let manual_site = build_manual_into(root, &version, &manual_dir, "workspace", false, true)?;
        verify_site(root, out_dir, &site_a, &manual_dir, Some(&manual_site))?;
    }

    println!(
        "docs-site: verified {} pages, 100% registry coverage, zero broken links/fragments, CSP/privacy, determinism, and compliance gates",
        site_a.html_pages.len()
    );
    Ok(())
}

fn run_build_manual(root: &Path, manual_dir: &Path, check_after: bool) -> Result<(), String> {
    let version = workspace_version(root);
    let site = build_manual_into(root, &version, manual_dir, "workspace", false, check_after)?;
    if check_after {
        verify_manual_build(root, manual_dir, &site)?;
    }
    println!("docs-site: built manual HTML v{version} ({} pages) into {}", site.html_pages.len(), manual_dir.display());
    Ok(())
}

fn run_release_snapshot(root: &Path, version_raw: &str, manual_dir: &Path, check_after: bool) -> Result<(), String> {
    let clean_ver = version_raw.trim().trim_start_matches('v');
    let site = build_manual_into(root, clean_ver, manual_dir, "release", true, check_after)?;
    let ver_label = format!("v{clean_ver}");

    let get_started_target = "manual/latest/get-started.html";
    let get_started_shim = render_root_redirect_shim(
        "Get Started & Installation",
        "Redirecting to CurveBook Get Started & Installation in the reference manual.",
        get_started_target,
        &ver_label,
    );
    write_atomic(&root.join("docs/get-started.html"), get_started_shim.as_bytes()).map_err(|e| format!("write docs/get-started.html: {e}"))?;

    let troubleshooting_target = "manual/latest/troubleshooting.html#known-issues";
    let troubleshooting_shim = render_root_redirect_shim(
        "Troubleshooting & Known Issues",
        "Redirecting to CurveBook Troubleshooting & Known Issues in the reference manual.",
        troubleshooting_target,
        &ver_label,
    );
    write_atomic(&root.join("docs/troubleshooting.html"), troubleshooting_shim.as_bytes())
        .map_err(|e| format!("write docs/troubleshooting.html: {e}"))?;
    seo::write_not_found_page(root)?;

    if check_after {
        verify_manual_build(root, manual_dir, &site)?;
    }
    println!(
        "docs-site: wrote release manual v{clean_ver} ({} pages) and _root/{{sitemap.xml,llms.txt,llms-full.txt}} into {} and refreshed committed root shims",
        site.html_pages.len(),
        manual_dir.display()
    );
    Ok(())
}

fn build_manual_into(
    root: &Path,
    version_raw: &str,
    manual_dir: &Path,
    source_kind: &str,
    require_git_commit: bool,
    check_determinism: bool,
) -> Result<GeneratedSite, String> {
    reject_committed_manual_dir(root, manual_dir)?;
    let clean_ver = version_raw.trim().trim_start_matches('v');
    let minor_dir = format_minor_dir(clean_ver)?;
    let commit = git_head_sha(root);
    if require_git_commit && commit.is_none() {
        return Err(String::from("cannot determine git HEAD commit SHA for --release manifest"));
    }
    let release_date = git_head_date(root);

    let meta = SiteMeta {
        version_label: format!("v{clean_ver}"),
        commit_sha: commit.clone(),
        release_date: release_date.clone(),
        is_release_snapshot: true,
    };
    let site = assemble_site(root, meta.clone())?;
    if check_determinism {
        let second = assemble_site(root, meta)?;
        if site.html_pages != second.html_pages {
            return Err(String::from("docs-site manual generation is non-deterministic across consecutive passes"));
        }
    }

    let snap_dir = manual_dir.join(&minor_dir);
    write_html_tree(&snap_dir, &site)?;

    let manifest = serde_json::json!({
        "schema": "curvebook.docs.manifest.v1",
        "version": clean_ver,
        "minor_dir": minor_dir,
        "source": source_kind,
        "commit": commit,
        "release_date": release_date,
        "page_count": site.html_pages.len(),
        "registry_counts": {
            "engine_commands": site.expected_coverage.engine_commands.len(),
            "ui_commands": site.expected_coverage.ui_commands.len(),
            "mcp_tools": site.expected_coverage.mcp_tools.len(),
            "mcp_prompts": site.expected_coverage.mcp_prompts.len(),
            "cli_commands": site.expected_coverage.cli_commands.len(),
            "analysis_kinds": site.expected_coverage.analysis_kinds.len(),
            "fit_equations": site.expected_coverage.fit_equations.len(),
            "graph_kinds": site.expected_coverage.graph_kinds.len(),
            "graph_options": site.expected_coverage.graph_options.len(),
            "table_kinds": site.expected_coverage.table_kinds.len(),
            "formats": site.expected_coverage.formats.len(),
            "preference_keys": site.expected_coverage.preference_keys.len()
        }
    });
    let manifest_str = serde_json::to_string_pretty(&manifest).map_err(|e| format!("serialize manifest.json: {e}"))?;
    let manifest_bytes = format!("{manifest_str}\n");
    let manifest_path = snap_dir.join("manifest.json");
    write_atomic(&manifest_path, manifest_bytes.as_bytes()).map_err(|e| format!("write {}: {e}", manifest_path.display()))?;

    let latest_dir = manual_dir.join("latest");
    write_html_tree(&latest_dir, &site)?;
    let latest_manifest_path = latest_dir.join("manifest.json");
    write_atomic(&latest_manifest_path, manifest_bytes.as_bytes()).map_err(|e| format!("write {}: {e}", latest_manifest_path.display()))?;

    let versions = collect_snapshot_dirs(manual_dir)?;
    let ver_label = format!("v{clean_ver}");
    let redirect_html = render_release_redirect_index("latest", &ver_label, &versions);
    let redirect_path = manual_dir.join("index.html");
    write_atomic(&redirect_path, redirect_html.as_bytes()).map_err(|e| format!("write {}: {e}", redirect_path.display()))?;

    let generated_root = manual_dir.join(crate::pages_site::MANUAL_ROOT_SUBDIR);
    seo::write_sitemap_into(root, Some(manual_dir), &generated_root)?;
    llms::write_llms_files_into(root, &generated_root)?;
    if check_determinism {
        let llms_full_a = llms::build_llms_full_txt(root)?;
        let llms_full_b = llms::build_llms_full_txt(root)?;
        if llms_full_a != llms_full_b {
            return Err(String::from("docs-site llms-full.txt generation is non-deterministic across consecutive passes"));
        }
    }

    Ok(site)
}

/// Convert `"0.1.0-alpha.2"` or `"1.2"` into `"v0.1"` / `"v1.2"`.
pub fn format_minor_dir(ver: &str) -> Result<String, String> {
    let base = ver.split(['-', '+']).next().unwrap_or(ver);
    let mut parts = base.split('.');
    let major: u64 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(|| format!("invalid semantic version `{ver}` (expected X.Y[.Z])"))?;
    let minor: u64 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(|| format!("invalid semantic version `{ver}` (expected X.Y[.Z])"))?;
    Ok(format!("v{major}.{minor}"))
}

fn collect_snapshot_dirs(manual_dir: &Path) -> Result<Vec<String>, String> {
    let mut dirs: Vec<((u64, u64), String)> = Vec::new();
    if std::fs::symlink_metadata(manual_dir).is_ok_and(|m| !m.file_type().is_symlink() && m.file_type().is_dir()) {
        for entry in std::fs::read_dir(manual_dir).map_err(|e| format!("read_dir {}: {e}", manual_dir.display()))? {
            let entry = entry.map_err(|e| format!("dir entry under docs/manual: {e}"))?;
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_symlink() || !ft.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(ver) = parse_minor_dir_version(&name) {
                dirs.push((ver, name));
            }
        }
    }
    dirs.sort_by_key(|a| std::cmp::Reverse(a.0));
    Ok(dirs.into_iter().map(|(_, n)| n).collect())
}

fn write_preview_site(root: &Path, out_dir: &Path, site: &GeneratedSite) -> Result<(), String> {
    write_html_tree(out_dir, site)?;

    let css_src = root.join("docs/site.css");
    let css_dst = out_dir.join("site.css");
    std::fs::copy(&css_src, &css_dst).map_err(|e| format!("copy {} -> {}: {e}", css_src.display(), css_dst.display()))?;

    let img_src_dir = root.join("docs/images");
    let img_dst_dir = out_dir.join("images");
    if std::fs::symlink_metadata(&img_src_dir).is_ok_and(|m| !m.file_type().is_symlink() && m.file_type().is_dir()) {
        std::fs::create_dir_all(&img_dst_dir).map_err(|e| format!("create {}: {e}", img_dst_dir.display()))?;
        let mut entries: Vec<_> = std::fs::read_dir(&img_src_dir)
            .map_err(|e| format!("read_dir {}: {e}", img_src_dir.display()))?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .collect();
        entries.sort();
        for src in entries {
            if std::fs::symlink_metadata(&src).is_ok_and(|m| !m.file_type().is_symlink() && m.file_type().is_file())
                && let Some(name) = src.file_name()
            {
                let dst = img_dst_dir.join(name);
                std::fs::copy(&src, &dst).map_err(|e| format!("copy {} -> {}: {e}", src.display(), dst.display()))?;
            }
        }
    }
    Ok(())
}

fn write_html_tree(out_dir: &Path, site: &GeneratedSite) -> Result<(), String> {
    if out_dir.exists() {
        let mut existing = BTreeSet::new();
        collect_rel_files(out_dir, out_dir, &mut existing)?;
        for old_rel in existing {
            if old_rel.ends_with(".html") || old_rel == "site.css" || old_rel == "manifest.json" || old_rel.starts_with("images/") {
                let _ = std::fs::remove_file(out_dir.join(&old_rel));
            }
        }
    }
    std::fs::create_dir_all(out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;
    for (rel, html) in &site.html_pages {
        let dst = out_dir.join(rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        write_atomic(&dst, html.as_bytes()).map_err(|e| format!("write {}: {e}", dst.display()))?;
    }
    Ok(())
}

fn collect_rel_files(base: &Path, dir: &Path, out: &mut BTreeSet<String>) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("dir entry {}: {e}", dir.display()))?;
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_symlink() {
            continue;
        }
        let path = entry.path();
        if ft.is_dir() {
            collect_rel_files(base, &path, out)?;
        } else if ft.is_file()
            && let Ok(rel) = path.strip_prefix(base)
        {
            out.insert(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

fn git_head_sha(root: &Path) -> Option<String> {
    let out = crate::dev_tool_command("git")
        .current_dir(root)
        .args(["rev-parse", "HEAD"])
        .wall_timeout(Duration::from_secs(30))
        .idle_timeout(Duration::from_secs(10))
        .run()
        .ok()?;
    if !out.success {
        return None;
    }
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if sha.len() == 40 { Some(sha) } else { None }
}

fn git_head_date(root: &Path) -> Option<String> {
    let out = crate::dev_tool_command("git")
        .current_dir(root)
        .args(["show", "-s", "--format=%cs", "HEAD"])
        .wall_timeout(Duration::from_secs(30))
        .idle_timeout(Duration::from_secs(10))
        .run()
        .ok()?;
    if !out.success {
        return None;
    }
    let date = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if date.is_empty() { None } else { Some(date) }
}
