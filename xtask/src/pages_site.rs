//! The set of files the GitHub Pages site publishes (`R12.9`, `R12.10`) and `cargo xtask pages-stage`.
//!
//! Pages deploys only the files listed here, combining committed site assets under `docs/` with
//! generated manual HTML built into a build directory (default `target/docs-manual/`, or
//! `--manual-dir <dir>`), so internal Markdown (roadmap, audits, planning, evidence, reference
//! sources, and schemas) is never served next to the site and generated manual HTML stays out of
//! git. The published set is:
//!
//! - the hand-written landing pages in [`LANDING_PAGES`] and the redirect pages in [`REDIRECT_PAGES`];
//! - every file of `manual/latest/` and every active versioned snapshot under
//!   `manual/v<major>.<minor>/` from the manual build directory, except their `manifest.json`;
//! - the shared [`SITE_FILES`] and every file under the [`SITE_DIRS`].
//!
//! Every published HTML page may link only to published files: [`check_publish_set_with_manual`]
//! fails on a local `href`, `src`, or meta-refresh target that the list does not contain, and on
//! any `docs/sitemap.xml` URL outside the list. This module needs no `docs-site` feature
//! dependency, so it continues to build with `--no-default-features`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use curvebook_platform::write_atomic;

/// Published base URL of the GitHub Pages site.
pub const SITE_BASE_URL: &str = "https://hausergroup.github.io/curvebook/";

/// Optional custom domain (`CNAME`) for single-switch cutover; keep `None` while serving on `github.io`.
pub const CUSTOM_DOMAIN: Option<&str> = None;

/// Hand-written top-level pages under `docs/` that search engines should index, in sitemap order.
/// Each must carry a canonical link and Open Graph / Twitter Card metadata (`docs-site --check`).
pub const LANDING_PAGES: &[&str] = &["index.html", "open-pzfx-files.html", "linux.html", "faq.html", "privacy.html"];

/// Published redirect pages that forward to the newest release snapshot (left out of the sitemap).
pub const REDIRECT_PAGES: &[&str] = &["get-started.html", "troubleshooting.html", "manual/index.html"];

/// Other individual published files, relative to `docs/`.
pub const SITE_FILES: &[&str] = &["404.html", "site.css", "sitemap.xml", "llms.txt", "llms-full.txt"];

/// Path prefix of [`SITE_BASE_URL`] (e.g. `"/curvebook/"` on `github.io`, `"/"` on a custom domain).
#[must_use]
pub fn site_base_path() -> &'static str {
    let rest = SITE_BASE_URL.strip_prefix("https://").or_else(|| SITE_BASE_URL.strip_prefix("http://")).unwrap_or(SITE_BASE_URL);
    rest.find('/').and_then(|idx| rest.get(idx..)).unwrap_or("/")
}

/// Directories under `docs/` whose files are all published.
pub const SITE_DIRS: &[&str] = &["images"];

/// Release-snapshot files that stay in the repository but are not published.
const SNAPSHOT_EXCLUDED_FILES: &[&str] = &["manifest.json"];

/// Default build directory holding CI/local generated manual HTML (`index.html`, `latest/`, `v<major>.<minor>/`).
#[must_use]
pub fn default_manual_dir(root: &Path) -> PathBuf {
    root.join("target/docs-manual")
}

const USAGE: &str = "usage: cargo xtask pages-stage [--manual-dir <dir>] (<output-dir> | --list | --check)";

#[derive(Debug, Clone, PartialEq, Eq)]
enum StageAction {
    List,
    Check,
    Help,
    Stage(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StageCli {
    manual_dir: Option<PathBuf>,
    action: StageAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StagedEntry {
    site_rel: String,
    source_path: PathBuf,
}

/// `cargo xtask pages-stage [--manual-dir <dir>] (<output-dir> | --list | --check)`.
pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let cli = parse_args(args)?;
    let manual_dir = cli.manual_dir.as_deref();
    match cli.action {
        StageAction::Help => {
            println!("{USAGE}");
            Ok(())
        }
        StageAction::List => {
            for rel in check_publish_set_with_manual(root, manual_dir)? {
                println!("{rel}");
            }
            Ok(())
        }
        StageAction::Check => {
            let files = check_publish_set_with_manual(root, manual_dir)?;
            println!("pages-stage: {} published files, every local link resolves inside the published set", files.len());
            Ok(())
        }
        StageAction::Stage(out) => {
            let out_dir = if out.is_absolute() { out } else { root.join(out) };
            let count = stage_with_manual(root, manual_dir, &out_dir)?;
            println!("pages-stage: staged {count} files into {}", out_dir.display());
            Ok(())
        }
    }
}

fn parse_args(args: &[String]) -> Result<StageCli, String> {
    let mut manual_dir: Option<PathBuf> = None;
    let mut action: Option<StageAction> = None;
    let mut idx = 0usize;
    while idx < args.len() {
        let arg = args[idx].as_str();
        if arg == "-h" || arg == "--help" {
            return Ok(StageCli { manual_dir: None, action: StageAction::Help });
        }
        if let Some(val) = arg.strip_prefix("--manual-dir=") {
            if val.trim().is_empty() || manual_dir.is_some() {
                return Err(String::from(USAGE));
            }
            manual_dir = Some(PathBuf::from(val));
        } else if arg == "--manual-dir" {
            idx += 1;
            let val = args.get(idx).filter(|v| !v.is_empty() && !v.starts_with('-')).ok_or_else(|| String::from(USAGE))?;
            if manual_dir.is_some() {
                return Err(String::from(USAGE));
            }
            manual_dir = Some(PathBuf::from(val));
        } else {
            let next_action = if arg == "--list" {
                StageAction::List
            } else if arg == "--check" {
                StageAction::Check
            } else if !arg.starts_with('-') {
                StageAction::Stage(PathBuf::from(arg))
            } else {
                return Err(String::from(USAGE));
            };
            if action.is_some() {
                return Err(String::from(USAGE));
            }
            action = Some(next_action);
        }
        idx += 1;
    }
    let action = action.ok_or_else(|| String::from(USAGE))?;
    Ok(StageCli { manual_dir, action })
}

/// Resolve the directory containing generated manual HTML (`index.html`, `latest/`, `v<major>.<minor>/`).
pub fn resolve_manual_dir(root: &Path, manual_dir: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(explicit) = manual_dir {
        let resolved = if explicit.is_absolute() { explicit.to_path_buf() } else { root.join(explicit) };
        if !is_real_dir(&resolved) {
            return Err(format!("manual directory `{}` does not exist (or is not a regular directory)", resolved.display()));
        }
        if !is_regular_file(&resolved.join("index.html")) {
            return Err(format!("manual directory `{}` is missing `index.html`", resolved.display()));
        }
        return Ok(resolved);
    }

    let default_dir = default_manual_dir(root);
    if is_real_dir(&default_dir) && is_regular_file(&default_dir.join("index.html")) {
        return Ok(default_dir);
    }

    let legacy_dir = root.join("docs/manual");
    if is_real_dir(&legacy_dir) && is_regular_file(&legacy_dir.join("index.html")) {
        return Ok(legacy_dir);
    }

    Err(format!(
        "manual HTML is not built (checked `{}` and `docs/manual`); run `cargo run -p xtask --features docs-site -- docs-site --build-manual` first or pass `--manual-dir <dir>`",
        default_dir.display()
    ))
}

/// Every published file, as a sorted `/`-separated path relative to the site root.
#[cfg(test)]
pub fn publish_list(root: &Path) -> Result<Vec<String>, String> {
    Ok(publish_entries(root, None)?.into_iter().map(|e| e.site_rel).collect())
}

fn publish_entries(root: &Path, manual_dir: Option<&Path>) -> Result<Vec<StagedEntry>, String> {
    let docs = root.join("docs");
    let mut entries: BTreeMap<String, PathBuf> = BTreeMap::new();
    for rel in LANDING_PAGES.iter().chain(REDIRECT_PAGES).chain(SITE_FILES) {
        if *rel == "manual/index.html" {
            continue;
        }
        let path = docs.join(rel);
        if !is_regular_file(&path) {
            return Err(format!("docs/{rel} is on the Pages publish list but missing (or not a regular file)"));
        }
        entries.insert((*rel).to_owned(), path);
    }
    for dir in SITE_DIRS {
        collect_entries(&docs, &docs.join(dir), "", &mut entries)?;
    }

    let manual_root = resolve_manual_dir(root, manual_dir)?;
    let manual_index = manual_root.join("index.html");
    if !is_regular_file(&manual_index) {
        return Err(format!("{}/index.html is on the Pages publish list but missing (or not a regular file)", manual_root.display()));
    }
    entries.insert(String::from("manual/index.html"), manual_index);

    let mut manual_page_count = 0usize;
    for snapshot in snapshot_dirs(&manual_root)? {
        let mut snap_entries = BTreeMap::new();
        collect_entries(&manual_root, &manual_root.join(&snapshot), "manual/", &mut snap_entries)?;
        for (rel, src) in snap_entries {
            if !SNAPSHOT_EXCLUDED_FILES.iter().any(|name| rel.rsplit('/').next() == Some(*name)) {
                manual_page_count += 1;
                entries.insert(rel, src);
            }
        }
    }
    let latest_dir = manual_root.join("latest");
    if is_real_dir(&latest_dir) && is_regular_file(&latest_dir.join("manifest.json")) {
        let mut latest_entries = BTreeMap::new();
        collect_entries(&manual_root, &latest_dir, "manual/", &mut latest_entries)?;
        for (rel, src) in latest_entries {
            if !SNAPSHOT_EXCLUDED_FILES.iter().any(|name| rel.rsplit('/').next() == Some(*name)) {
                manual_page_count += 1;
                entries.insert(rel, src);
            }
        }
    }
    if manual_page_count == 0 {
        return Err(format!("manual directory `{}` contains no snapshot pages (`latest/` or `v<major>.<minor>/`)", manual_root.display()));
    }
    if let Some(md) = entries.keys().find(|rel| rel.ends_with(".md")) {
        return Err(format!("{md}: Markdown sources are never published on Pages"));
    }
    Ok(entries.into_iter().map(|(site_rel, source_path)| StagedEntry { site_rel, source_path }).collect())
}

/// Build the publish list and check that published pages and the sitemap reference only published files.
#[cfg(test)]
pub fn check_publish_set(root: &Path) -> Result<Vec<String>, String> {
    check_publish_set_with_manual(root, None)
}

/// Build the publish list using `manual_dir` (or default) and verify every local link and sitemap entry.
pub fn check_publish_set_with_manual(root: &Path, manual_dir: Option<&Path>) -> Result<Vec<String>, String> {
    Ok(check_publish_entries(root, manual_dir)?.into_iter().map(|e| e.site_rel).collect())
}

fn check_publish_entries(root: &Path, manual_dir: Option<&Path>) -> Result<Vec<StagedEntry>, String> {
    let entries = publish_entries(root, manual_dir)?;
    let published: BTreeSet<&str> = entries.iter().map(|e| e.site_rel.as_str()).collect();
    let docs = root.join("docs");
    let mut problems = Vec::new();
    for entry in entries.iter().filter(|e| e.site_rel.ends_with(".html")) {
        let rel = entry.site_rel.as_str();
        let html = std::fs::read_to_string(&entry.source_path).map_err(|e| format!("read {}: {e}", entry.source_path.display()))?;
        for target in local_references(&html) {
            match resolve(rel, &target) {
                Some(resolved) if published.contains(resolved.as_str()) => {}
                Some(resolved) => problems.push(format!("{rel} links to `{target}` ({resolved}), which is not published")),
                None => problems.push(format!("{rel} links to `{target}`, which leaves the site root")),
            }
        }
    }
    let sitemap = std::fs::read_to_string(docs.join("sitemap.xml")).map_err(|e| format!("read docs/sitemap.xml: {e}"))?;
    for loc in sitemap.split("<loc>").skip(1).filter_map(|chunk| chunk.split("</loc>").next()) {
        let Some(rel) = loc.strip_prefix(SITE_BASE_URL) else {
            problems.push(format!("docs/sitemap.xml lists `{loc}` outside `{SITE_BASE_URL}`"));
            continue;
        };
        let rel = if rel.is_empty() { "index.html" } else { rel };
        if !published.contains(rel) {
            problems.push(format!("docs/sitemap.xml lists `{loc}`, which is not published"));
        }
    }
    if problems.is_empty() { Ok(entries) } else { Err(format!("Pages publish set has {} problem(s):\n  {}", problems.len(), problems.join("\n  "))) }
}

/// Copy the checked publish set into `out_dir`, which must be absent or empty and outside `docs/`.
#[cfg(test)]
pub fn stage(root: &Path, out_dir: &Path) -> Result<usize, String> {
    stage_with_manual(root, None, out_dir)
}

/// Copy the checked publish set (combining `docs/` and `manual_dir`) into `out_dir`.
pub fn stage_with_manual(root: &Path, manual_dir: Option<&Path>, out_dir: &Path) -> Result<usize, String> {
    let docs = root.join("docs");
    if out_dir.starts_with(&docs) {
        return Err(format!("staging directory {} must be outside docs/", out_dir.display()));
    }
    let resolved_manual = resolve_manual_dir(root, manual_dir)?;
    if out_dir.starts_with(&resolved_manual) {
        return Err(format!("staging directory {} must be outside manual directory {}", out_dir.display(), resolved_manual.display()));
    }
    if std::fs::symlink_metadata(out_dir).is_ok() {
        let mut entries = std::fs::read_dir(out_dir).map_err(|e| format!("read_dir {}: {e}", out_dir.display()))?;
        if entries.next().is_some() {
            return Err(format!("staging directory {} must be empty", out_dir.display()));
        }
    }
    let entries = check_publish_entries(root, Some(&resolved_manual))?;
    for entry in &entries {
        let bytes = std::fs::read(&entry.source_path).map_err(|e| format!("read {}: {e}", entry.source_path.display()))?;
        let dst = out_dir.join(&entry.site_rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        write_atomic(&dst, &bytes).map_err(|e| format!("write {}: {e}", dst.display()))?;
    }
    let extra = stage_custom_domain_files(out_dir, SITE_BASE_URL, CUSTOM_DOMAIN)?;
    Ok(entries.len() + extra)
}

fn stage_custom_domain_files(out_dir: &Path, site_base_url: &str, custom_domain: Option<&str>) -> Result<usize, String> {
    let Some(domain) = custom_domain else {
        return Ok(0);
    };
    let trimmed = domain.trim();
    let cname = format!("{trimmed}\n");
    write_atomic(&out_dir.join("CNAME"), cname.as_bytes()).map_err(|e| format!("write {}/CNAME: {e}", out_dir.display()))?;
    let trimmed_base = site_base_url.trim_end_matches('/');
    let robots = format!("User-agent: *\nAllow: /\n\nSitemap: {trimmed_base}/sitemap.xml\n");
    write_atomic(&out_dir.join("robots.txt"), robots.as_bytes()).map_err(|e| format!("write {}/robots.txt: {e}", out_dir.display()))?;
    Ok(2)
}

fn is_regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file())
}

fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| !m.file_type().is_symlink() && m.file_type().is_dir())
}

/// `v<major>.<minor>` release-snapshot directories under a manual root that carry a `manifest.json`.
fn snapshot_dirs(manual: &Path) -> Result<Vec<String>, String> {
    if !is_real_dir(manual) {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(manual).map_err(|e| format!("read_dir {}: {e}", manual.display()))? {
        let entry = entry.map_err(|e| format!("dir entry under {}: {e}", manual.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_versioned = name
            .strip_prefix('v')
            .and_then(|core| core.split_once('.'))
            .is_some_and(|(maj, min)| !maj.is_empty() && !min.is_empty() && maj.bytes().chain(min.bytes()).all(|b| b.is_ascii_digit()));
        if is_versioned && entry.file_type().is_ok_and(|t| t.is_dir()) && is_regular_file(&entry.path().join("manifest.json")) {
            out.push(name);
        }
    }
    out.sort();
    Ok(out)
}

/// Collect regular files below `dir` as `/`-separated site-relative paths; symlinks are refused.
fn collect_entries(base: &Path, dir: &Path, site_prefix: &str, out: &mut BTreeMap<String, PathBuf>) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("dir entry under {}: {e}", dir.display()))?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|e| format!("file type of {}: {e}", path.display()))?;
        if file_type.is_symlink() {
            return Err(format!("{}: symlinks are never published on Pages", path.display()));
        }
        if file_type.is_dir() {
            collect_entries(base, &path, site_prefix, out)?;
        } else if file_type.is_file() {
            let rel = path.strip_prefix(base).map_err(|e| e.to_string())?;
            let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            out.insert(format!("{site_prefix}{}", parts.join("/")), path);
        }
    }
    Ok(())
}

/// Local targets of `href="…"`, `src="…"`, and meta-refresh `url=…` in `html`, without fragments or queries.
fn local_references(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    for attr in ["href=\"", "src=\""] {
        for chunk in html.split(attr).skip(1) {
            if let Some(value) = chunk.split('"').next() {
                push_local(value, &mut out);
            }
        }
    }
    for chunk in html.split("http-equiv=\"refresh\"").skip(1) {
        let tag = chunk.split('>').next().unwrap_or("");
        if let Some(target) = tag.split("url=").nth(1).and_then(|rest| rest.split('"').next()) {
            push_local(target.trim(), &mut out);
        }
    }
    out
}

fn push_local(raw: &str, out: &mut Vec<String>) {
    let value = raw.trim();
    let is_external = value.starts_with("//") || value.split_once(':').is_some_and(|(scheme, _)| !scheme.contains('/'));
    let path = value.split(['#', '?']).next().unwrap_or("");
    if !is_external && !path.is_empty() {
        out.push(path.to_owned());
    }
}

/// Resolve `target` relative to the page at `from_rel`; `None` when it climbs above the site root.
/// Root-absolute paths (`/x`) return `None` on normal pages (a project site lives below `/curvebook/`),
/// except on `404.html`, which GitHub Pages serves at arbitrary nested paths and therefore uses
/// `site_base_path()`-prefixed URLs.
fn resolve(from_rel: &str, target: &str) -> Option<String> {
    if target.starts_with('/') {
        if from_rel == "404.html"
            && let Some(stripped) = target.strip_prefix(site_base_path())
        {
            return resolve("index.html", stripped);
        }
        return None;
    }
    let mut parts: Vec<&str> = from_rel.split('/').collect();
    parts.pop();
    for segment in target.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    if target.ends_with('/') || joined.is_empty() {
        Some(if joined.is_empty() { String::from("index.html") } else { format!("{joined}/index.html") })
    } else {
        Some(joined)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempRoot(std::path::PathBuf);

    impl TempRoot {
        fn new(label: &str) -> Self {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
            Self(std::env::temp_dir().join(format!("cb-pages-site-{label}-{}-{nanos}", std::process::id())))
        }

        fn write(&self, rel: &str, body: &str) {
            let path = self.0.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A minimal site with generated manual HTML in `target/docs-manual/` plus internal files Pages must not publish.
    fn seed(label: &str) -> TempRoot {
        let root = TempRoot::new(label);
        root.write("docs/index.html", "<a href=\"open-pzfx-files.html#convert\">x</a><a href=\"faq.html\">faq</a><img src=\"images/a.png\"><a href=\"https://github.com/x\">gh</a><a href=\"#top\">t</a><a href=\"mailto:a@b.c\">m</a>");
        root.write("docs/open-pzfx-files.html", "<a href=\"manual/latest/files.html\">f</a><link rel=\"stylesheet\" href=\"site.css\">");
        root.write("docs/linux.html", "<a href=\"index.html\">h</a><a href=\"manual/latest/get-started.html\">gs</a>");
        root.write("docs/faq.html", "<a href=\"index.html\">h</a><a href=\"manual/latest/index.html\">m</a>");
        root.write("docs/privacy.html", "<a href=\"index.html\">h</a>");
        root.write("docs/get-started.html", "<meta http-equiv=\"refresh\" content=\"0; url=manual/latest/get-started.html\">");
        root.write("docs/troubleshooting.html", "<meta http-equiv=\"refresh\" content=\"0; url=manual/latest/troubleshooting.html#known-issues\">");
        root.write("target/docs-manual/index.html", "<meta http-equiv=\"refresh\" content=\"0; url=latest/index.html\">");
        root.write("target/docs-manual/latest/manifest.json", "{}");
        root.write("target/docs-manual/latest/index.html", "<a href=\"../../index.html\">home</a><a href=\"registry/formats.html\">r</a>");
        root.write("target/docs-manual/latest/files.html", "<link href=\"../../site.css\">");
        root.write("target/docs-manual/latest/get-started.html", "gs");
        root.write("target/docs-manual/latest/troubleshooting.html", "ts");
        root.write("target/docs-manual/latest/registry/formats.html", "<a href=\"../index.html\">up</a>");
        root.write("target/docs-manual/v0.1/manifest.json", "{}");
        root.write("target/docs-manual/v0.1/index.html", "<a href=\"../../index.html\">home</a><a href=\"registry/formats.html\">r</a>");
        root.write("target/docs-manual/v0.1/files.html", "<link href=\"../../site.css\">");
        root.write("target/docs-manual/v0.1/get-started.html", "gs");
        root.write("target/docs-manual/v0.1/troubleshooting.html", "ts");
        root.write("target/docs-manual/v0.1/registry/formats.html", "<a href=\"../index.html\">up</a>");
        root.write("target/docs-manual/scratch/notes.html", "not a release snapshot");
        root.write("docs/site.css", "body{}");
        root.write(
            "docs/404.html",
            "<link rel=\"stylesheet\" href=\"/curvebook/site.css\"><a href=\"/curvebook/\">h</a><a href=\"/curvebook/manual/latest/index.html\">m</a>",
        );
        root.write("docs/images/a.png", "png");
        root.write("docs/llms.txt", "# CurveBook\n");
        root.write("docs/llms-full.txt", "# CurveBook Full\n");
        root.write(
            "docs/sitemap.xml",
            "<urlset><url><loc>https://hausergroup.github.io/curvebook/</loc></url><url><loc>https://hausergroup.github.io/curvebook/faq.html</loc></url><url><loc>https://hausergroup.github.io/curvebook/manual/latest/files.html</loc></url></urlset>",
        );
        root.write("docs/ROADMAP.md", "internal");
        root.write("docs/audit/report.md", "internal");
        root.write("docs/schemas/preferences.schema.json", "{}");
        root
    }

    #[test]
    fn r12_9_publish_list_holds_only_site_files() {
        let root = seed("list");
        let files = check_publish_set(&root.0).unwrap();
        assert_eq!(
            files,
            [
                "404.html",
                "faq.html",
                "get-started.html",
                "images/a.png",
                "index.html",
                "linux.html",
                "llms-full.txt",
                "llms.txt",
                "manual/index.html",
                "manual/latest/files.html",
                "manual/latest/get-started.html",
                "manual/latest/index.html",
                "manual/latest/registry/formats.html",
                "manual/latest/troubleshooting.html",
                "manual/v0.1/files.html",
                "manual/v0.1/get-started.html",
                "manual/v0.1/index.html",
                "manual/v0.1/registry/formats.html",
                "manual/v0.1/troubleshooting.html",
                "open-pzfx-files.html",
                "privacy.html",
                "site.css",
                "sitemap.xml",
                "troubleshooting.html",
            ]
        );
    }

    #[test]
    fn r12_9_stage_copies_the_list_and_refuses_unsafe_targets() {
        let root = seed("stage");
        let out = root.0.join("_site");
        assert_eq!(stage(&root.0, &out).unwrap(), 24);
        assert_eq!(std::fs::read_to_string(out.join("manual/latest/registry/formats.html")).unwrap(), "<a href=\"../index.html\">up</a>");
        assert_eq!(std::fs::read_to_string(out.join("manual/v0.1/registry/formats.html")).unwrap(), "<a href=\"../index.html\">up</a>");
        assert!(out.join("faq.html").is_file() && out.join("llms.txt").is_file() && out.join("llms-full.txt").is_file());
        assert!(!out.join("ROADMAP.md").exists() && !out.join("audit").exists() && !out.join("schemas").exists());
        assert!(!out.join("manual/v0.1/manifest.json").exists() && !out.join("manual/latest/manifest.json").exists());

        assert!(stage(&root.0, &out).unwrap_err().contains("must be empty"));
        assert!(stage(&root.0, &root.0.join("docs/_site")).unwrap_err().contains("outside docs/"));
        assert!(stage(&root.0, &root.0.join("target/docs-manual/_site")).unwrap_err().contains("outside manual directory"));

        let custom_out = root.0.join("_custom");
        std::fs::create_dir_all(&custom_out).unwrap();
        assert_eq!(stage_custom_domain_files(&custom_out, "https://curvebook.org/", Some("curvebook.org")).unwrap(), 2);
        assert_eq!(std::fs::read_to_string(custom_out.join("CNAME")).unwrap(), "curvebook.org\n");
        assert!(std::fs::read_to_string(custom_out.join("robots.txt")).unwrap().contains("Sitemap: https://curvebook.org/sitemap.xml"));
    }

    #[test]
    fn r12_10_stage_supports_explicit_manual_dir_and_fails_closed_when_unbuilt() {
        let root = seed("explicit-manual");
        let moved = root.0.join("build/custom-manual");
        std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
        std::fs::rename(root.0.join("target/docs-manual"), &moved).unwrap();

        assert!(check_publish_set(&root.0).unwrap_err().contains("--build-manual"));
        let files = check_publish_set_with_manual(&root.0, Some(&moved)).unwrap();
        assert_eq!(files.len(), 24);
        assert!(files.contains(&String::from("manual/latest/index.html")));
        assert!(files.contains(&String::from("manual/v0.1/index.html")));

        let out = root.0.join("_staged");
        assert_eq!(stage_with_manual(&root.0, Some(&moved), &out).unwrap(), 24);
        assert!(out.join("manual/index.html").is_file());
        assert!(out.join("manual/latest/files.html").is_file());
        assert!(out.join("manual/v0.1/files.html").is_file());
        assert!(!out.join("manual/scratch").exists());
        assert!(stage_with_manual(&root.0, Some(&moved), &moved.join("_nested")).unwrap_err().contains("outside manual directory"));

        let parsed = parse_args(&[String::from("--manual-dir"), String::from("build/custom-manual"), String::from("_site")]).unwrap();
        assert_eq!(parsed.manual_dir, Some(PathBuf::from("build/custom-manual")));
        assert_eq!(parsed.action, StageAction::Stage(PathBuf::from("_site")));
        let parsed_eq = parse_args(&[String::from("--check"), String::from("--manual-dir=build/custom-manual")]).unwrap();
        assert_eq!(parsed_eq.manual_dir, Some(PathBuf::from("build/custom-manual")));
        assert_eq!(parsed_eq.action, StageAction::Check);
    }

    #[test]
    fn r12_9_links_to_unpublished_files_fail_the_check() {
        let root = seed("links");
        root.write("docs/privacy.html", "<a href=\"ROADMAP.md\">roadmap</a><a href=\"../outside.html\">up</a>");
        let err = check_publish_set(&root.0).unwrap_err();
        assert!(err.contains("`ROADMAP.md` (ROADMAP.md), which is not published"), "{err}");
        assert!(err.contains("`../outside.html`, which leaves the site root"), "{err}");

        let root = seed("refresh");
        root.write("docs/get-started.html", "<meta http-equiv=\"refresh\" content=\"0; url=manual/v0.2/get-started.html\">");
        assert!(check_publish_set(&root.0).unwrap_err().contains("manual/v0.2/get-started.html"));

        let root = seed("sitemap");
        root.write("docs/sitemap.xml", "<urlset><url><loc>https://hausergroup.github.io/curvebook/ROADMAP.md</loc></url></urlset>");
        assert!(check_publish_set(&root.0).unwrap_err().contains("sitemap.xml lists"));
    }

    #[test]
    fn r12_9_markdown_and_symlinks_are_never_published() {
        let root = seed("md");
        root.write("docs/images/notes.md", "oops");
        assert!(publish_list(&root.0).unwrap_err().contains("Markdown sources are never published"));

        #[cfg(unix)]
        {
            let root = seed("symlink");
            std::os::unix::fs::symlink(root.0.join("docs/ROADMAP.md"), root.0.join("docs/images/link.png")).unwrap();
            assert!(publish_list(&root.0).unwrap_err().contains("symlinks are never published"));
        }

        let root = seed("missing");
        std::fs::remove_file(root.0.join("docs/privacy.html")).unwrap();
        assert!(publish_list(&root.0).unwrap_err().contains("docs/privacy.html is on the Pages publish list but missing"));
    }

    #[test]
    fn r12_9_resolve_handles_relative_parent_and_root_paths() {
        assert_eq!(resolve("manual/v0.1/registry/formats.html", "../index.html").as_deref(), Some("manual/v0.1/index.html"));
        assert_eq!(resolve("manual/v0.1/index.html", "../../site.css").as_deref(), Some("site.css"));
        assert_eq!(resolve("index.html", "manual/").as_deref(), Some("manual/index.html"));
        assert_eq!(resolve("index.html", "../x.html"), None);
        assert_eq!(resolve("index.html", "/ROADMAP.md"), None);
        assert_eq!(resolve("404.html", "/curvebook/"), Some(String::from("index.html")));
        assert_eq!(resolve("404.html", "/curvebook/site.css").as_deref(), Some("site.css"));
        assert_eq!(resolve("404.html", "/other/site.css"), None);
    }
}
