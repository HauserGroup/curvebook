//! Search-engine and social metadata for the GitHub Pages site (`R12.8`, `R12.10`):
//! `docs/sitemap.xml`, `docs/llms.txt`, `docs/llms-full.txt`, canonical links, Open Graph tags,
//! and single-switch custom-domain helpers.
//!
//! The sitemap lists the hand-written top-level pages in [`LANDING_PAGES`] plus every page of
//! `docs/manual/latest/` (when `docs/manual/latest/manifest.json` exists, falling back to the
//! newest committed `docs/manual/v<major>.<minor>/` snapshot otherwise). Redirect shims
//! (`docs/get-started.html`, `docs/troubleshooting.html`, `docs/manual/index.html`) and older
//! snapshots are left out so search engines index one canonical copy of each page. The output
//! carries no `<lastmod>` dates, so it is a pure function of the committed page set and `--check`
//! can compare it byte for byte.

use std::collections::BTreeMap;
use std::path::Path;

use curvebook_platform::write_atomic;

use super::llms::{LLMS_FULL_TXT_REL_PATH, LLMS_TXT_REL_PATH, build_llms_full_txt, build_llms_txt};
use super::verify::parse_minor_dir_version;

/// The published base URL, optional custom domain, and landing pages live with the Pages publish
/// list (`R12.9`, `R12.10`), which builds without the `full-gates` feature.
pub use crate::pages_site::{CUSTOM_DOMAIN, LANDING_PAGES, SITE_BASE_URL};

/// Repository-relative path of the generated sitemap.
pub const SITEMAP_REL_PATH: &str = "docs/sitemap.xml";

/// Repository-relative path of the generated custom 404 page.
pub const NOT_FOUND_REL_PATH: &str = "docs/404.html";

/// Published URL for a `docs/`-relative page path; the site root maps to [`SITE_BASE_URL`].
#[must_use]
pub fn published_url(docs_rel: &str) -> String {
    if docs_rel == "index.html" { SITE_BASE_URL.to_owned() } else { format!("{SITE_BASE_URL}{docs_rel}") }
}

/// Stable canonical URL for a generated manual page under `/manual/latest/`.
#[must_use]
pub fn manual_canonical_url(rel_path: &str) -> String {
    let clean = rel_path.trim_start_matches('/');
    format!("{SITE_BASE_URL}manual/latest/{clean}")
}

/// Exact canonical `<link>` element expected in the hand-written page at `docs_rel`.
#[must_use]
pub fn canonical_link_tag(docs_rel: &str) -> String {
    format!("<link rel=\"canonical\" href=\"{}\">", published_url(docs_rel))
}

/// Render a GitHub Pages `CNAME` file for a custom domain.
#[must_use]
pub fn build_cname(domain: &str) -> String {
    format!("{}\n", domain.trim())
}

/// Render `robots.txt` pointing crawlers at the site's `sitemap.xml`.
#[must_use]
pub fn build_robots_txt(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    format!("User-agent: *\nAllow: /\n\nSitemap: {trimmed}/sitemap.xml\n")
}

/// Validate consistency between `SITE_BASE_URL` and `CUSTOM_DOMAIN`.
pub fn validate_site_config(site_base_url: &str, custom_domain: Option<&str>) -> Result<(), String> {
    if !site_base_url.starts_with("https://") || !site_base_url.ends_with('/') {
        return Err(format!("SITE_BASE_URL `{site_base_url}` must start with `https://` and end with `/`"));
    }
    if let Some(domain) = custom_domain {
        let trimmed = domain.trim();
        if trimmed.is_empty() || trimmed.contains('/') || trimmed.contains("://") || trimmed.chars().any(char::is_whitespace) {
            return Err(format!("CUSTOM_DOMAIN `{domain}` must be a bare hostname without scheme or path"));
        }
        let expected_base = format!("https://{trimmed}/");
        if site_base_url != expected_base {
            return Err(format!("CUSTOM_DOMAIN `{trimmed}` requires SITE_BASE_URL `{expected_base}`, found `{site_base_url}`"));
        }
    }
    Ok(())
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
    let manual_dir = docs_dir.join("manual");
    let snapshot_sub =
        if is_regular_file(&manual_dir.join("latest/manifest.json")) { Some(String::from("latest")) } else { newest_snapshot_dir(&manual_dir)? };
    if let Some(snapshot) = snapshot_sub {
        let snap_root = manual_dir.join(&snapshot);
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

/// Regenerate `docs/404.html` (`cargo xtask docs-site --sitemap`, and after `--release`).
pub fn write_not_found_page(root: &Path) -> Result<(), String> {
    let html = super::template::render_not_found_page(crate::pages_site::site_base_path());
    let path = root.join(NOT_FOUND_REL_PATH);
    write_atomic(&path, html.as_bytes()).map_err(|e| format!("write {NOT_FOUND_REL_PATH}: {e}"))
}

/// `--check` gate: the committed sitemap, `404.html`, and `llms*.txt` match the published content byte-for-byte,
/// and every landing page declares its canonical link and Open Graph / Twitter Card metadata.
pub fn check_search_metadata(root: &Path) -> Result<(), String> {
    validate_site_config(SITE_BASE_URL, CUSTOM_DOMAIN)?;

    let expected_sitemap = build_sitemap(root)?;
    let committed_sitemap = std::fs::read_to_string(root.join(SITEMAP_REL_PATH))
        .map_err(|e| format!("read {SITEMAP_REL_PATH}: {e} (run `cargo xtask docs-site --sitemap`)"))?;
    if committed_sitemap != expected_sitemap {
        return Err(format!("{SITEMAP_REL_PATH} is out of date with the published pages; run `cargo xtask docs-site --sitemap`"));
    }

    let expected_404 = super::template::render_not_found_page(crate::pages_site::site_base_path());
    let committed_404 = std::fs::read_to_string(root.join(NOT_FOUND_REL_PATH))
        .map_err(|e| format!("read {NOT_FOUND_REL_PATH}: {e} (run `cargo xtask docs-site --sitemap`)"))?;
    if committed_404 != expected_404 {
        return Err(format!("{NOT_FOUND_REL_PATH} is out of date; run `cargo xtask docs-site --sitemap`"));
    }

    let expected_llms = build_llms_txt(root)?;
    let committed_llms = std::fs::read_to_string(root.join(LLMS_TXT_REL_PATH))
        .map_err(|e| format!("read {LLMS_TXT_REL_PATH}: {e} (run `cargo xtask docs-site --sitemap`)"))?;
    if committed_llms != expected_llms {
        return Err(format!("{LLMS_TXT_REL_PATH} is out of date; run `cargo xtask docs-site --sitemap`"));
    }

    let expected_llms_full = build_llms_full_txt(root)?;
    let committed_llms_full = std::fs::read_to_string(root.join(LLMS_FULL_TXT_REL_PATH))
        .map_err(|e| format!("read {LLMS_FULL_TXT_REL_PATH}: {e} (run `cargo xtask docs-site --sitemap`)"))?;
    if committed_llms_full != expected_llms_full {
        return Err(format!("{LLMS_FULL_TXT_REL_PATH} is out of date; run `cargo xtask docs-site --sitemap`"));
    }

    for page in LANDING_PAGES {
        let html = std::fs::read_to_string(root.join("docs").join(page)).map_err(|e| format!("read docs/{page}: {e}"))?;
        let tag = canonical_link_tag(page);
        let lower = html.to_ascii_lowercase();
        let canonical_count = lower.matches("rel=\"canonical\"").count() + lower.matches("rel='canonical'").count();
        if canonical_count != 1 || !html.contains(&tag) {
            return Err(format!("docs/{page}: expected exactly one canonical link `{tag}`"));
        }
        check_landing_page_social_meta(root, page, &html)?;
    }
    Ok(())
}

fn check_landing_page_social_meta(root: &Path, page: &str, html: &str) -> Result<(), String> {
    let metas = extract_meta_map(html);
    for required in ["og:type", "og:title", "og:description", "og:url", "og:image", "twitter:card"] {
        match metas.get(required) {
            Some(val) if !val.trim().is_empty() => {}
            _ => return Err(format!("docs/{page}: missing or empty `<meta>` tag for `{required}`")),
        }
    }

    let expected_url = published_url(page);
    let actual_url = metas.get("og:url").map(String::as_str).unwrap_or("");
    if actual_url != expected_url {
        return Err(format!("docs/{page}: `og:url` is `{actual_url}`, expected `{expected_url}`"));
    }

    let og_image = metas.get("og:image").map(String::as_str).unwrap_or("");
    let Some(rel_img) = og_image.strip_prefix(SITE_BASE_URL) else {
        return Err(format!("docs/{page}: `og:image` `{og_image}` must start with `{SITE_BASE_URL}`"));
    };
    if !rel_img.starts_with("images/") || rel_img.split('/').any(|seg| seg.is_empty() || seg == "..") {
        return Err(format!("docs/{page}: `og:image` `{og_image}` must point to a file under `docs/images/`"));
    }
    let img_path = root.join("docs").join(rel_img);
    if !is_regular_file(&img_path) {
        return Err(format!("docs/{page}: `og:image` target `docs/{rel_img}` does not exist"));
    }

    Ok(())
}

/// Extract `<meta property="..." content="...">` and `<meta name="..." content="...">` pairs.
fn extract_meta_map(html: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let mut rest = html;
    while let Some((_, after_lt)) = rest.split_once('<') {
        if let Some(after_comment) = after_lt.strip_prefix("!--") {
            let Some((_, after_close)) = after_comment.split_once("-->") else {
                break;
            };
            rest = after_close;
            continue;
        }
        let Some(tag_end) = find_tag_end(after_lt) else {
            break;
        };
        let (tag_body, tail) = after_lt.split_at(tag_end);
        rest = tail.strip_prefix('>').unwrap_or(tail);

        let trimmed = tag_body.trim();
        let tag_name = trimmed.split_whitespace().next().unwrap_or("");
        if !tag_name.eq_ignore_ascii_case("meta") {
            continue;
        }
        let attrs = parse_meta_attrs(trimmed);
        let mut key: Option<String> = None;
        let mut content: Option<String> = None;
        for (k, v) in attrs {
            if k.eq_ignore_ascii_case("property") || k.eq_ignore_ascii_case("name") {
                key = Some(v.to_ascii_lowercase());
            } else if k.eq_ignore_ascii_case("content") {
                content = Some(v);
            }
        }
        if let (Some(k), Some(c)) = (key, content) {
            map.insert(k, c);
        }
    }
    map
}

fn find_tag_end(rest: &str) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (idx, &b) in rest.as_bytes().iter().enumerate() {
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
        } else if b == b'"' || b == b'\'' {
            quote = Some(b);
        } else if b == b'>' {
            return Some(idx);
        }
    }
    None
}

fn parse_meta_attrs(trimmed_tag: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    let bytes = trimmed_tag.as_bytes();
    let mut i = bytes.iter().position(u8::is_ascii_whitespace).unwrap_or(bytes.len());
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let k_start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' && bytes[i] != b'/' {
            i += 1;
        }
        let key = trimmed_tag.get(k_start..i).unwrap_or("").to_owned();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
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
            raw
        } else {
            let v_start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            trimmed_tag.get(v_start..i).unwrap_or("").to_owned()
        };
        attrs.push((key, val));
    }
    attrs
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
    use super::super::llms::write_llms_files;
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

    fn valid_landing_html(page: &str) -> String {
        let url = published_url(page);
        let img = format!("{SITE_BASE_URL}images/workspace-split-fit.png");
        format!(
            "<html><head>\
             {}\
             <meta property=\"og:type\" content=\"website\">\
             <meta property=\"og:title\" content=\"CurveBook\">\
             <meta property=\"og:description\" content=\"CurveBook biostatistics.\">\
             <meta property=\"og:url\" content=\"{url}\">\
             <meta property=\"og:image\" content=\"{img}\">\
             <meta name=\"twitter:card\" content=\"summary_large_image\">\
             </head></html>",
            canonical_link_tag(page),
        )
    }

    fn seed_landing_pages(root: &Path) {
        write(root, "docs/images/workspace-split-fit.png", "png");
        for page in LANDING_PAGES {
            write(root, &format!("docs/{page}"), &valid_landing_html(page));
        }
    }

    #[test]
    fn site_config_and_custom_domain_helpers_stay_consistent() {
        assert!(validate_site_config(SITE_BASE_URL, CUSTOM_DOMAIN).is_ok());
        assert!(validate_site_config("https://curvebook.org/", Some("curvebook.org")).is_ok());
        assert!(validate_site_config(SITE_BASE_URL, Some("curvebook.org")).is_err());
        assert!(validate_site_config("https://curvebook.org/", Some("https://curvebook.org")).is_err());
        assert!(validate_site_config("http://curvebook.org/", None).is_err());
        assert_eq!(build_cname(" curvebook.org "), "curvebook.org\n");
        assert_eq!(build_robots_txt("https://curvebook.org/"), "User-agent: *\nAllow: /\n\nSitemap: https://curvebook.org/sitemap.xml\n");
        assert_eq!(manual_canonical_url("analyses.html"), "https://hausergroup.github.io/curvebook/manual/latest/analyses.html");
    }

    #[test]
    fn sitemap_prefers_latest_and_falls_back_to_newest_versioned_snapshot() {
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
                "https://hausergroup.github.io/curvebook/linux.html",
                "https://hausergroup.github.io/curvebook/faq.html",
                "https://hausergroup.github.io/curvebook/privacy.html",
                "https://hausergroup.github.io/curvebook/manual/v0.2/index.html",
                "https://hausergroup.github.io/curvebook/manual/v0.2/registry/formats.html",
            ]
        );
        assert!(!xml.contains("lastmod"), "sitemap must stay deterministic");

        // Adding docs/manual/latest/ switches sitemap indexing to /manual/latest/.
        write(&root, "docs/manual/latest/manifest.json", "{}");
        write(&root, "docs/manual/latest/index.html", "latest");
        write(&root, "docs/manual/latest/registry/formats.html", "latest");
        let latest_xml = build_sitemap(&root).unwrap();
        let latest_locs: Vec<&str> = latest_xml.lines().filter_map(|l| l.trim().strip_prefix("<url><loc>")?.strip_suffix("</loc></url>")).collect();
        assert_eq!(
            latest_locs,
            [
                "https://hausergroup.github.io/curvebook/",
                "https://hausergroup.github.io/curvebook/open-pzfx-files.html",
                "https://hausergroup.github.io/curvebook/linux.html",
                "https://hausergroup.github.io/curvebook/faq.html",
                "https://hausergroup.github.io/curvebook/privacy.html",
                "https://hausergroup.github.io/curvebook/manual/latest/index.html",
                "https://hausergroup.github.io/curvebook/manual/latest/registry/formats.html",
            ]
        );

        write_sitemap(&root).unwrap();
        write_not_found_page(&root).unwrap();
        write_llms_files(&root).unwrap();
        assert!(check_search_metadata(&root).is_ok());

        // A new snapshot page makes the committed sitemap stale.
        write(&root, "docs/manual/latest/glossary.html", "new");
        assert!(check_search_metadata(&root).unwrap_err().contains("--sitemap"));

        // Regenerate sitemap, then stale 404.html and llms.txt must fail check_search_metadata.
        write_sitemap(&root).unwrap();
        write(&root, NOT_FOUND_REL_PATH, "stale 404\n");
        assert!(check_search_metadata(&root).unwrap_err().contains("docs/404.html"));
        write_not_found_page(&root).unwrap();
        write(&root, LLMS_TXT_REL_PATH, "stale\n");
        assert!(check_search_metadata(&root).unwrap_err().contains("llms.txt"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn landing_pages_validate_canonical_and_open_graph_metadata() {
        let root = temp_root("canonical");
        seed_landing_pages(&root);
        write_sitemap(&root).unwrap();
        write_not_found_page(&root).unwrap();
        write_llms_files(&root).unwrap();
        assert!(check_search_metadata(&root).is_ok());

        // Wrong canonical URL.
        write(&root, "docs/privacy.html", "<link rel=\"canonical\" href=\"https://hausergroup.github.io/curvebook/\">");
        assert!(check_search_metadata(&root).unwrap_err().contains("docs/privacy.html"));

        // Duplicate canonical link.
        let doubled = format!("{0}{0}", canonical_link_tag("privacy.html"));
        write(&root, "docs/privacy.html", &doubled);
        assert!(check_search_metadata(&root).unwrap_err().contains("exactly one"));

        // Mismatched og:url.
        let bad_og_url = valid_landing_html("privacy.html").replace("privacy.html\"", "index.html\"");
        // Keep canonical valid, break only og:url.
        let bad_og_url = bad_og_url.replacen("index.html\"", "privacy.html\"", 1);
        write(&root, "docs/privacy.html", &bad_og_url);
        assert!(check_search_metadata(&root).unwrap_err().contains("og:url"));

        // Missing og:image file under docs/images/.
        let bad_og_img = valid_landing_html("privacy.html").replace("workspace-split-fit.png", "does-not-exist.png");
        write(&root, "docs/privacy.html", &bad_og_img);
        assert!(check_search_metadata(&root).unwrap_err().contains("og:image"));

        // Missing twitter:card.
        let no_twitter = valid_landing_html("privacy.html").replace("twitter:card", "other:card");
        write(&root, "docs/privacy.html", &no_twitter);
        assert!(check_search_metadata(&root).unwrap_err().contains("twitter:card"));

        std::fs::remove_file(root.join("docs/privacy.html")).unwrap();
        assert!(build_sitemap(&root).unwrap_err().contains("LANDING_PAGES"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
