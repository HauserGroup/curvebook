//! Static GitHub Pages documentation site generator (`cargo xtask docs-site`).

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
use crate::scan_util::{collect_files_no_symlinks, is_real_dir, is_regular_file, read_dir_paths_sorted};
use sitemap::{GeneratedSite, assemble_site};
use template::{SiteMeta, render_release_redirect_index, render_root_redirect_shim};
use verify::{parse_minor_dir_version, verify_manual_snapshot_build, verify_site};

const MAX_STATIC_ASSET_BYTES: usize = 32 * 1024 * 1024;
const USAGE: &str = "usage: cargo xtask docs-site [--check] [--output <dir>] [--build-manual [<dir>]] [--manual-dir <dir>] [--release <ver> [--refresh-screenshots]] [--sitemap]";

type SnapshotDir = ((u64, u64), String);

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

pub fn run_cli(root: &Path, args: &[String]) -> Result<(), String> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return Ok(());
    }
    let root_buf = std::env::var_os("CURVEBOOK_DOCS_SITE_ROOT").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| root.to_path_buf());
    let (root, opts) = (root_buf.as_path(), parse_args(args)?);
    if opts.sitemap {
        if opts.check || opts.output.is_some() || opts.build_manual || opts.manual_dir.is_some() || opts.release.is_some() || opts.refresh_screenshots
        {
            return Err(format!("`--sitemap` cannot be combined with other options\n{USAGE}"));
        }
        seo::write_not_found_page(root)?;
        let manual_dir = crate::pages_site::default_manual_dir(root);
        let built = build_manual_into(root, &workspace_version(root), &manual_dir, "workspace", false, false)?;
        println!(
            "docs-site: wrote {} and refreshed {} manual pages + _root/{{sitemap.xml,llms.txt,llms-full.txt}} in {}",
            seo::NOT_FOUND_REL_PATH,
            built.site.html_pages.len(),
            manual_dir.display()
        );
        return Ok(());
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
        let manual_dir = resolve_opt_path(root, opts.manual_dir.or(opts.output)).unwrap_or_else(|| crate::pages_site::default_manual_dir(root));
        return run_release_snapshot(root, version, &manual_dir, opts.check);
    }
    if opts.refresh_screenshots {
        return Err(format!("`--refresh-screenshots` requires `--release <ver>`\n{USAGE}"));
    }
    if opts.build_manual {
        if opts.output.is_some() && opts.manual_dir.is_some() {
            return Err(format!("specify only one of `--build-manual <dir>`, `--manual-dir <dir>`, or `--output <dir>`\n{USAGE}"));
        }
        let (manual_dir, version) = (
            resolve_opt_path(root, opts.manual_dir.or(opts.output)).unwrap_or_else(|| crate::pages_site::default_manual_dir(root)),
            workspace_version(root),
        );
        let built = build_manual_into(root, &version, &manual_dir, "workspace", false, opts.check)?;
        if opts.check {
            verify_manual_snapshot_build(root, &manual_dir, if built.is_newest { "latest" } else { &built.minor_dir }, &built.site)?;
        }
        println!("docs-site: built manual HTML v{version} ({} pages) into {}", built.site.html_pages.len(), manual_dir.display());
        return Ok(());
    }
    let (custom_output, out_dir) = (opts.output.is_some(), resolve_opt_path(root, opts.output).unwrap_or_else(|| root.join("target/docs-site")));
    if opts.check {
        run_check_into(root, &out_dir, resolve_opt_path(root, opts.manual_dir).as_deref(), custom_output)
    } else if opts.manual_dir.is_some() {
        Err(format!("`--manual-dir` requires `--check`, `--build-manual`, or `--release`\n{USAGE}"))
    } else {
        let site = assemble_site(root, preview_meta(root))?;
        write_preview_site(root, &out_dir, &site)?;
        println!("docs-site: generated {} HTML pages into {}", site.html_pages.len(), out_dir.display());
        Ok(())
    }
}

pub fn run_check(root: &Path) -> Result<(), String> {
    run_check_into(root, &root.join("target/docs-site"), None, false)
}

fn parse_args(args: &[String]) -> Result<CliOptions, String> {
    let (mut opts, mut idx) = (CliOptions::default(), 0usize);
    let set_dir = |slot: &mut Option<PathBuf>, flag: &str, val: &str| {
        if val.trim().is_empty() || val.starts_with('-') || slot.replace(PathBuf::from(val)).is_some() {
            Err(format!("{flag} requires a single directory path\n{USAGE}"))
        } else {
            Ok(())
        }
    };
    while let Some(arg) = args.get(idx).map(String::as_str) {
        let (flag, eq_val) = arg.split_once('=').map_or((arg, None), |(f, v)| (f, Some(v)));
        match flag {
            "--check" if eq_val.is_none() => opts.check = true,
            "--sitemap" if eq_val.is_none() => opts.sitemap = true,
            "--refresh-screenshots" if eq_val.is_none() => opts.refresh_screenshots = true,
            "--build-manual" => {
                opts.build_manual = true;
                if let Some(v) = eq_val {
                    set_dir(&mut opts.manual_dir, "--build-manual=", v)?;
                } else if let Some(next) = args.get(idx + 1).filter(|v| !v.is_empty() && !v.starts_with('-')) {
                    set_dir(&mut opts.manual_dir, "--build-manual", next)?;
                    idx += 1;
                }
            }
            "--manual-dir" | "--output" => {
                let val = match eq_val {
                    Some(v) => v,
                    None => {
                        idx += 1;
                        args.get(idx).map(String::as_str).ok_or_else(|| format!("{flag} requires a directory path\n{USAGE}"))?
                    }
                };
                set_dir(if flag == "--output" { &mut opts.output } else { &mut opts.manual_dir }, flag, val)?;
            }
            "--release" => {
                let val = match eq_val {
                    Some(v) => v,
                    None => {
                        idx += 1;
                        args.get(idx).map(String::as_str).ok_or_else(|| format!("--release requires a semantic version\n{USAGE}"))?
                    }
                };
                if val.trim().is_empty() || val.starts_with('-') || opts.release.replace(val.to_owned()).is_some() {
                    return Err(format!("--release requires a single semantic version\n{USAGE}"));
                }
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

fn workspace_version(root: &Path) -> String {
    std::fs::read_to_string(root.join("Cargo.toml"))
        .ok()
        .and_then(|t| crate::version::read_workspace_version(&t).ok())
        .unwrap_or_else(|| String::from(env!("CARGO_PKG_VERSION")))
}

fn preview_meta(root: &Path) -> SiteMeta {
    SiteMeta {
        version_label: format!("v{} (local preview)", workspace_version(root)),
        commit_sha: git_head_query(root, &["rev-parse", "HEAD"]).filter(|s| s.len() == 40),
        release_date: git_head_query(root, &["show", "-s", "--format=%cs", "HEAD"]),
        is_release_snapshot: false,
        is_tagged_release: false,
    }
}

fn reject_output_inside_docs(root: &Path, target_dir: &Path, label: &str) -> Result<(), String> {
    if std::fs::symlink_metadata(target_dir).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(format!("refusing to write {label} through symlinked directory `{}`", target_dir.display()));
    }
    let (norm_target, norm_docs) = (resolve_path_for_containment(target_dir)?, resolve_path_for_containment(&root.join("docs"))?);
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

fn run_check_into(root: &Path, out_dir: &Path, explicit_manual_dir: Option<&Path>, custom_output: bool) -> Result<(), String> {
    reject_output_inside_docs(root, out_dir, "docs-site preview")?;
    let p_meta = preview_meta(root);
    let site_a = assemble_site(root, p_meta.clone())?;
    if site_a.html_pages != assemble_site(root, p_meta)?.html_pages {
        return Err(String::from("docs-site generation is non-deterministic across consecutive passes"));
    }
    write_preview_site(root, out_dir, &site_a)?;
    if let Some(manual_dir) = explicit_manual_dir {
        verify_site(root, out_dir, &site_a, manual_dir, None)?;
    } else {
        let (manual_dir, _cleanup) = if custom_output {
            let dir = root.join("target").join(format!(
                ".docs-manual-check-{}-{}",
                std::process::id(),
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()
            ));
            (dir.clone(), TempDirCleanup(Some(dir)))
        } else {
            (crate::pages_site::default_manual_dir(root), TempDirCleanup(None))
        };
        let built = build_manual_into(root, &workspace_version(root), &manual_dir, "workspace", false, true)?;
        verify_site(root, out_dir, &site_a, &manual_dir, Some(&built.site))?;
    }
    println!(
        "docs-site: verified {} pages, 100% registry coverage, zero broken links/fragments, CSP/privacy, determinism, and compliance gates",
        site_a.html_pages.len()
    );
    Ok(())
}

fn run_release_snapshot(root: &Path, version_raw: &str, manual_dir: &Path, check_after: bool) -> Result<(), String> {
    let clean_ver = version_raw.trim().trim_start_matches('v');
    let built = build_manual_into(root, clean_ver, manual_dir, "release", true, check_after)?;
    if built.is_newest {
        let ver_label = format!("v{clean_ver}");
        for (rel, title, desc, target) in [
            (
                "docs/get-started.html",
                "Get Started & Installation",
                "Redirecting to CurveBook Get Started & Installation in the reference manual.",
                "manual/latest/get-started.html",
            ),
            (
                "docs/troubleshooting.html",
                "Troubleshooting & Known Issues",
                "Redirecting to CurveBook Troubleshooting & Known Issues in the reference manual.",
                "manual/latest/troubleshooting.html#known-issues",
            ),
        ] {
            write_atomic(&root.join(rel), render_root_redirect_shim(title, desc, target, &ver_label).as_bytes())
                .map_err(|e| format!("write {rel}: {e}"))?;
        }
        seo::write_not_found_page(root)?;
    }
    if check_after {
        verify_manual_snapshot_build(root, manual_dir, if built.is_newest { "latest" } else { &built.minor_dir }, &built.site)?;
    }
    println!(
        "docs-site: wrote release manual v{clean_ver} ({} pages) and _root/{{sitemap.xml,llms.txt,llms-full.txt}} into {}{}",
        built.site.html_pages.len(),
        manual_dir.display(),
        if built.is_newest { " and refreshed committed root shims" } else { " (older minor; preserved latest/ and root shims)" }
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
    let minor_dir = format_minor_dir(version_raw)?;
    let commit = git_head_query(root, &["rev-parse", "HEAD"]).filter(|s| s.len() == 40);
    if require_git_commit && commit.is_none() {
        return Err(String::from("cannot determine git HEAD commit SHA for --release manifest"));
    }
    let release_date = git_head_query(root, &["show", "-s", "--format=%cs", "HEAD"]);
    let meta = SiteMeta {
        version_label: format!("v{clean_ver}"),
        commit_sha: commit.clone(),
        release_date: release_date.clone(),
        is_release_snapshot: true,
        is_tagged_release: source_kind == "release",
    };
    let site = assemble_site(root, meta.clone())?;
    if check_determinism && site.html_pages != assemble_site(root, meta)?.html_pages {
        return Err(String::from("docs-site manual generation is non-deterministic across consecutive passes"));
    }
    let newest_existing = collect_snapshot_dir_versions(manual_dir)?.into_iter().next();
    let is_newest = newest_existing.as_ref().is_none_or(|(ver, _)| (parsed_ver.major, parsed_ver.minor) >= *ver);

    let snap_dir = manual_dir.join(&minor_dir);
    write_html_tree(&snap_dir, &site)?;
    let ec = &site.expected_coverage;
    let manifest = serde_json::json!({
        "schema": "curvebook.docs.manifest.v1", "version": clean_ver, "minor_dir": minor_dir, "source": source_kind,
        "commit": commit, "release_date": release_date, "page_count": site.html_pages.len(),
        "registry_counts": {
            "engine_commands": ec.engine_commands.len(), "ui_commands": ec.ui_commands.len(), "mcp_tools": ec.mcp_tools.len(),
            "mcp_prompts": ec.mcp_prompts.len(), "cli_commands": ec.cli_commands.len(), "analysis_kinds": ec.analysis_kinds.len(),
            "fit_equations": ec.fit_equations.len(), "graph_kinds": ec.graph_kinds.len(), "graph_options": ec.graph_options.len(),
            "table_kinds": ec.table_kinds.len(), "formats": ec.formats.len(), "preference_keys": ec.preference_keys.len()
        }
    });
    let manifest_bytes = format!("{}\n", serde_json::to_string_pretty(&manifest).map_err(|e| format!("serialize manifest.json: {e}"))?);
    write_atomic(&snap_dir.join("manifest.json"), manifest_bytes.as_bytes()).map_err(|e| format!("write manifest.json: {e}"))?;

    let latest_dir = manual_dir.join("latest");
    if is_newest {
        write_html_tree(&latest_dir, &site)?;
        write_atomic(&latest_dir.join("manifest.json"), manifest_bytes.as_bytes()).map_err(|e| format!("write latest/manifest.json: {e}"))?;
    }
    let versions: Vec<String> = collect_snapshot_dir_versions(manual_dir)?.into_iter().map(|(_, n)| n).collect();
    let latest_label = if is_newest {
        format!("v{clean_ver}")
    } else if let Some((_, newest_name)) = newest_existing {
        std::fs::read_to_string(manual_dir.join(&newest_name).join("manifest.json"))
            .ok()
            .and_then(|r| serde_json::from_str::<serde_json::Value>(&r).ok())
            .and_then(|v| v.get("version").and_then(serde_json::Value::as_str).map(|s| format!("v{s}")))
            .unwrap_or(newest_name)
    } else {
        format!("v{clean_ver}")
    };
    let redirect_target = if is_real_dir(&latest_dir) && is_regular_file(&latest_dir.join("manifest.json")) {
        String::from("latest")
    } else {
        versions.first().cloned().unwrap_or_else(|| minor_dir.clone())
    };
    let redirect_path = manual_dir.join("index.html");
    reject_symlink_path(&redirect_path)?;
    write_atomic(&redirect_path, render_release_redirect_index(&redirect_target, &latest_label, &versions).as_bytes())
        .map_err(|e| format!("write {}: {e}", redirect_path.display()))?;

    let generated_root = manual_dir.join(crate::pages_site::MANUAL_ROOT_SUBDIR);
    reject_symlink_path(&generated_root)?;
    seo::write_sitemap_into(root, Some(manual_dir), &generated_root)?;
    llms::write_llms_files_into(root, &generated_root)?;
    if check_determinism && llms::build_llms_full_txt(root)? != llms::build_llms_full_txt(root)? {
        return Err(String::from("docs-site llms-full.txt generation is non-deterministic across consecutive passes"));
    }
    Ok(BuiltManual { site, minor_dir, is_newest })
}

pub fn format_minor_dir(ver: &str) -> Result<String, String> {
    let parsed = semver::Version::parse(ver.trim().trim_start_matches('v')).map_err(|e| format!("invalid semantic version `{ver}`: {e}"))?;
    Ok(format!("v{}.{}", parsed.major, parsed.minor))
}

fn collect_snapshot_dir_versions(manual_dir: &Path) -> Result<Vec<SnapshotDir>, String> {
    let mut dirs = Vec::new();
    if is_real_dir(manual_dir) {
        for path in read_dir_paths_sorted(manual_dir)? {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if is_real_dir(&path)
                && let Some(ver) = parse_minor_dir_version(&name)
                && is_regular_file(&path.join("manifest.json"))
            {
                dirs.push((ver, name));
            }
        }
    }
    dirs.sort_by_key(|a| std::cmp::Reverse(a.0));
    Ok(dirs)
}

fn reject_symlink_path(path: &Path) -> Result<(), String> {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(format!("{}: symlinks are forbidden in docs-site output", path.display()));
    }
    Ok(())
}

fn write_file_safe(dst: &Path, bytes: &[u8]) -> Result<(), String> {
    reject_symlink_path(dst)?;
    if let Some(p) = dst.parent() {
        reject_symlink_path(p)?;
        std::fs::create_dir_all(p).map_err(|e| format!("create {}: {e}", p.display()))?;
    }
    write_atomic(dst, bytes).map_err(|e| format!("write {}: {e}", dst.display()))
}

fn write_preview_site(root: &Path, out_dir: &Path, site: &GeneratedSite) -> Result<(), String> {
    reject_output_inside_docs(root, out_dir, "docs-site preview")?;
    write_html_tree(out_dir, site)?;
    let css_src = root.join("docs/site.css");
    write_file_safe(
        &out_dir.join("site.css"),
        &read_regular_file_bytes(&css_src, MAX_STATIC_ASSET_BYTES).map_err(|e| format!("read {}: {e}", css_src.display()))?,
    )?;
    let (img_src, img_dst) = (root.join("docs/images"), out_dir.join("images"));
    if let Ok(meta) = std::fs::symlink_metadata(&img_src) {
        if meta.file_type().is_symlink() || !meta.file_type().is_dir() {
            return Err(format!("{}: static asset directory must be a regular non-symlink directory", img_src.display()));
        }
        reject_symlink_path(&img_dst)?;
        std::fs::create_dir_all(&img_dst).map_err(|e| format!("create {}: {e}", img_dst.display()))?;
        let mut rels = Vec::new();
        collect_files_no_symlinks(&img_src, &img_src, "symlinks are forbidden in docs/images/", &mut rels)?;
        for rel in rels {
            let src = img_src.join(&rel);
            write_file_safe(
                &img_dst.join(&rel),
                &read_regular_file_bytes(&src, MAX_STATIC_ASSET_BYTES).map_err(|e| format!("read {}: {e}", src.display()))?,
            )?;
        }
    }
    Ok(())
}

fn write_html_tree(out_dir: &Path, site: &GeneratedSite) -> Result<(), String> {
    if let Ok(meta) = std::fs::symlink_metadata(out_dir) {
        if meta.file_type().is_symlink() || !meta.file_type().is_dir() {
            return Err(format!("{}: docs-site output path must be a regular non-symlink directory", out_dir.display()));
        }
        let mut existing = Vec::new();
        collect_files_no_symlinks(out_dir, out_dir, "symlinks are forbidden in docs-site output", &mut existing)?;
        for old_rel in existing {
            if old_rel.ends_with(".html") || old_rel == "site.css" || old_rel == "manifest.json" || old_rel.starts_with("images/") {
                let target = out_dir.join(&old_rel);
                std::fs::remove_file(&target).map_err(|e| format!("remove stale {}: {e}", target.display()))?;
            }
        }
    }
    std::fs::create_dir_all(out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;
    for (rel, html) in &site.html_pages {
        write_file_safe(&out_dir.join(rel), html.as_bytes())?;
    }
    Ok(())
}

fn git_head_query(root: &Path, args: &[&str]) -> Option<String> {
    crate::dev_tool_command("git")
        .current_dir(root)
        .args(args)
        .wall_timeout(Duration::from_secs(30))
        .idle_timeout(Duration::from_secs(10))
        .run()
        .ok()
        .filter(|o| o.success)
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::scan_util::test_support::TestDir;

    #[test]
    fn semver_flags_docs_guard_symlinks_and_nested_images() {
        assert_eq!((format_minor_dir("0.1.0-alpha.2").unwrap(), format_minor_dir("v1.2.3").unwrap()), (String::from("v0.1"), String::from("v1.2")));
        for bad in ["0.1.garbage", "+0.+1.0", "0.1-../../evil", "0.1", ""] {
            assert!(format_minor_dir(bad).is_err());
        }
        for bad_args in [vec!["--output", ""], vec!["--output", "--check"], vec!["--release", ""], vec!["--output="], vec!["--release="]] {
            assert!(parse_args(&bad_args.into_iter().map(String::from).collect::<Vec<_>>()).is_err());
        }
        let root = TestDir::new("docs-mod-guard");
        root.write("docs/site.css", "body{}\n");
        root.write("docs/images/nested/icon.png", "png");
        for bad in [root.join("docs"), root.join("docs/sub"), root.join("target/../docs"), root.path().to_path_buf()] {
            assert!(reject_output_inside_docs(root.path(), &bad, "preview").is_err());
        }
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
        }
    }
}
