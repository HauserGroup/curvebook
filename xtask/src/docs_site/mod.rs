//! Static GitHub Pages documentation site generator (`cargo xtask docs-site`).
//!
//! Renders hand-written reference guides (`docs/reference/*.md`, `docs/reference/automation/*.md`,
//! `docs/mcp-for-beginners.md`), direct GitHub contributor links, and live typed Rust
//! registries (`cmd::registry`, `UI_COMMANDS`, `curvebook-mcp`, `cli_catalog`, `curvebook-fit`,
//! `curvebook-doc`, `curvebook-palette`, `curvebook-format`, `docs/schemas/*.schema.json`)
//! into a deterministic, script-free Classic coral HTML manual.

#![allow(dead_code)]

pub mod markdown;
pub mod registries;
pub mod sitemap;
pub mod template;
pub mod verify;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use curvebook_platform::write_atomic;

use sitemap::{GeneratedSite, assemble_site};
use template::{SiteMeta, render_release_redirect_index};
use verify::{parse_minor_dir_version, verify_site};

const USAGE: &str = "usage: cargo xtask docs-site [--check] [--output <dir>] [--release <ver>]";

#[derive(Debug, Default)]
struct CliOptions {
    check: bool,
    output: Option<PathBuf>,
    release: Option<String>,
}

/// CLI entrypoint for `cargo xtask docs-site [--check] [--output <dir>] [--release <ver>]`.
pub fn run_cli(root: &Path, args: &[String]) -> Result<(), String> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return Ok(());
    }
    let root_buf = std::env::var_os("CURVEBOOK_DOCS_SITE_ROOT").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| root.to_path_buf());
    let root = root_buf.as_path();
    let opts = parse_args(args)?;
    if let Some(ref version) = opts.release {
        if opts.output.is_some() {
            return Err(format!("`--release` and `--output` are mutually exclusive\n{USAGE}"));
        }
        return run_release_snapshot(root, version, opts.check);
    }
    let out_dir = opts.output.unwrap_or_else(|| default_output_dir(root));
    if opts.check {
        run_check_into(root, &out_dir)
    } else {
        let meta = SiteMeta {
            version_label: format!("v{} (local preview)", env!("CARGO_PKG_VERSION")),
            commit_sha: git_head_sha(root),
            release_date: git_head_date(root),
            is_release_snapshot: false,
        };
        let site = assemble_site(root, meta)?;
        write_preview_site(root, &out_dir, &site)?;
        println!("docs-site: generated {} HTML pages into {}", site.html_pages.len(), out_dir.display());
        Ok(())
    }
}

/// Gate entrypoint called from `cargo xtask ci` (`policy` stage).
pub fn run_check(root: &Path) -> Result<(), String> {
    let out_dir = default_output_dir(root);
    run_check_into(root, &out_dir)
}

fn parse_args(args: &[String]) -> Result<CliOptions, String> {
    let mut opts = CliOptions::default();
    let mut idx = 0usize;
    while idx < args.len() {
        let arg = args[idx].as_str();
        if arg == "--check" {
            opts.check = true;
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

fn default_output_dir(root: &Path) -> PathBuf {
    root.join("target/docs-site")
}

fn run_check_into(root: &Path, out_dir: &Path) -> Result<(), String> {
    let meta = SiteMeta {
        version_label: format!("v{} (local preview)", env!("CARGO_PKG_VERSION")),
        commit_sha: git_head_sha(root),
        release_date: git_head_date(root),
        is_release_snapshot: false,
    };
    let site_a = assemble_site(root, meta.clone())?;
    let site_b = assemble_site(root, meta)?;
    if site_a.html_pages != site_b.html_pages {
        return Err(String::from("docs-site generation is non-deterministic across consecutive passes"));
    }

    write_preview_site(root, out_dir, &site_a)?;
    verify_site(root, out_dir, &site_a)?;
    println!(
        "docs-site: verified {} pages, 100% registry coverage, zero broken links/fragments, CSP/privacy, determinism, and compliance gates",
        site_a.html_pages.len()
    );
    Ok(())
}

fn run_release_snapshot(root: &Path, version_raw: &str, check_after: bool) -> Result<(), String> {
    let clean_ver = version_raw.trim().trim_start_matches('v');
    let minor_dir = format_minor_dir(clean_ver)?;
    let commit = git_head_sha(root).ok_or_else(|| String::from("cannot determine git HEAD commit SHA for --release manifest"))?;
    let release_date = git_head_date(root);

    let meta = SiteMeta {
        version_label: format!("v{clean_ver}"),
        commit_sha: Some(commit.clone()),
        release_date: release_date.clone(),
        is_release_snapshot: true,
    };
    let site = assemble_site(root, meta)?;

    let snap_dir = root.join("docs/manual").join(&minor_dir);
    write_html_tree(&snap_dir, &site)?;

    let manifest = serde_json::json!({
        "schema": "curvebook.docs.manifest.v1",
        "version": clean_ver,
        "minor_dir": minor_dir,
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
    let manifest_path = snap_dir.join("manifest.json");
    write_atomic(&manifest_path, format!("{manifest_str}\n").as_bytes()).map_err(|e| format!("write {}: {e}", manifest_path.display()))?;

    let versions = collect_snapshot_dirs(&root.join("docs/manual"))?;
    let redirect_html = render_release_redirect_index(&minor_dir, &format!("v{clean_ver}"), &versions);
    let redirect_path = root.join("docs/manual/index.html");
    write_atomic(&redirect_path, redirect_html.as_bytes()).map_err(|e| format!("write docs/manual/index.html: {e}"))?;

    if check_after {
        verify_site(root, &snap_dir, &site)?;
    }
    println!("docs-site: wrote release snapshot v{clean_ver} ({} pages) into {}", site.html_pages.len(), snap_dir.display());
    Ok(())
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
