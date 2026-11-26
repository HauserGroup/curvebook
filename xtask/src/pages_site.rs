//! The set of files the GitHub Pages site publishes (`R12.9`, `R12.10`, `R21.4`) and `cargo xtask pages-stage`.
//!
//! Pages deploys only the files listed here, combining committed site assets under `docs/` with
//! generated manual HTML and generated root metadata (`sitemap.xml`, `llms.txt`, `llms-full.txt`)
//! built into a build directory (default `target/docs-manual/`, or `--manual-dir <dir>`), so
//! internal Markdown (roadmap, planning, evidence, reference sources, and schemas) is never
//! served next to the site and generated outputs stay out of git. The published set is:
//!
//! - the hand-written landing pages in [`LANDING_PAGES`] and the redirect pages in [`REDIRECT_PAGES`];
//! - every file of `manual/latest/` and every active versioned snapshot under
//!   `manual/v<major>.<minor>/` from the manual build directory, except their `manifest.json`;
//! - the committed [`COMMITTED_SITE_FILES`], the generated [`GENERATED_SITE_FILES`] under
//!   `<manual_dir>/_root/`, and every file under the [`SITE_DIRS`].
//!
//! Every published HTML page may link only to published files: [`check_publish_set_with_manual`]
//! fails on a local `href`, `src`, or meta-refresh target that the list does not contain, and on
//! any `sitemap.xml` URL outside the list. This module needs no `docs-site` feature
//! dependency, so it continues to build with `--no-default-features`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use curvebook_platform::write_atomic;

use crate::scan_util::{is_real_dir, is_regular_file, read_dir_paths_sorted, rel_slash};

/// Published base URL of the GitHub Pages site.
pub const SITE_BASE_URL: &str = "https://hausergroup.github.io/curvebook/";

/// Optional custom domain (`CNAME`) for single-switch cutover; keep `None` while serving on `github.io`.
pub const CUSTOM_DOMAIN: Option<&str> = None;

/// Hand-written top-level pages under `docs/` that search engines should index, in sitemap order.
/// Each must carry a canonical link and Open Graph / Twitter Card metadata (`docs-site --check`).
pub const LANDING_PAGES: &[&str] = &["index.html", "open-pzfx-files.html", "linux.html", "faq.html", "privacy.html"];

/// Published redirect pages that forward to the newest release snapshot (left out of the sitemap).
pub const REDIRECT_PAGES: &[&str] = &["get-started.html", "troubleshooting.html", "manual/index.html"];

/// Committed individual published files, relative to `docs/`.
pub const COMMITTED_SITE_FILES: &[&str] = &["404.html", "site.css"];

/// Generated root files written into `<manual_dir>/_root/` by `docs-site --build-manual` (`R21.4`)
/// and staged to the root of `_site/` (never committed under `docs/`).
pub const GENERATED_SITE_FILES: &[&str] = &["sitemap.xml", "llms.txt", "llms-full.txt"];

/// Subdirectory inside `<manual_dir>` holding [`GENERATED_SITE_FILES`].
pub const MANUAL_ROOT_SUBDIR: &str = "_root";

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
    for rel in LANDING_PAGES.iter().chain(REDIRECT_PAGES).chain(COMMITTED_SITE_FILES) {
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

    let generated_root = manual_root.join(MANUAL_ROOT_SUBDIR);
    for rel in GENERATED_SITE_FILES {
        let path = generated_root.join(rel);
        if !is_regular_file(&path) {
            return Err(format!(
                "{}/{MANUAL_ROOT_SUBDIR}/{rel} is on the Pages publish list but missing (run `cargo run -p xtask --features docs-site -- docs-site --build-manual` first)",
                manual_root.display()
            ));
        }
        entries.insert((*rel).to_owned(), path);
    }

    let mut snap_roots: Vec<PathBuf> = snapshot_dirs(&manual_root)?.into_iter().map(|s| manual_root.join(s)).collect();
    let latest_dir = manual_root.join("latest");
    if let Ok(meta) = std::fs::symlink_metadata(&latest_dir) {
        if meta.file_type().is_symlink() {
            return Err(format!("{}: symlinks are never published on Pages", latest_dir.display()));
        }
        let latest_manifest = latest_dir.join("manifest.json");
        if let Ok(m_meta) = std::fs::symlink_metadata(&latest_manifest)
            && m_meta.file_type().is_symlink()
        {
            return Err(format!("{}: symlinks are never published on Pages", latest_manifest.display()));
        }
        if meta.file_type().is_dir() && is_regular_file(&latest_manifest) {
            snap_roots.push(latest_dir);
        }
    }
    let mut manual_page_count = 0usize;
    for snap_dir in snap_roots {
        let mut snap_entries = BTreeMap::new();
        collect_entries(&manual_root, &snap_dir, "manual/", &mut snap_entries)?;
        for (rel, src) in snap_entries {
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
    let mut problems = Vec::new();
    for entry in entries.iter().filter(|e| e.site_rel.ends_with(".html")) {
        let rel = entry.site_rel.as_str();
        let html = std::fs::read_to_string(&entry.source_path).map_err(|e| format!("read {}: {e}", entry.source_path.display()))?;
        match local_references(&html) {
            Ok(targets) => {
                for target in targets {
                    match resolve(rel, &target) {
                        Some(resolved) if published.contains(resolved.as_str()) => {}
                        Some(resolved) => problems.push(format!("{rel} links to `{target}` ({resolved}), which is not published")),
                        None => problems.push(format!("{rel} links to `{target}`, which leaves the site root")),
                    }
                }
            }
            Err(err) => problems.push(format!("{rel}: {err}")),
        }
    }
    let sitemap_entry =
        entries.iter().find(|e| e.site_rel == "sitemap.xml").ok_or_else(|| String::from("sitemap.xml missing from Pages publish list"))?;
    let sitemap = std::fs::read_to_string(&sitemap_entry.source_path).map_err(|e| format!("read {}: {e}", sitemap_entry.source_path.display()))?;
    for loc in sitemap.split("<loc>").skip(1).filter_map(|chunk| chunk.split("</loc>").next()) {
        let Some(rel) = loc.strip_prefix(SITE_BASE_URL) else {
            problems.push(format!("sitemap.xml lists `{loc}` outside `{SITE_BASE_URL}`"));
            continue;
        };
        let rel = if rel.is_empty() { "index.html" } else { rel };
        if !published.contains(rel) {
            problems.push(format!("sitemap.xml lists `{loc}`, which is not published"));
        }
    }
    if problems.is_empty() { Ok(entries) } else { Err(format!("Pages publish set has {} problem(s):\n  {}", problems.len(), problems.join("\n  "))) }
}

/// Copy the checked publish set into `out_dir`, which must be absent or empty and outside `docs/`.
#[cfg(test)]
pub fn stage(root: &Path, out_dir: &Path) -> Result<usize, String> {
    stage_with_manual(root, None, out_dir)
}

/// Parse a strict `v<major>.<minor>` snapshot directory name (ASCII digits only, fitting in `u64`).
#[must_use]
pub fn parse_minor_dir_version(name: &str) -> Option<(u64, u64)> {
    let core = name.strip_prefix('v')?;
    let (maj, min) = core.split_once('.')?;
    if maj.is_empty() || min.is_empty() || !maj.bytes().chain(min.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((maj.parse::<u64>().ok()?, min.parse::<u64>().ok()?))
}

/// Normalize `path` for containment checks by resolving symlinks on existing prefixes while
/// folding `.` and `..` segments cleanly even when trailing components do not yet exist.
pub(crate) fn resolve_path_for_containment(path: &Path) -> Result<PathBuf, String> {
    let mut cur = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(p) => cur.push(p.as_os_str()),
            Component::RootDir => cur.push(comp.as_os_str()),
            Component::CurDir => {}
            Component::Normal(seg) => {
                cur.push(seg);
                if let Ok(meta) = std::fs::symlink_metadata(&cur)
                    && meta.file_type().is_symlink()
                {
                    cur = std::fs::canonicalize(&cur).map_err(|e| format!("canonicalize {}: {e}", cur.display()))?;
                }
            }
            Component::ParentDir => {
                if std::fs::symlink_metadata(&cur).is_ok() {
                    cur = std::fs::canonicalize(&cur).map_err(|e| format!("canonicalize {}: {e}", cur.display()))?;
                }
                cur.pop();
            }
        }
    }
    let mut tail = Vec::new();
    let mut probe = cur.clone();
    while !probe.as_os_str().is_empty() && std::fs::symlink_metadata(&probe).is_err() {
        if let Some(name) = probe.file_name() {
            tail.push(name.to_os_string());
        }
        if !probe.pop() {
            break;
        }
    }
    let mut resolved = if !probe.as_os_str().is_empty() && std::fs::symlink_metadata(&probe).is_ok() {
        std::fs::canonicalize(&probe).map_err(|e| format!("canonicalize {}: {e}", probe.display()))?
    } else {
        cur
    };
    for seg in tail.into_iter().rev() {
        resolved.push(seg);
    }
    Ok(resolved)
}

/// Copy the checked publish set (combining `docs/` and `manual_dir`) into `out_dir`.
pub fn stage_with_manual(root: &Path, manual_dir: Option<&Path>, out_dir: &Path) -> Result<usize, String> {
    if let Ok(meta) = std::fs::symlink_metadata(out_dir) {
        if meta.file_type().is_symlink() {
            return Err(format!("staging directory {} must not be a symlink", out_dir.display()));
        }
        if !meta.file_type().is_dir() {
            return Err(format!("staging directory {} must be a directory", out_dir.display()));
        }
        let mut entries = std::fs::read_dir(out_dir).map_err(|e| format!("read_dir {}: {e}", out_dir.display()))?;
        if entries.next().is_some() {
            return Err(format!("staging directory {} must be empty", out_dir.display()));
        }
    }
    let docs = root.join("docs");
    let norm_out = resolve_path_for_containment(out_dir)?;
    let norm_docs = resolve_path_for_containment(&docs)?;
    if norm_out.starts_with(&norm_docs) || norm_docs.starts_with(&norm_out) {
        return Err(format!("staging directory {} must be outside docs/", out_dir.display()));
    }
    let resolved_manual = resolve_manual_dir(root, manual_dir)?;
    let norm_manual = resolve_path_for_containment(&resolved_manual)?;
    if norm_out.starts_with(&norm_manual) || norm_manual.starts_with(&norm_out) {
        return Err(format!("staging directory {} must be outside manual directory {}", out_dir.display(), resolved_manual.display()));
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

/// `v<major>.<minor>` release-snapshot directories under a manual root that carry a `manifest.json`.
fn snapshot_dirs(manual: &Path) -> Result<Vec<String>, String> {
    if !is_real_dir(manual) {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for path in read_dir_paths_sorted(manual)? {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if parse_minor_dir_version(&name).is_some() {
            let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("stat {}: {e}", path.display()))?;
            if meta.file_type().is_symlink() {
                return Err(format!("{}: symlinks are never published on Pages", path.display()));
            }
            let manifest = path.join("manifest.json");
            if let Ok(m_meta) = std::fs::symlink_metadata(&manifest)
                && m_meta.file_type().is_symlink()
            {
                return Err(format!("{}: symlinks are never published on Pages", manifest.display()));
            }
            if meta.file_type().is_dir() && is_regular_file(&manifest) {
                out.push(name);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Collect regular files below `dir` as `/`-separated site-relative paths; symlinks are refused.
fn collect_entries(base: &Path, dir: &Path, site_prefix: &str, out: &mut BTreeMap<String, PathBuf>) -> Result<(), String> {
    let dir_meta = std::fs::symlink_metadata(dir).map_err(|e| format!("file type of {}: {e}", dir.display()))?;
    if dir_meta.file_type().is_symlink() {
        return Err(format!("{}: symlinks are never published on Pages", dir.display()));
    }
    if !dir_meta.file_type().is_dir() {
        return Err(format!("{}: expected a regular directory for Pages staging", dir.display()));
    }
    for path in read_dir_paths_sorted(dir)? {
        let file_type = std::fs::symlink_metadata(&path).map_err(|e| format!("file type of {}: {e}", path.display()))?.file_type();
        if file_type.is_symlink() {
            return Err(format!("{}: symlinks are never published on Pages", path.display()));
        }
        if file_type.is_dir() {
            collect_entries(base, &path, site_prefix, out)?;
        } else if file_type.is_file() {
            out.insert(format!("{site_prefix}{}", rel_slash(base, &path)?), path);
        }
    }
    Ok(())
}

/// Given the slice immediately following `<!--`, return the byte offset immediately after the
/// HTML5 comment terminator (`<!-->`, `<!--->`, `-->`, or `--!>`), or `None` if unclosed.
pub(crate) fn find_html_comment_end(after_open: &str) -> Option<usize> {
    #[derive(Copy, Clone, PartialEq, Eq)]
    enum State {
        Start,
        StartDash,
        Comment,
        EndDash,
        End,
        EndBang,
    }
    let mut state = State::Start;
    for (i, &b) in after_open.as_bytes().iter().enumerate() {
        match state {
            State::Start => match b {
                b'>' => return Some(i + 1),
                b'-' => state = State::StartDash,
                _ => state = State::Comment,
            },
            State::StartDash => match b {
                b'>' => return Some(i + 1),
                b'-' => state = State::End,
                _ => state = State::Comment,
            },
            State::Comment => {
                if b == b'-' {
                    state = State::EndDash;
                }
            }
            State::EndDash => {
                if b == b'-' {
                    state = State::End;
                } else {
                    state = State::Comment;
                }
            }
            State::End => match b {
                b'>' => return Some(i + 1),
                b'!' => state = State::EndBang,
                b'-' => {}
                _ => state = State::Comment,
            },
            State::EndBang => match b {
                b'>' => return Some(i + 1),
                b'-' => state = State::EndDash,
                _ => state = State::Comment,
            },
        }
    }
    None
}

/// HTML5 §13.2.5.6 (Tag open state): `<` begins a tag, comment, or markup declaration ONLY when
/// immediately followed by an ASCII letter, `/`, `!`, or `?`; otherwise `<` is literal text.
#[must_use]
pub(crate) fn is_html_tag_start(after_lt: &str) -> bool {
    matches!(after_lt.as_bytes().first(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'/' | b'!' | b'?'))
}

/// Find the byte offset of the first matching HTML5 raw-text or RCDATA end tag `</tag_lower`
/// (case-insensitive) followed by `>`, `/`, or HTML5 ASCII whitespace (including form feed `0x0C`).
#[must_use]
pub(crate) fn find_html_raw_close_tag_start(html: &str, body_start: usize, tag_lower: &str) -> Option<usize> {
    let bytes = html.as_bytes();
    let tag_bytes = tag_lower.as_bytes();
    let mut i = body_start;
    while i + 2 + tag_bytes.len() <= bytes.len() {
        if bytes[i] == b'<'
            && bytes[i + 1] == b'/'
            && bytes[i + 2..i + 2 + tag_bytes.len()].eq_ignore_ascii_case(tag_bytes)
            && matches!(bytes.get(i + 2 + tag_bytes.len()), Some(b'>' | b'/' | b' ' | b'\t' | b'\r' | b'\n' | b'\x0c'))
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Skip an HTML5 raw-text or RCDATA element body (`<script>`, `<style>`, `<title>`, `<textarea>`)
/// starting at `body_start` (immediately after the opening tag's `>`), returning
/// `(body_end_abs, after_close_tag_abs)`. Fails closed if unclosed.
pub(crate) fn skip_html_raw_or_rcdata_element(html: &str, body_start: usize, tag_lower: &str) -> Result<(usize, usize), String> {
    let body_end = find_html_raw_close_tag_start(html, body_start, tag_lower).ok_or_else(|| format!("unclosed `<{tag_lower}>` element"))?;
    let after_lt = html.get(body_end + 1..).ok_or_else(|| format!("unclosed `</{tag_lower}>` closing tag"))?;
    let close_gt = find_html_tag_close(after_lt).ok_or_else(|| format!("unclosed `</{tag_lower}>` closing tag"))?;
    Ok((body_end, body_end + 1 + close_gt + 1))
}

/// Find the byte offset of the closing `>` of an HTML5 tag body (starting immediately after `<`),
/// entering quote mode only immediately after `=` (+ optional ASCII whitespace) per WHATWG HTML5
/// §13.2.5.32–38 so quotes in attribute names or unquoted values cannot desync tag boundaries.
pub(crate) fn find_html_tag_close(rest: &str) -> Option<usize> {
    let bytes = rest.as_bytes();
    if matches!(bytes.first(), Some(b'!' | b'?')) || (bytes.first() == Some(&b'/') && !bytes.get(1).is_some_and(u8::is_ascii_alphabetic)) {
        return rest.find('>');
    }
    #[derive(Copy, Clone, PartialEq, Eq)]
    enum State {
        TagName,
        BeforeAttrName,
        AttrName,
        AfterAttrName,
        BeforeAttrValue,
        DoubleQuoted,
        SingleQuoted,
        AfterQuotedValue,
        UnquotedValue,
    }
    let mut state = State::TagName;
    let mut i = usize::from(bytes.first() == Some(&b'/'));
    while i < bytes.len() {
        let b = bytes[i];
        match state {
            State::TagName => match b {
                b'>' => return Some(i),
                b'/' => state = State::BeforeAttrName,
                _ if b.is_ascii_whitespace() => state = State::BeforeAttrName,
                _ => {}
            },
            State::BeforeAttrName => match b {
                b'>' => return Some(i),
                b'/' => {}
                _ if b.is_ascii_whitespace() => {}
                _ => state = State::AttrName,
            },
            State::AttrName => match b {
                b'>' => return Some(i),
                b'=' => state = State::BeforeAttrValue,
                b'/' => state = State::BeforeAttrName,
                _ if b.is_ascii_whitespace() => state = State::AfterAttrName,
                _ => {}
            },
            State::AfterAttrName => match b {
                b'>' => return Some(i),
                b'=' => state = State::BeforeAttrValue,
                b'/' => state = State::BeforeAttrName,
                _ if b.is_ascii_whitespace() => {}
                _ => state = State::AttrName,
            },
            State::BeforeAttrValue => match b {
                b'>' => return Some(i),
                b'"' => state = State::DoubleQuoted,
                b'\'' => state = State::SingleQuoted,
                _ if b.is_ascii_whitespace() => {}
                _ => state = State::UnquotedValue,
            },
            State::DoubleQuoted => {
                if b == b'"' {
                    state = State::AfterQuotedValue;
                }
            }
            State::SingleQuoted => {
                if b == b'\'' {
                    state = State::AfterQuotedValue;
                }
            }
            State::AfterQuotedValue => match b {
                b'>' => return Some(i),
                b'/' => state = State::BeforeAttrName,
                _ if b.is_ascii_whitespace() => state = State::BeforeAttrName,
                _ => state = State::AttrName,
            },
            State::UnquotedValue => match b {
                b'>' => return Some(i),
                _ if b.is_ascii_whitespace() => state = State::BeforeAttrName,
                _ => {}
            },
        }
        i += 1;
    }
    None
}

/// Extract the HTML5 tag name from a trimmed tag body (stopping at ASCII whitespace or `/`).
#[must_use]
pub(crate) fn extract_html_tag_name(trimmed_tag: &str) -> &str {
    let body = trimmed_tag.strip_prefix('/').unwrap_or(trimmed_tag).trim_start();
    let end = body.bytes().position(|b| b.is_ascii_whitespace() || b == b'/').unwrap_or(body.len());
    body.get(..end).unwrap_or("")
}

/// Parse HTML5 tag attributes from a trimmed tag body, supporting `/` tag-name delimiters,
/// double/single/unquoted attribute values, whitespace around `=`, and entity decoding.
pub(crate) fn parse_html_tag_attributes(trimmed_tag: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    let bytes = trimmed_tag.as_bytes();
    let mut i = bytes.iter().position(|b| b.is_ascii_whitespace() || *b == b'/').unwrap_or(bytes.len());
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let k_start = i;
        // HTML5 §13.2.5.32: the first non-whitespace, non-`/` byte starts the attribute name even if `=`.
        i += 1;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' && bytes[i] != b'/' {
            i += 1;
        }
        let key = trimmed_tag.get(k_start..i).unwrap_or("").to_owned();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            if !key.is_empty() {
                attrs.push((key, String::new()));
            }
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let val = if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
            let q = bytes[i];
            i += 1;
            let v_start = i;
            while i < bytes.len() && bytes[i] != q {
                i += 1;
            }
            let raw = trimmed_tag.get(v_start..i).unwrap_or("").to_owned();
            if i < bytes.len() {
                i += 1;
            }
            unescape_html_attr(&raw)
        } else {
            let v_start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            unescape_html_attr(trimmed_tag.get(v_start..i).unwrap_or(""))
        };
        attrs.push((key, val));
    }
    attrs
}

pub(crate) fn unescape_html_attr(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let bytes = raw.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'&'
            && let Some((ch, consumed)) = decode_html_entity(raw.get(i..).unwrap_or(""))
        {
            out.push(ch);
            i += consumed;
            continue;
        }
        if let Some(rest) = raw.get(i..)
            && let Some(ch) = rest.chars().next()
        {
            out.push(ch);
            i += ch.len_utf8();
        } else {
            i += 1;
        }
    }
    out
}

fn decode_html_entity(slice: &str) -> Option<(char, usize)> {
    let rest = slice.strip_prefix('&')?;
    if let Some(num) = rest.strip_prefix('#') {
        let (tail, radix, max_digits, prefix_len) =
            if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) { (hex, 16, 8, 3) } else { (num, 10, 10, 2) };
        let digits_len = tail.bytes().take_while(|b| if radix == 16 { b.is_ascii_hexdigit() } else { b.is_ascii_digit() }).count();
        if digits_len == 0 {
            return None;
        }
        let trimmed = tail.get(..digits_len)?.trim_start_matches('0');
        let code = if trimmed.is_empty() {
            0u32
        } else if trimmed.len() > max_digits {
            return None;
        } else {
            u32::from_str_radix(trimmed, radix).ok()?
        };
        let semi = usize::from(tail.as_bytes().get(digits_len) == Some(&b';'));
        let ch = char::from_u32(code).filter(|&c| c != '\0').unwrap_or('\u{FFFD}');
        return Some((ch, prefix_len + digits_len + semi));
    }
    let semi_pos = rest.find(';')?;
    if semi_pos == 0 || semi_pos > 16 {
        return None;
    }
    let ch = match rest.get(..semi_pos)?.to_ascii_lowercase().as_str() {
        "amp" => '&',
        "quot" => '"',
        "apos" => '\'',
        "lt" => '<',
        "gt" => '>',
        "colon" => ':',
        "sol" => '/',
        "semi" => ';',
        "period" => '.',
        "equals" => '=',
        "quest" => '?',
        "nbsp" => ' ',
        "tab" => '\t',
        "newline" => '\n',
        _ => return None,
    };
    Some((ch, semi_pos + 2))
}

/// Parse the redirect target URL from a `<meta http-equiv="refresh" content="...">` value per
/// WHATWG HTML §4.2.5.3 (supporting `;`, `,`, or whitespace separators, optional `url=`, and quotes).
#[must_use]
pub(crate) fn parse_meta_refresh_url(content: &str) -> Option<&str> {
    let bytes = content.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let digits_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == digits_start && bytes.get(i) != Some(&b'.') {
        return None;
    }
    while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
        i += 1;
    }
    let ws_before = i;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let saw_ws = i > ws_before;
    if bytes[i] == b';' || bytes[i] == b',' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
    } else if !saw_ws {
        return None;
    }
    let rest = content.get(i..)?;
    if rest.is_empty() {
        return None;
    }
    let url_slice = if let Some(after_u) = rest.get(..3).filter(|s| s.eq_ignore_ascii_case("url")).and_then(|_| rest.get(3..)) {
        let after_ws = after_u.trim_start_matches(|c: char| c.is_ascii_whitespace());
        if let Some(after_eq) = after_ws.strip_prefix('=') { after_eq.trim_start_matches(|c: char| c.is_ascii_whitespace()) } else { rest }
    } else {
        rest
    };
    let target = if let Some(after_q) = url_slice.strip_prefix('"') {
        after_q.split('"').next().unwrap_or(after_q)
    } else if let Some(after_q) = url_slice.strip_prefix('\'') {
        after_q.split('\'').next().unwrap_or(after_q)
    } else {
        url_slice.trim_end_matches(|c: char| c.is_ascii_whitespace())
    };
    (!target.is_empty()).then_some(target)
}

/// Local targets of `href`, `src`, `srcset`, `poster`, `action`, `formaction`, `ping`, `xlink:href`,
/// and meta-refresh values in `html`, without fragments or queries. Fails closed on unclosed HTML
/// comments or tags.
fn local_references(html: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some((_, after_lt)) = rest.split_once('<') {
        if !is_html_tag_start(after_lt) {
            rest = after_lt;
            continue;
        }
        if let Some(after_comment) = after_lt.strip_prefix("!--") {
            let Some(close_end) = find_html_comment_end(after_comment) else {
                return Err(String::from("unclosed HTML comment `<!--`"));
            };
            rest = after_comment.get(close_end..).ok_or_else(|| String::from("unclosed HTML comment `<!--`"))?;
            continue;
        }
        let Some(tag_end) = find_html_tag_close(after_lt) else {
            return Err(String::from("unclosed HTML tag or attribute quote"));
        };
        let Some((tag_body, tail)) = after_lt.split_at_checked(tag_end) else {
            return Err(String::from("unclosed HTML tag or attribute quote"));
        };
        rest = tail.strip_prefix('>').unwrap_or(tail);
        let trimmed = tag_body.trim();
        if trimmed.starts_with('/') {
            continue;
        }
        let tag_lower = extract_html_tag_name(trimmed).to_ascii_lowercase();
        let attrs = parse_html_tag_attributes(trimmed);
        if let Some((bad_key, _)) = attrs.iter().find(|(k, _)| k.contains(['"', '\'', '<'])) {
            return Err(format!("malformed HTML attribute name `{bad_key}` on `<{tag_lower}>` (unclosed quote or parse error)"));
        }
        collect_tag_local_refs(&tag_lower, &attrs, &mut out);
        if matches!(tag_lower.as_str(), "title" | "textarea" | "script" | "style") {
            let (_, after_close) = skip_html_raw_or_rcdata_element(rest, 0, &tag_lower)?;
            rest = rest.get(after_close..).unwrap_or("");
        }
    }
    Ok(out)
}

fn collect_tag_local_refs(tag_lower: &str, attrs: &[(String, String)], out: &mut Vec<String>) {
    let mut is_meta_refresh = false;
    let mut refresh_contents: Vec<&str> = Vec::new();
    let mut saw_http_equiv = false;
    for (k, v) in attrs {
        let key_lower = k.to_ascii_lowercase();
        match key_lower.as_str() {
            "href" | "src" | "poster" | "action" | "formaction" | "xlink:href" => push_local(v, out),
            "srcset" | "imagesrcset" => {
                for candidate in v.split(',') {
                    if let Some(url) = candidate.split_whitespace().next() {
                        push_local(url, out);
                    }
                }
            }
            "ping" => {
                for url in v.split_whitespace() {
                    push_local(url, out);
                }
            }
            "http-equiv" if tag_lower == "meta" && !saw_http_equiv => {
                saw_http_equiv = true;
                if v.trim().eq_ignore_ascii_case("refresh") {
                    is_meta_refresh = true;
                }
            }
            "content" if tag_lower == "meta" => {
                refresh_contents.push(v.as_str());
            }
            _ => {}
        }
    }
    if is_meta_refresh {
        for content in refresh_contents {
            if let Some(target) = parse_meta_refresh_url(content) {
                push_local(target, out);
            }
        }
    }
}

fn push_local(raw: &str, out: &mut Vec<String>) {
    let value = raw.trim();
    if value.starts_with("//") {
        return;
    }
    let path = value.split(['#', '?']).next().unwrap_or("").trim();
    let is_scheme = path.split_once(':').is_some_and(|(scheme, _)| !scheme.is_empty() && !scheme.contains('/'));
    if !is_scheme && !path.is_empty() {
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
    use crate::scan_util::test_support::TestDir;

    /// A minimal site with generated manual HTML in `target/docs-manual/` plus internal files Pages must not publish.
    fn seed(label: &str) -> TestDir {
        let root = TestDir::new(&format!("pages-site-{label}"));
        for (rel, body) in [
            (
                "docs/index.html",
                "<a href=\"open-pzfx-files.html#convert\">x</a><a href=\"faq.html\">faq</a><img src=\"images/a.png\"><a href=\"https://github.com/x\">gh</a><a href=\"#top\">t</a><a href=\"mailto:a@b.c\">m</a>",
            ),
            ("docs/open-pzfx-files.html", "<a href=\"manual/latest/files.html\">f</a><link rel=\"stylesheet\" href=\"site.css\">"),
            ("docs/linux.html", "<a href=\"index.html\">h</a><a href=\"manual/latest/get-started.html\">gs</a>"),
            ("docs/faq.html", "<a href=\"index.html\">h</a><a href=\"manual/latest/index.html\">m</a>"),
            ("docs/privacy.html", "<a href=\"index.html\">h</a>"),
            ("docs/get-started.html", "<meta http-equiv=\"refresh\" content=\"0; url=manual/latest/get-started.html\">"),
            ("docs/troubleshooting.html", "<meta http-equiv=\"refresh\" content=\"0; url=manual/latest/troubleshooting.html#known-issues\">"),
            ("target/docs-manual/index.html", "<meta http-equiv=\"refresh\" content=\"0; url=latest/index.html\">"),
            ("target/docs-manual/scratch/notes.html", "not a release snapshot"),
            ("docs/site.css", "body{}"),
            (
                "docs/404.html",
                "<link rel=\"stylesheet\" href=\"/curvebook/site.css\"><a href=\"/curvebook/\">h</a><a href=\"/curvebook/manual/latest/index.html\">m</a>",
            ),
            ("docs/images/a.png", "png"),
            ("target/docs-manual/_root/llms.txt", "# CurveBook\n"),
            ("target/docs-manual/_root/llms-full.txt", "# CurveBook Full\n"),
            (
                "target/docs-manual/_root/sitemap.xml",
                "<urlset><url><loc>https://hausergroup.github.io/curvebook/</loc></url><url><loc>https://hausergroup.github.io/curvebook/faq.html</loc></url><url><loc>https://hausergroup.github.io/curvebook/manual/latest/files.html</loc></url></urlset>",
            ),
            ("docs/ROADMAP.md", "internal"),
            ("docs/audit/report.md", "internal"),
            ("docs/schemas/preferences.schema.json", "{}"),
        ] {
            root.write(rel, body);
        }
        for snap in ["latest", "v0.1"] {
            for (rel, body) in [
                ("manifest.json", "{}"),
                ("index.html", "<a href=\"../../index.html\">home</a><a href=\"registry/formats.html\">r</a>"),
                ("files.html", "<link href=\"../../site.css\">"),
                ("get-started.html", "gs"),
                ("troubleshooting.html", "ts"),
                ("registry/formats.html", "<a href=\"../index.html\">up</a>"),
            ] {
                root.write(&format!("target/docs-manual/{snap}/{rel}"), body);
            }
        }
        root
    }

    #[test]
    fn r12_9_publish_list_holds_only_site_files() {
        let root = seed("list");
        let files = check_publish_set(root.path()).unwrap();
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
        let out = root.join("_site");
        assert_eq!(stage(root.path(), &out).unwrap(), 24);
        assert_eq!(std::fs::read_to_string(out.join("manual/latest/registry/formats.html")).unwrap(), "<a href=\"../index.html\">up</a>");
        assert_eq!(std::fs::read_to_string(out.join("manual/v0.1/registry/formats.html")).unwrap(), "<a href=\"../index.html\">up</a>");
        assert!(out.join("faq.html").is_file() && out.join("llms.txt").is_file() && out.join("llms-full.txt").is_file());
        assert!(!out.join("ROADMAP.md").exists() && !out.join("audit").exists() && !out.join("schemas").exists());
        assert!(!out.join("manual/v0.1/manifest.json").exists() && !out.join("manual/latest/manifest.json").exists());

        assert!(stage(root.path(), &out).unwrap_err().contains("must be empty"));
        assert!(stage(root.path(), &root.join("docs/_site")).unwrap_err().contains("outside docs/"));
        assert!(stage(root.path(), &root.join("target/docs-manual/_site")).unwrap_err().contains("outside manual directory"));

        let custom_out = root.join("_custom");
        std::fs::create_dir_all(&custom_out).unwrap();
        assert_eq!(stage_custom_domain_files(&custom_out, "https://curvebook.org/", Some("curvebook.org")).unwrap(), 2);
        assert_eq!(std::fs::read_to_string(custom_out.join("CNAME")).unwrap(), "curvebook.org\n");
        assert!(std::fs::read_to_string(custom_out.join("robots.txt")).unwrap().contains("Sitemap: https://curvebook.org/sitemap.xml"));
    }

    #[test]
    fn r12_10_stage_supports_explicit_manual_dir_and_fails_closed_when_unbuilt() {
        let root = seed("explicit-manual");
        let moved = root.join("build/custom-manual");
        std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
        std::fs::rename(root.join("target/docs-manual"), &moved).unwrap();

        assert!(check_publish_set(root.path()).unwrap_err().contains("--build-manual"));
        let files = check_publish_set_with_manual(root.path(), Some(&moved)).unwrap();
        assert_eq!(files.len(), 24);
        assert!(files.contains(&String::from("manual/latest/index.html")) && files.contains(&String::from("manual/v0.1/index.html")));

        let out = root.join("_staged");
        assert_eq!(stage_with_manual(root.path(), Some(&moved), &out).unwrap(), 24);
        assert!(
            out.join("manual/index.html").is_file() && out.join("manual/latest/files.html").is_file() && out.join("manual/v0.1/files.html").is_file()
        );
        assert!(!out.join("manual/scratch").exists());
        assert!(stage_with_manual(root.path(), Some(&moved), &moved.join("_nested")).unwrap_err().contains("outside manual directory"));

        let parsed = parse_args(&[String::from("--manual-dir"), String::from("build/custom-manual"), String::from("_site")]).unwrap();
        assert_eq!(parsed, StageCli { manual_dir: Some(PathBuf::from("build/custom-manual")), action: StageAction::Stage(PathBuf::from("_site")) });
        let parsed_eq = parse_args(&[String::from("--check"), String::from("--manual-dir=build/custom-manual")]).unwrap();
        assert_eq!(parsed_eq, StageCli { manual_dir: Some(PathBuf::from("build/custom-manual")), action: StageAction::Check });
    }

    #[test]
    fn r12_9_links_to_unpublished_files_fail_the_check() {
        let root = seed("links");
        root.write("docs/privacy.html", "<a href=\"ROADMAP.md\">roadmap</a><a href=\"../outside.html\">up</a>");
        let err = check_publish_set(root.path()).unwrap_err();
        assert!(
            err.contains("`ROADMAP.md` (ROADMAP.md), which is not published") && err.contains("`../outside.html`, which leaves the site root"),
            "{err}"
        );

        for (label, rel, body, needle) in [
            (
                "refresh",
                "docs/get-started.html",
                "<meta http-equiv=\"refresh\" content=\"0; url=manual/v0.2/get-started.html\">",
                "manual/v0.2/get-started.html",
            ),
            (
                "sitemap",
                "target/docs-manual/_root/sitemap.xml",
                "<urlset><url><loc>https://hausergroup.github.io/curvebook/ROADMAP.md</loc></url></urlset>",
                "sitemap.xml lists",
            ),
        ] {
            let r = seed(label);
            r.write(rel, body);
            assert!(check_publish_set(r.path()).unwrap_err().contains(needle));
        }
    }

    #[test]
    fn r12_9_markdown_and_symlinks_are_never_published() {
        let root = seed("md");
        root.write("docs/images/notes.md", "oops");
        assert!(publish_list(root.path()).unwrap_err().contains("Markdown sources are never published"));

        #[cfg(unix)]
        {
            let root = seed("symlink");
            std::os::unix::fs::symlink(root.join("docs/ROADMAP.md"), root.join("docs/images/link.png")).unwrap();
            assert!(publish_list(root.path()).unwrap_err().contains("symlinks are never published"));
        }

        let root = seed("missing");
        std::fs::remove_file(root.join("docs/privacy.html")).unwrap();
        assert!(publish_list(root.path()).unwrap_err().contains("docs/privacy.html is on the Pages publish list but missing"));
    }

    #[test]
    fn r12_9_resolve_handles_relative_parent_and_root_paths() {
        for (from, target, expected) in [
            ("manual/v0.1/registry/formats.html", "../index.html", Some("manual/v0.1/index.html")),
            ("manual/v0.1/index.html", "../../site.css", Some("site.css")),
            ("index.html", "manual/", Some("manual/index.html")),
            ("index.html", "../x.html", None),
            ("index.html", "/ROADMAP.md", None),
            ("404.html", "/curvebook/", Some("index.html")),
            ("404.html", "/curvebook/site.css", Some("site.css")),
            ("404.html", "/other/site.css", None),
        ] {
            assert_eq!(resolve(from, target).as_deref(), expected, "resolve({from}, {target})");
        }
    }

    #[test]
    fn aud_12_07_and_26_traversal_symlinks_html_links_and_snapshot_grammar() {
        // 1. stage() rejects `..` traversal into docs/ or manual_dir and symlinked out_dir.
        let root = seed("aud-12-07-stage");
        assert!(stage(root.path(), &root.join("_site/../docs/sub")).unwrap_err().contains("outside docs/"));
        assert!(stage(root.path(), &root.join("_site/../target/docs-manual/sub")).unwrap_err().contains("outside manual directory"));

        #[cfg(unix)]
        {
            let empty_dir = root.join("_empty_target");
            std::fs::create_dir_all(&empty_dir).unwrap();
            let link_out = root.join("_symlink_out");
            std::os::unix::fs::symlink(&empty_dir, &link_out).unwrap();
            assert!(stage(root.path(), &link_out).unwrap_err().contains("symlink"));

            let link_to_docs = root.join("_link_docs");
            std::os::unix::fs::symlink(root.join("docs"), &link_to_docs).unwrap();
            assert!(stage(root.path(), &link_to_docs.join("nested")).unwrap_err().contains("outside docs/"));

            // 2. Symlinked SITE_DIRS root (`docs/images`) must be rejected.
            let sym_root = seed("aud-12-07-sitedir");
            std::fs::remove_dir_all(sym_root.join("docs/images")).unwrap();
            std::os::unix::fs::symlink(sym_root.join("docs/audit"), sym_root.join("docs/images")).unwrap();
            assert!(publish_list(sym_root.path()).unwrap_err().contains("symlinks are never published"));
        }

        // 3. local_references / push_local catch single quotes, uppercase, unquoted, spaced '=',
        //    slash tag delimiter, reversed meta refresh, url=-less/comma refresh, and colon in fragment/query.
        for snippet in [
            "<a href='ROADMAP.md'>x</a>",
            "<A HREF=\"ROADMAP.md\">x</A>",
            "<a href=ROADMAP.md>x</a>",
            "<a href = \"ROADMAP.md\">x</a>",
            "<a/href=\"ROADMAP.md\">x</a>",
            "<meta content=\"0; url=ROADMAP.md\" http-equiv=\"refresh\">",
            "<meta http-equiv=\"refresh\" content=\"0; ROADMAP.md\">",
            "<meta http-equiv=\"refresh\" content=\"0, url=ROADMAP.md\">",
            "<a href=\"ROADMAP.md#r1:2\">x</a>",
            "<a href=\"ROADMAP.md?v=1:2\">x</a>",
        ] {
            let r = seed("aud-12-07-links");
            r.write("docs/privacy.html", snippet);
            assert!(check_publish_set(r.path()).unwrap_err().contains("ROADMAP.md"), "expected `{snippet}` to fail publish set check");
        }

        // 4. Unified parse_minor_dir_version rejects + signs and u64 overflow.
        assert_eq!(parse_minor_dir_version("v0.1"), Some((0, 1)));
        assert_eq!(parse_minor_dir_version("v+0.+1"), None);
        assert_eq!(parse_minor_dir_version("v999999999999999999999999.0"), None);

        // 5. HTML5 quote-desync, abrupt/bang comment closes, and unclosed tag/comment fail-closed.
        for snippet in [
            "<p title=x\"><a href=\"ROADMAP.md\">x</a><b z=1\">",
            "<p a\"><a href=\"ROADMAP.md\">x</a><b z=\">",
            "<!--><a href=\"ROADMAP.md\">x</a><!-- -->",
            "<!---><a href=\"ROADMAP.md\">x</a><!-- -->",
            "<!-- c --!><a href=\"ROADMAP.md\">x</a><!-- -->",
            "<!-- unclosed comment",
            "<a href=\"index.html\" title=\"unclosed",
            "< a=\"><a href=\"ROADMAP.md\">x</a>",
            "<<a href=\"ROADMAP.md\">x</a>",
            "<p =\" href=\"ROADMAP.md\" x=\">",
            "<title><!--</title><a href=\"ROADMAP.md\">x</a><!-- -->",
            "<textarea><!--</textarea><a href=\"ROADMAP.md\">x</a><!-- -->",
        ] {
            let r = seed("aud-12-07-html5");
            r.write("docs/privacy.html", snippet);
            assert!(check_publish_set(r.path()).is_err(), "expected `{snippet}` to fail publish set check");
        }
    }
}
