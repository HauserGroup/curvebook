//! The set of files the GitHub Pages site publishes (`R12.9`) and `cargo xtask pages-stage`.
//!
//! Pages deploys only the files listed here, copied out of `docs/` by `pages-stage`, so the
//! repository's internal Markdown (roadmap, audits, planning, evidence, reference sources, and
//! schemas) is never served next to the site. The list is:
//!
//! - the hand-written landing pages in [`LANDING_PAGES`] and the redirect pages in [`REDIRECT_PAGES`];
//! - every file of every release snapshot under `docs/manual/v<major>.<minor>/` except its
//!   `manifest.json`;
//! - the shared [`SITE_FILES`] and every file under the [`SITE_DIRS`].
//!
//! Every published HTML page may link only to published files: [`check_publish_set`] fails on a
//! local `href`, `src`, or meta-refresh target that the list does not contain, and on any
//! `docs/sitemap.xml` URL outside the list. This module needs no `full-gates` dependency, so the
//! deploy workflow can build `xtask` with `--no-default-features`.

use std::collections::BTreeSet;
use std::path::Path;

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
    match rest.find('/') {
        Some(idx) => &rest[idx..],
        None => "/",
    }
}

/// Directories under `docs/` whose files are all published.
pub const SITE_DIRS: &[&str] = &["images"];

/// Release-snapshot files that stay in the repository but are not published.
const SNAPSHOT_EXCLUDED_FILES: &[&str] = &["manifest.json"];

const USAGE: &str = "usage: cargo xtask pages-stage <output-dir> | --list | --check";

/// `cargo xtask pages-stage <output-dir> | --list | --check`.
pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    match args {
        [flag] if flag == "--list" => {
            for rel in check_publish_set(root)? {
                println!("{rel}");
            }
            Ok(())
        }
        [flag] if flag == "--check" => {
            let files = check_publish_set(root)?;
            println!("pages-stage: {} published files, every local link resolves inside the published set", files.len());
            Ok(())
        }
        [flag] if flag == "-h" || flag == "--help" => {
            println!("{USAGE}");
            Ok(())
        }
        [out] if !out.starts_with('-') => {
            let out_dir = if Path::new(out).is_absolute() { Path::new(out).to_path_buf() } else { root.join(out) };
            let count = stage(root, &out_dir)?;
            println!("pages-stage: staged {count} files into {}", out_dir.display());
            Ok(())
        }
        _ => Err(String::from(USAGE)),
    }
}

/// Every published file, as a sorted `/`-separated path relative to `docs/`.
pub fn publish_list(root: &Path) -> Result<Vec<String>, String> {
    let docs = root.join("docs");
    let mut out = BTreeSet::new();
    for rel in LANDING_PAGES.iter().chain(REDIRECT_PAGES).chain(SITE_FILES) {
        if !is_regular_file(&docs.join(rel)) {
            return Err(format!("docs/{rel} is on the Pages publish list but missing (or not a regular file)"));
        }
        out.insert((*rel).to_owned());
    }
    for dir in SITE_DIRS {
        collect_files(&docs, &docs.join(dir), &mut out)?;
    }
    for snapshot in snapshot_dirs(&docs.join("manual"))? {
        let mut files = BTreeSet::new();
        collect_files(&docs, &docs.join("manual").join(&snapshot), &mut files)?;
        out.extend(files.into_iter().filter(|rel| !SNAPSHOT_EXCLUDED_FILES.iter().any(|name| rel.rsplit('/').next() == Some(*name))));
    }
    let latest_dir = docs.join("manual/latest");
    if std::fs::symlink_metadata(&latest_dir).is_ok_and(|m| m.file_type().is_dir()) && is_regular_file(&latest_dir.join("manifest.json")) {
        let mut files = BTreeSet::new();
        collect_files(&docs, &latest_dir, &mut files)?;
        out.extend(files.into_iter().filter(|rel| !SNAPSHOT_EXCLUDED_FILES.iter().any(|name| rel.rsplit('/').next() == Some(*name))));
    }
    if let Some(md) = out.iter().find(|rel| rel.ends_with(".md")) {
        return Err(format!("docs/{md}: Markdown sources are never published on Pages"));
    }
    Ok(out.into_iter().collect())
}

/// Build the publish list and check that published pages and the sitemap reference only published files.
pub fn check_publish_set(root: &Path) -> Result<Vec<String>, String> {
    let files = publish_list(root)?;
    let published: BTreeSet<&str> = files.iter().map(String::as_str).collect();
    let docs = root.join("docs");
    let mut problems = Vec::new();
    for rel in files.iter().filter(|rel| rel.ends_with(".html")) {
        let html = std::fs::read_to_string(docs.join(rel)).map_err(|e| format!("read docs/{rel}: {e}"))?;
        for target in local_references(&html) {
            match resolve(rel, &target) {
                Some(resolved) if published.contains(resolved.as_str()) => {}
                Some(resolved) => problems.push(format!("docs/{rel} links to `{target}` (docs/{resolved}), which is not published")),
                None => problems.push(format!("docs/{rel} links to `{target}`, which leaves the site root")),
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
    if problems.is_empty() { Ok(files) } else { Err(format!("Pages publish set has {} problem(s):\n  {}", problems.len(), problems.join("\n  "))) }
}

/// Copy the checked publish set into `out_dir`, which must be absent or empty and outside `docs/`.
pub fn stage(root: &Path, out_dir: &Path) -> Result<usize, String> {
    let docs = root.join("docs");
    if out_dir.starts_with(&docs) {
        return Err(format!("staging directory {} must be outside docs/", out_dir.display()));
    }
    if std::fs::symlink_metadata(out_dir).is_ok() {
        let mut entries = std::fs::read_dir(out_dir).map_err(|e| format!("read_dir {}: {e}", out_dir.display()))?;
        if entries.next().is_some() {
            return Err(format!("staging directory {} must be empty", out_dir.display()));
        }
    }
    let files = check_publish_set(root)?;
    for rel in &files {
        let bytes = std::fs::read(docs.join(rel)).map_err(|e| format!("read docs/{rel}: {e}"))?;
        let dst = out_dir.join(rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        write_atomic(&dst, &bytes).map_err(|e| format!("write {}: {e}", dst.display()))?;
    }
    let extra = stage_custom_domain_files(out_dir, SITE_BASE_URL, CUSTOM_DOMAIN)?;
    Ok(files.len() + extra)
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

/// `v<major>.<minor>` release-snapshot directories under `docs/manual/` that carry a `manifest.json`.
fn snapshot_dirs(manual: &Path) -> Result<Vec<String>, String> {
    if !std::fs::symlink_metadata(manual).is_ok_and(|m| m.file_type().is_dir()) {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(manual).map_err(|e| format!("read_dir {}: {e}", manual.display()))? {
        let entry = entry.map_err(|e| format!("dir entry under docs/manual: {e}"))?;
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

/// Collect regular files below `dir` as `/`-separated paths relative to `docs`; symlinks are refused.
fn collect_files(docs: &Path, dir: &Path, out: &mut BTreeSet<String>) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("dir entry under {}: {e}", dir.display()))?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|e| format!("file type of {}: {e}", path.display()))?;
        if file_type.is_symlink() {
            return Err(format!("{}: symlinks are never published on Pages", path.display()));
        }
        if file_type.is_dir() {
            collect_files(docs, &path, out)?;
        } else if file_type.is_file() {
            let rel = path.strip_prefix(docs).map_err(|e| e.to_string())?;
            let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            out.insert(parts.join("/"));
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

    /// A minimal site plus the internal files Pages must not publish.
    fn seed(label: &str) -> TempRoot {
        let root = TempRoot::new(label);
        root.write("docs/index.html", "<a href=\"open-pzfx-files.html#convert\">x</a><a href=\"faq.html\">faq</a><img src=\"images/a.png\"><a href=\"https://github.com/x\">gh</a><a href=\"#top\">t</a><a href=\"mailto:a@b.c\">m</a>");
        root.write("docs/open-pzfx-files.html", "<a href=\"manual/latest/files.html\">f</a><link rel=\"stylesheet\" href=\"site.css\">");
        root.write("docs/linux.html", "<a href=\"index.html\">h</a><a href=\"manual/latest/get-started.html\">gs</a>");
        root.write("docs/faq.html", "<a href=\"index.html\">h</a><a href=\"manual/latest/index.html\">m</a>");
        root.write("docs/privacy.html", "<a href=\"index.html\">h</a>");
        root.write("docs/get-started.html", "<meta http-equiv=\"refresh\" content=\"0; url=manual/latest/get-started.html\">");
        root.write("docs/troubleshooting.html", "<meta http-equiv=\"refresh\" content=\"0; url=manual/latest/troubleshooting.html#known-issues\">");
        root.write("docs/manual/index.html", "<meta http-equiv=\"refresh\" content=\"0; url=latest/index.html\">");
        root.write("docs/manual/latest/manifest.json", "{}");
        root.write("docs/manual/latest/index.html", "<a href=\"../../index.html\">home</a><a href=\"registry/formats.html\">r</a>");
        root.write("docs/manual/latest/files.html", "<link href=\"../../site.css\">");
        root.write("docs/manual/latest/get-started.html", "gs");
        root.write("docs/manual/latest/troubleshooting.html", "ts");
        root.write("docs/manual/latest/registry/formats.html", "<a href=\"../index.html\">up</a>");
        root.write("docs/manual/v0.1/manifest.json", "{}");
        root.write("docs/manual/v0.1/index.html", "<a href=\"../../index.html\">home</a><a href=\"registry/formats.html\">r</a>");
        root.write("docs/manual/v0.1/files.html", "<link href=\"../../site.css\">");
        root.write("docs/manual/v0.1/get-started.html", "gs");
        root.write("docs/manual/v0.1/troubleshooting.html", "ts");
        root.write("docs/manual/v0.1/registry/formats.html", "<a href=\"../index.html\">up</a>");
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
        root.write("docs/manual/scratch/notes.html", "not a release snapshot");
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

        let custom_out = root.0.join("_custom");
        std::fs::create_dir_all(&custom_out).unwrap();
        assert_eq!(stage_custom_domain_files(&custom_out, "https://curvebook.org/", Some("curvebook.org")).unwrap(), 2);
        assert_eq!(std::fs::read_to_string(custom_out.join("CNAME")).unwrap(), "curvebook.org\n");
        assert!(std::fs::read_to_string(custom_out.join("robots.txt")).unwrap().contains("Sitemap: https://curvebook.org/sitemap.xml"));
    }

    #[test]
    fn r12_9_links_to_unpublished_files_fail_the_check() {
        let root = seed("links");
        root.write("docs/privacy.html", "<a href=\"ROADMAP.md\">roadmap</a><a href=\"../outside.html\">up</a>");
        let err = check_publish_set(&root.0).unwrap_err();
        assert!(err.contains("`ROADMAP.md` (docs/ROADMAP.md), which is not published"), "{err}");
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
