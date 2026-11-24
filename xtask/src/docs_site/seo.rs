//! Search-engine metadata for the GitHub Pages site (`R12.8`): `docs/sitemap.xml` and canonical links.
//!
//! The sitemap lists the hand-written top-level pages in [`LANDING_PAGES`] plus every page of the
//! newest committed release snapshot under `docs/manual/v<major>.<minor>/`. Redirect shims
//! (`docs/get-started.html`, `docs/troubleshooting.html`, `docs/manual/index.html`) and older
//! snapshots are left out so search engines index one copy of each page. The output carries no
//! `<lastmod>` dates, so it is a pure function of the committed page set and `--check` can compare
//! it byte for byte.

use std::path::Path;

use curvebook_platform::write_atomic;

use super::verify::parse_minor_dir_version;

/// Published base URL of the GitHub Pages site (`main` branch, `/docs` folder).
pub const SITE_BASE_URL: &str = "https://hausergroup.github.io/curvebook/";

/// Hand-written top-level pages under `docs/` that search engines should index, in sitemap order.
/// Each must carry a canonical link to its own published URL.
pub const LANDING_PAGES: &[&str] = &["index.html", "open-pzfx-files.html", "privacy.html"];

/// Repository-relative path of the generated sitemap.
pub const SITEMAP_REL_PATH: &str = "docs/sitemap.xml";

/// Published URL for a `docs/`-relative page path; the site root maps to [`SITE_BASE_URL`].
#[must_use]
pub fn published_url(docs_rel: &str) -> String {
    if docs_rel == "index.html" { SITE_BASE_URL.to_owned() } else { format!("{SITE_BASE_URL}{docs_rel}") }
}

/// Exact canonical `<link>` element expected in the hand-written page at `docs_rel`.
#[must_use]
pub fn canonical_link_tag(docs_rel: &str) -> String {
    format!("<link rel=\"canonical\" href=\"{}\">", published_url(docs_rel))
}

/// Render `docs/sitemap.xml` from the committed `docs/` tree under `root`.
pub fn build_sitemap(root: &Path) -> Result<String, String> {
    let docs_dir = root.join("docs");
    let mut rel_paths: Vec<String> = Vec::new();
    for page in LANDING_PAGES {
        if !is_regular_file(&docs_dir.join(page)) {
            return Err(format!("docs/{page} is listed in LANDING_PAGES but missing"));
        }
        rel_paths.push((*page).to_owned());
    }
    if let Some(snapshot) = newest_snapshot_dir(&docs_dir.join("manual"))? {
        let snap_root = docs_dir.join("manual").join(&snapshot);
        let mut snap_pages = Vec::new();
        collect_html_files(&snap_root, &snap_root, &mut snap_pages)?;
        snap_pages.sort();
        rel_paths.extend(snap_pages.into_iter().map(|p| format!("manual/{snapshot}/{p}")));
    }

    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n");
    for rel in &rel_paths {
        if !rel.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'/' | b'-' | b'_')) {
            return Err(format!("docs/{rel}: page path must be plain ASCII (letters, digits, `.`, `/`, `-`, `_`) to appear in the sitemap"));
        }
        out.push_str("  <url><loc>");
        out.push_str(&published_url(rel));
        out.push_str("</loc></url>\n");
    }
    out.push_str("</urlset>\n");
    Ok(out)
}

/// Regenerate `docs/sitemap.xml` (`cargo xtask docs-site --sitemap`, and after `--release`).
pub fn write_sitemap(root: &Path) -> Result<usize, String> {
    let xml = build_sitemap(root)?;
    let path = root.join(SITEMAP_REL_PATH);
    write_atomic(&path, xml.as_bytes()).map_err(|e| format!("write {SITEMAP_REL_PATH}: {e}"))?;
    Ok(xml.matches("<url>").count())
}

/// `--check` gate: the committed sitemap matches the page set, and every landing page declares its canonical URL.
pub fn check_search_metadata(root: &Path) -> Result<(), String> {
    let expected = build_sitemap(root)?;
    let committed = std::fs::read_to_string(root.join(SITEMAP_REL_PATH))
        .map_err(|e| format!("read {SITEMAP_REL_PATH}: {e} (run `cargo xtask docs-site --sitemap`)"))?;
    if committed != expected {
        return Err(format!("{SITEMAP_REL_PATH} is out of date with the published pages; run `cargo xtask docs-site --sitemap`"));
    }
    for page in LANDING_PAGES {
        let html = std::fs::read_to_string(root.join("docs").join(page)).map_err(|e| format!("read docs/{page}: {e}"))?;
        let tag = canonical_link_tag(page);
        let lower = html.to_ascii_lowercase();
        let canonical_count = lower.matches("rel=\"canonical\"").count() + lower.matches("rel='canonical'").count();
        if canonical_count != 1 || !html.contains(&tag) {
            return Err(format!("docs/{page}: expected exactly one canonical link `{tag}`"));
        }
    }
    Ok(())
}

fn is_regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file())
}

/// Newest `docs/manual/v<major>.<minor>/` directory that carries a release `manifest.json`.
fn newest_snapshot_dir(manual_dir: &Path) -> Result<Option<String>, String> {
    if !std::fs::symlink_metadata(manual_dir).is_ok_and(|m| m.file_type().is_dir()) {
        return Ok(None);
    }
    let mut best: Option<((u64, u64), String)> = None;
    for entry in std::fs::read_dir(manual_dir).map_err(|e| format!("read_dir {}: {e}", manual_dir.display()))? {
        let entry = entry.map_err(|e| format!("dir entry under docs/manual: {e}"))?;
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(ver) = parse_minor_dir_version(&name)
            && is_regular_file(&entry.path().join("manifest.json"))
            && best.as_ref().is_none_or(|(b, _)| ver > *b)
        {
            best = Some((ver, name));
        }
    }
    Ok(best.map(|(_, name)| name))
}

/// Collect `/`-separated `.html` paths below `base`, skipping symlinks.
fn collect_html_files(base: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("dir entry under {}: {e}", dir.display()))?;
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if ft.is_dir() {
            collect_html_files(base, &path, out)?;
        } else if ft.is_file() && path.extension().is_some_and(|e| e == "html") {
            let rel = path.strip_prefix(base).map_err(|e| e.to_string())?;
            let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            out.push(parts.join("/"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        std::env::temp_dir().join(format!("cb-docs-seo-{label}-{}-{nanos}", std::process::id()))
    }

    fn write(root: &Path, rel: &str, body: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }

    fn seed_landing_pages(root: &Path) {
        for page in LANDING_PAGES {
            write(root, &format!("docs/{page}"), &format!("<html><head>{}</head></html>", canonical_link_tag(page)));
        }
    }

    #[test]
    fn sitemap_lists_landing_pages_and_only_the_newest_snapshot() {
        let root = temp_root("newest");
        seed_landing_pages(&root);
        write(&root, "docs/get-started.html", "shim");
        write(&root, "docs/manual/index.html", "redirect");
        write(&root, "docs/manual/v0.1/manifest.json", "{}");
        write(&root, "docs/manual/v0.1/index.html", "old");
        write(&root, "docs/manual/v0.2/manifest.json", "{}");
        write(&root, "docs/manual/v0.2/index.html", "new");
        write(&root, "docs/manual/v0.2/registry/formats.html", "new");
        write(&root, "docs/manual/v0.3/index.html", "no manifest, so not a release snapshot");

        let xml = build_sitemap(&root).unwrap();
        let locs: Vec<&str> = xml.lines().filter_map(|l| l.trim().strip_prefix("<url><loc>")?.strip_suffix("</loc></url>")).collect();
        assert_eq!(
            locs,
            [
                "https://hausergroup.github.io/curvebook/",
                "https://hausergroup.github.io/curvebook/open-pzfx-files.html",
                "https://hausergroup.github.io/curvebook/privacy.html",
                "https://hausergroup.github.io/curvebook/manual/v0.2/index.html",
                "https://hausergroup.github.io/curvebook/manual/v0.2/registry/formats.html",
            ]
        );
        assert!(!xml.contains("lastmod"), "sitemap must stay deterministic");

        write_sitemap(&root).unwrap();
        assert!(check_search_metadata(&root).is_ok());

        // A new snapshot page makes the committed sitemap stale.
        write(&root, "docs/manual/v0.2/glossary.html", "new");
        assert!(check_search_metadata(&root).unwrap_err().contains("--sitemap"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn landing_pages_need_exactly_one_matching_canonical_link() {
        let root = temp_root("canonical");
        seed_landing_pages(&root);
        write_sitemap(&root).unwrap();
        assert!(check_search_metadata(&root).is_ok());

        write(&root, "docs/privacy.html", "<link rel=\"canonical\" href=\"https://hausergroup.github.io/curvebook/\">");
        assert!(check_search_metadata(&root).unwrap_err().contains("docs/privacy.html"));

        let doubled = format!("{0}{0}", canonical_link_tag("privacy.html"));
        write(&root, "docs/privacy.html", &doubled);
        assert!(check_search_metadata(&root).unwrap_err().contains("exactly one"));

        std::fs::remove_file(root.join("docs/privacy.html")).unwrap();
        assert!(build_sitemap(&root).unwrap_err().contains("LANDING_PAGES"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
