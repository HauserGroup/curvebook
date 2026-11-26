//! Static GitHub Pages documentation site generator (`cargo xtask docs-site`).
//!
//! Renders hand-written reference guides (`docs/reference/*.md`, `docs/reference/automation/*.md`,
//! `docs/mcp-for-beginners.md`), direct GitHub contributor links, and live typed Rust
//! registries (`cmd::registry`, `UI_COMMANDS`, `curvebook-mcp`, `cli_catalog`, `curvebook-fit`,
//! `curvebook-doc`, `curvebook-palette`, `curvebook-format`, `docs/schemas/*.schema.json`)
//! into a deterministic, script-free Classic coral HTML manual.

pub mod llms;
pub mod markdown;
pub mod registries;
pub mod seo;
pub mod sitemap;
pub mod template;
pub mod verify;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use curvebook_platform::{read_regular_file_bytes, write_atomic};

use crate::pages_site::resolve_path_for_containment;
use crate::scan_util::{is_real_dir, is_regular_file, read_dir_paths_sorted, rel_slash};
use sitemap::{GeneratedSite, assemble_site};
use template::{SiteMeta, render_release_redirect_index, render_root_redirect_shim};
use verify::{parse_minor_dir_version, verify_manual_snapshot_build, verify_site};

const MAX_STATIC_ASSET_BYTES: usize = 32 * 1024 * 1024;

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
    let take_next = |idx: &mut usize, msg: &str| -> Result<&str, String> {
        *idx += 1;
        args.get(*idx).map(String::as_str).ok_or_else(|| format!("{msg}\n{USAGE}"))
    };
    while idx < args.len() {
        let arg = args[idx].as_str();
        match arg {
            "--check" => opts.check = true,
            "--sitemap" => opts.sitemap = true,
            "--refresh-screenshots" => opts.refresh_screenshots = true,
            "--build-manual" => {
                opts.build_manual = true;
                if let Some(next) = args.get(idx + 1).filter(|v| !v.is_empty() && !v.starts_with('-')) {
                    if opts.manual_dir.is_some() {
                        return Err(format!("duplicate manual directory argument\n{USAGE}"));
                    }
                    opts.manual_dir = Some(PathBuf::from(next));
                    idx += 1;
                }
            }
            "--manual-dir" => {
                let val = take_next(&mut idx, "--manual-dir requires a directory path")?;
                if val.is_empty() || val.starts_with('-') {
                    return Err(format!("--manual-dir requires a directory path\n{USAGE}"));
                }
                if opts.manual_dir.is_some() {
                    return Err(format!("duplicate `--manual-dir` argument\n{USAGE}"));
                }
                opts.manual_dir = Some(PathBuf::from(val));
            }
            "--output" => {
                let val = take_next(&mut idx, "--output requires a directory path")?;
                if val.trim().is_empty() || val.starts_with('-') {
                    return Err(format!("--output requires a directory path\n{USAGE}"));
                }
                if opts.output.is_some() {
                    return Err(format!("duplicate `--output` argument\n{USAGE}"));
                }
                opts.output = Some(PathBuf::from(val));
            }
            "--release" => {
                let val = take_next(&mut idx, "--release requires a semantic version")?;
                if val.trim().is_empty() || val.starts_with('-') {
                    return Err(format!("--release requires a semantic version\n{USAGE}"));
                }
                if opts.release.is_some() {
                    return Err(format!("duplicate `--release` argument\n{USAGE}"));
                }
                opts.release = Some(val.to_owned());
            }
            _ if arg.starts_with("--build-manual=") => {
                let val = arg.trim_start_matches("--build-manual=");
                if val.trim().is_empty() || opts.manual_dir.is_some() {
                    return Err(format!("--build-manual= requires a single directory path\n{USAGE}"));
                }
                opts.build_manual = true;
                opts.manual_dir = Some(PathBuf::from(val));
            }
            _ if arg.starts_with("--manual-dir=") => {
                let val = arg.trim_start_matches("--manual-dir=");
                if val.trim().is_empty() || opts.manual_dir.is_some() {
                    return Err(format!("--manual-dir= requires a single directory path\n{USAGE}"));
                }
                opts.manual_dir = Some(PathBuf::from(val));
            }
            _ if arg.starts_with("--output=") => {
                let val = arg.trim_start_matches("--output=");
                if val.trim().is_empty() || val.starts_with('-') || opts.output.is_some() {
                    return Err(format!("--output= requires a single directory path\n{USAGE}"));
                }
                opts.output = Some(PathBuf::from(val));
            }
            _ if arg.starts_with("--release=") => {
                let val = arg.trim_start_matches("--release=");
                if val.trim().is_empty() || val.starts_with('-') || opts.release.is_some() {
                    return Err(format!("--release= requires a single semantic version\n{USAGE}"));
                }
                opts.release = Some(val.to_owned());
            }
            _ => return Err(format!("unknown argument `{arg}`\n{USAGE}")),
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
        is_tagged_release: false,
    }
}

fn reject_output_inside_docs(root: &Path, target_dir: &Path, label: &str) -> Result<(), String> {
    if let Ok(meta) = std::fs::symlink_metadata(target_dir)
        && meta.file_type().is_symlink()
    {
        return Err(format!("refusing to write {label} through symlinked directory `{}`", target_dir.display()));
    }
    let docs = root.join("docs");
    let norm_target = resolve_path_for_containment(target_dir)?;
    let norm_docs = resolve_path_for_containment(&docs)?;
    if norm_target.starts_with(&norm_docs) || norm_docs.starts_with(&norm_target) {
        return Err(format!("refusing to write {label} into `{}` (output directory must be strictly outside `docs/`)", target_dir.display()));
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
    let built = build_manual_into(root, &version, &manual_dir, "workspace", false, false)?;
    println!(
        "docs-site: wrote {} and refreshed {} manual pages + _root/{{sitemap.xml,llms.txt,llms-full.txt}} in {}",
        seo::NOT_FOUND_REL_PATH,
        built.site.html_pages.len(),
        manual_dir.display()
    );
    Ok(())
}

fn run_check_into(root: &Path, out_dir: &Path, explicit_manual_dir: Option<&Path>, custom_output: bool) -> Result<(), String> {
    reject_output_inside_docs(root, out_dir, "docs-site preview")?;
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
        let built = build_manual_into(root, &version, &manual_dir, "workspace", false, true)?;
        verify_site(root, out_dir, &site_a, &manual_dir, Some(&built.site))?;
    }

    println!(
        "docs-site: verified {} pages, 100% registry coverage, zero broken links/fragments, CSP/privacy, determinism, and compliance gates",
        site_a.html_pages.len()
    );
    Ok(())
}

fn run_build_manual(root: &Path, manual_dir: &Path, check_after: bool) -> Result<(), String> {
    let version = workspace_version(root);
    let built = build_manual_into(root, &version, manual_dir, "workspace", false, check_after)?;
    if check_after {
        let snap_sub = if built.is_newest { "latest" } else { built.minor_dir.as_str() };
        verify_manual_snapshot_build(root, manual_dir, snap_sub, &built.site)?;
    }
    println!("docs-site: built manual HTML v{version} ({} pages) into {}", built.site.html_pages.len(), manual_dir.display());
    Ok(())
}

fn run_release_snapshot(root: &Path, version_raw: &str, manual_dir: &Path, check_after: bool) -> Result<(), String> {
    let clean_ver = version_raw.trim().trim_start_matches('v');
    let built = build_manual_into(root, clean_ver, manual_dir, "release", true, check_after)?;
    if built.is_newest {
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
    }

    if check_after {
        let snap_sub = if built.is_newest { "latest" } else { built.minor_dir.as_str() };
        verify_manual_snapshot_build(root, manual_dir, snap_sub, &built.site)?;
    }
    println!(
        "docs-site: wrote release manual v{clean_ver} ({} pages) and _root/{{sitemap.xml,llms.txt,llms-full.txt}} into {}{}",
        built.site.html_pages.len(),
        manual_dir.display(),
        if built.is_newest { " and refreshed committed root shims" } else { " (older minor; preserved latest/ and root shims)" },
    );
    Ok(())
}

struct BuiltManual {
    site: GeneratedSite,
    minor_dir: String,
    is_newest: bool,
}

fn build_manual_into(
    root: &Path,
    version_raw: &str,
    manual_dir: &Path,
    source_kind: &str,
    require_git_commit: bool,
    check_determinism: bool,
) -> Result<BuiltManual, String> {
    reject_output_inside_docs(root, manual_dir, "generated manual HTML")?;
    let clean_ver = version_raw.trim().trim_start_matches('v');
    let parsed_ver = semver::Version::parse(clean_ver).map_err(|e| format!("invalid semantic version `{version_raw}`: {e}"))?;
    let minor_dir = format_minor_dir(clean_ver)?;
    let this_minor = (parsed_ver.major, parsed_ver.minor);
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
        is_tagged_release: source_kind == "release",
    };
    let site = assemble_site(root, meta.clone())?;
    if check_determinism {
        let second = assemble_site(root, meta)?;
        if site.html_pages != second.html_pages {
            return Err(String::from("docs-site manual generation is non-deterministic across consecutive passes"));
        }
    }

    let existing_snaps = collect_snapshot_dir_versions(manual_dir)?;
    let newest_existing = existing_snaps.first().cloned();
    let is_newest = newest_existing.as_ref().is_none_or(|(ver, _)| this_minor >= *ver);

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
    if is_newest {
        write_html_tree(&latest_dir, &site)?;
        let latest_manifest_path = latest_dir.join("manifest.json");
        write_atomic(&latest_manifest_path, manifest_bytes.as_bytes()).map_err(|e| format!("write {}: {e}", latest_manifest_path.display()))?;
    }

    let versions = collect_snapshot_dirs(manual_dir)?;
    let latest_label = if is_newest {
        format!("v{clean_ver}")
    } else if let Some((_, newest_name)) = newest_existing {
        read_manifest_version_label(&manual_dir.join(&newest_name).join("manifest.json")).unwrap_or(newest_name)
    } else {
        format!("v{clean_ver}")
    };
    let redirect_target = if is_real_dir(&latest_dir) && is_regular_file(&latest_dir.join("manifest.json")) {
        String::from("latest")
    } else {
        versions.first().cloned().unwrap_or_else(|| minor_dir.clone())
    };
    let redirect_html = render_release_redirect_index(&redirect_target, &latest_label, &versions);
    let redirect_path = manual_dir.join("index.html");
    reject_symlink_path(&redirect_path)?;
    write_atomic(&redirect_path, redirect_html.as_bytes()).map_err(|e| format!("write {}: {e}", redirect_path.display()))?;

    let generated_root = manual_dir.join(crate::pages_site::MANUAL_ROOT_SUBDIR);
    reject_symlink_path(&generated_root)?;
    seo::write_sitemap_into(root, Some(manual_dir), &generated_root)?;
    llms::write_llms_files_into(root, &generated_root)?;
    if check_determinism {
        let llms_full_a = llms::build_llms_full_txt(root)?;
        let llms_full_b = llms::build_llms_full_txt(root)?;
        if llms_full_a != llms_full_b {
            return Err(String::from("docs-site llms-full.txt generation is non-deterministic across consecutive passes"));
        }
    }

    Ok(BuiltManual { site, minor_dir, is_newest })
}

fn read_manifest_version_label(manifest_path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(manifest_path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let ver = val.get("version").and_then(serde_json::Value::as_str)?;
    Some(format!("v{ver}"))
}

/// Convert a valid SemVer 2.0.0 string such as `"0.1.0-alpha.2"` into `"v0.1"`.
pub fn format_minor_dir(ver: &str) -> Result<String, String> {
    let clean = ver.trim().trim_start_matches('v');
    let parsed = semver::Version::parse(clean).map_err(|e| format!("invalid semantic version `{ver}`: {e}"))?;
    Ok(format!("v{}.{}", parsed.major, parsed.minor))
}

type SnapshotDirVersion = ((u64, u64), String);

fn collect_snapshot_dir_versions(manual_dir: &Path) -> Result<Vec<SnapshotDirVersion>, String> {
    let mut dirs: Vec<SnapshotDirVersion> = Vec::new();
    if is_real_dir(manual_dir) {
        for path in read_dir_paths_sorted(manual_dir)? {
            if !is_real_dir(&path) {
                continue;
            }
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if let Some(ver) = parse_minor_dir_version(&name)
                && is_regular_file(&path.join("manifest.json"))
            {
                dirs.push((ver, name));
            }
        }
    }
    dirs.sort_by_key(|a| std::cmp::Reverse(a.0));
    Ok(dirs)
}

fn collect_snapshot_dirs(manual_dir: &Path) -> Result<Vec<String>, String> {
    Ok(collect_snapshot_dir_versions(manual_dir)?.into_iter().map(|(_, n)| n).collect())
}

fn reject_symlink_path(path: &Path) -> Result<(), String> {
    if let Ok(meta) = std::fs::symlink_metadata(path)
        && meta.file_type().is_symlink()
    {
        return Err(format!("{}: symlinks are forbidden in docs-site output", path.display()));
    }
    Ok(())
}

fn collect_existing_files_no_symlinks(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    for path in read_dir_paths_sorted(dir)? {
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("stat {}: {e}", path.display()))?;
        if meta.file_type().is_symlink() {
            return Err(format!("{}: symlinks are forbidden in docs-site output", path.display()));
        }
        if meta.file_type().is_dir() {
            collect_existing_files_no_symlinks(root, &path, out)?;
        } else if meta.file_type().is_file() {
            out.push(rel_slash(root, &path)?);
        }
    }
    Ok(())
}

fn copy_regular_tree_atomic(src_root: &Path, src_dir: &Path, dst_root: &Path) -> Result<(), String> {
    let dir_meta = std::fs::symlink_metadata(src_dir).map_err(|e| format!("stat {}: {e}", src_dir.display()))?;
    if dir_meta.file_type().is_symlink() || !dir_meta.file_type().is_dir() {
        return Err(format!("{}: static asset directory must be a regular non-symlink directory", src_dir.display()));
    }
    for src in read_dir_paths_sorted(src_dir)? {
        let meta = std::fs::symlink_metadata(&src).map_err(|e| format!("stat {}: {e}", src.display()))?;
        if meta.file_type().is_symlink() {
            return Err(format!("{}: symlinks are forbidden in docs/images/", src.display()));
        }
        if meta.file_type().is_dir() {
            copy_regular_tree_atomic(src_root, &src, dst_root)?;
        } else if meta.file_type().is_file() {
            let rel = rel_slash(src_root, &src)?;
            let dst = dst_root.join(&rel);
            reject_symlink_path(&dst)?;
            if let Some(parent) = dst.parent() {
                reject_symlink_path(parent)?;
                std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
            }
            let bytes = read_regular_file_bytes(&src, MAX_STATIC_ASSET_BYTES).map_err(|e| format!("read {}: {e}", src.display()))?;
            write_atomic(&dst, &bytes).map_err(|e| format!("write {}: {e}", dst.display()))?;
        }
    }
    Ok(())
}

fn write_preview_site(root: &Path, out_dir: &Path, site: &GeneratedSite) -> Result<(), String> {
    reject_output_inside_docs(root, out_dir, "docs-site preview")?;
    write_html_tree(out_dir, site)?;

    let css_src = root.join("docs/site.css");
    let css_dst = out_dir.join("site.css");
    reject_symlink_path(&css_dst)?;
    let css_bytes = read_regular_file_bytes(&css_src, MAX_STATIC_ASSET_BYTES).map_err(|e| format!("read {}: {e}", css_src.display()))?;
    write_atomic(&css_dst, &css_bytes).map_err(|e| format!("write {}: {e}", css_dst.display()))?;

    let img_src_dir = root.join("docs/images");
    let img_dst_dir = out_dir.join("images");
    if std::fs::symlink_metadata(&img_src_dir).is_ok() {
        reject_symlink_path(&img_dst_dir)?;
        std::fs::create_dir_all(&img_dst_dir).map_err(|e| format!("create {}: {e}", img_dst_dir.display()))?;
        copy_regular_tree_atomic(&img_src_dir, &img_src_dir, &img_dst_dir)?;
    }
    Ok(())
}

fn write_html_tree(out_dir: &Path, site: &GeneratedSite) -> Result<(), String> {
    if let Ok(meta) = std::fs::symlink_metadata(out_dir) {
        if meta.file_type().is_symlink() || !meta.file_type().is_dir() {
            return Err(format!("{}: docs-site output path must be a regular non-symlink directory", out_dir.display()));
        }
        let mut existing = Vec::new();
        collect_existing_files_no_symlinks(out_dir, out_dir, &mut existing)?;
        for old_rel in existing {
            if old_rel.ends_with(".html") || old_rel == "site.css" || old_rel == "manifest.json" || old_rel.starts_with("images/") {
                let target = out_dir.join(&old_rel);
                std::fs::remove_file(&target).map_err(|e| format!("remove stale {}: {e}", target.display()))?;
            }
        }
    }
    std::fs::create_dir_all(out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;
    for (rel, html) in &site.html_pages {
        let dst = out_dir.join(rel);
        reject_symlink_path(&dst)?;
        if let Some(parent) = dst.parent() {
            reject_symlink_path(parent)?;
            std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        write_atomic(&dst, html.as_bytes()).map_err(|e| format!("write {}: {e}", dst.display()))?;
    }
    Ok(())
}

fn git_head_query(root: &Path, args: &[&str]) -> Option<String> {
    let out = crate::dev_tool_command("git")
        .current_dir(root)
        .args(args)
        .wall_timeout(Duration::from_secs(30))
        .idle_timeout(Duration::from_secs(10))
        .run()
        .ok()?;
    out.success.then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned()).filter(|s| !s.is_empty())
}

fn git_head_sha(root: &Path) -> Option<String> {
    git_head_query(root, &["rev-parse", "HEAD"]).filter(|s| s.len() == 40)
}

fn git_head_date(root: &Path) -> Option<String> {
    git_head_query(root, &["show", "-s", "--format=%cs", "HEAD"])
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{GeneratedSite, SiteMeta, format_minor_dir, parse_args, reject_output_inside_docs, write_html_tree, write_preview_site};
    use crate::scan_util::test_support::TestDir;

    #[test]
    fn aud_12_25_semver_flags_docs_guard_symlinks_and_nested_images() {
        // 1. Strict SemVer validation in format_minor_dir.
        assert_eq!(format_minor_dir("0.1.0-alpha.2").unwrap(), "v0.1");
        assert_eq!(format_minor_dir("v1.2.3").unwrap(), "v1.2");
        for bad in ["0.1.garbage", "+0.+1.0", "0.1-../../evil", "0.1", ""] {
            assert!(format_minor_dir(bad).is_err(), "expected SemVer rejection for `{bad}`");
        }

        // 2. Strict flag parsing rejects empty / flag-like values for --output and --release.
        for bad_args in [
            vec![String::from("--output"), String::from("")],
            vec![String::from("--output"), String::from("--check")],
            vec![String::from("--release"), String::from("")],
            vec![String::from("--release"), String::from("--check")],
            vec![String::from("--output=")],
            vec![String::from("--release=")],
        ] {
            assert!(parse_args(&bad_args).is_err(), "expected parse_args error for {bad_args:?}");
        }

        // 3. Containment guard refuses docs/, docs/sub, target/../docs, and root itself.
        let root = TestDir::new("docs-mod-aud-12-25");
        root.write("docs/site.css", "body{}\n");
        root.write("docs/images/nested/icon.png", "png");
        assert!(reject_output_inside_docs(root.path(), &root.join("docs"), "preview").is_err());
        assert!(reject_output_inside_docs(root.path(), &root.join("docs/sub"), "preview").is_err());
        assert!(reject_output_inside_docs(root.path(), &root.join("target/../docs"), "preview").is_err());
        assert!(reject_output_inside_docs(root.path(), root.path(), "preview").is_err());
        assert!(reject_output_inside_docs(root.path(), &root.join("target/docs-site"), "preview").is_ok());

        // 4. write_preview_site copies nested docs/images/ subdirectories and rejects symlinks.
        let site = GeneratedSite {
            html_pages: BTreeMap::from([(String::from("index.html"), String::from("<h1>Ok</h1>"))]),
            expected_coverage: Default::default(),
            meta: SiteMeta {
                version_label: String::from("v0.1.0"),
                commit_sha: None,
                release_date: None,
                is_release_snapshot: false,
                is_tagged_release: false,
            },
        };
        let out_dir = root.join("target/preview");
        write_preview_site(root.path(), &out_dir, &site).unwrap();
        assert!(out_dir.join("images/nested/icon.png").is_file());

        #[cfg(unix)]
        {
            let secret = root.write("outside.txt", "keep");
            let sym_out = root.join("target/sym-out");
            std::fs::create_dir_all(&sym_out).unwrap();
            std::os::unix::fs::symlink(&secret, sym_out.join("index.html")).unwrap();
            assert!(write_html_tree(&sym_out, &site).unwrap_err().contains("symlinks are forbidden"));
            assert_eq!(std::fs::read_to_string(&secret).unwrap(), "keep");
        }
    }
}
