//! Search-engine and social metadata for the GitHub Pages site (`R12.8`, `R12.10`):
//! `docs/sitemap.xml`, `docs/llms.txt`, `docs/llms-full.txt`, canonical links, Open Graph tags,
//! and single-switch custom-domain helpers.
//!
//! The sitemap lists the hand-written top-level pages in [`LANDING_PAGES`] plus every page of
//! `manual/latest/` (read from the generated manual build directory or the assembled manual page
//! list, falling back to the newest `v<major>.<minor>/` snapshot when `latest/manifest.json` is
//! absent). Redirect shims (`docs/get-started.html`, `docs/troubleshooting.html`,
//! `manual/index.html`) and versioned snapshots are left out so search engines index one canonical
//! copy of each page. The output carries no `<lastmod>` dates, so it is a pure function of the
//! published page set and `--check` can compare it byte for byte.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use curvebook_platform::write_atomic;

use super::llms::{build_llms_full_txt, build_llms_txt};
use crate::pages_site::{extract_html_tag_name, find_html_tag_close, parse_html_tag_attributes, parse_minor_dir_version};
use crate::scan_util::{is_real_dir, is_regular_file, read_dir_paths_sorted, walk_rel_files};

/// The published base URL, optional custom domain, and landing pages live with the Pages publish
/// list (`R12.9`, `R12.10`), which builds without the `full-gates` feature.
pub use crate::pages_site::{CUSTOM_DOMAIN, LANDING_PAGES, SITE_BASE_URL};

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

/// Validate consistency between `SITE_BASE_URL` and `CUSTOM_DOMAIN`.
pub fn validate_site_config(site_base_url: &str, custom_domain: Option<&str>) -> Result<(), String> {
    if !site_base_url.starts_with("https://") || !site_base_url.ends_with('/') {
        return Err(format!("SITE_BASE_URL `{site_base_url}` must start with `https://` and end with `/`"));
    }
    if site_base_url.chars().any(|c| c.is_whitespace() || c.is_ascii_control() || matches!(c, '"' | '\'' | '<' | '>' | '&')) {
        return Err(format!("SITE_BASE_URL `{site_base_url}` must not contain whitespace, control chars, or HTML metacharacters"));
    }
    if let Some(domain) = custom_domain {
        let trimmed = domain.trim();
        if trimmed.is_empty()
            || trimmed.contains('/')
            || trimmed.contains("://")
            || trimmed.chars().any(|c| c.is_whitespace() || c.is_ascii_control() || matches!(c, '"' | '\'' | '<' | '>' | '&'))
        {
            return Err(format!("CUSTOM_DOMAIN `{domain}` must be a bare hostname without scheme, path, or HTML metacharacters"));
        }
        let expected_base = format!("https://{trimmed}/");
        if site_base_url != expected_base {
            return Err(format!("CUSTOM_DOMAIN `{trimmed}` requires SITE_BASE_URL `{expected_base}`, found `{site_base_url}`"));
        }
    }
    Ok(())
}

/// Render `docs/sitemap.xml` from `LANDING_PAGES` plus the supplied manual page paths under `manual/<snapshot_sub>/`.
pub fn build_sitemap_for_manual_pages(root: &Path, snapshot_sub: &str, manual_rel_pages: &[String]) -> Result<String, String> {
    let docs_dir = root.join("docs");
    let mut rel_paths: Vec<String> = Vec::new();
    for page in LANDING_PAGES {
        if !is_regular_file(&docs_dir.join(page)) {
            return Err(format!("docs/{page} is listed in LANDING_PAGES but missing"));
        }
        rel_paths.push((*page).to_owned());
    }
    let mut snap_pages: Vec<String> = manual_rel_pages.iter().filter(|p| p.ends_with(".html")).cloned().collect();
    snap_pages.sort();
    snap_pages.dedup();
    rel_paths.extend(snap_pages.into_iter().map(|p| format!("manual/{snapshot_sub}/{p}")));

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

/// Render `docs/sitemap.xml` from `LANDING_PAGES` plus generated manual pages under `manual_dir`
/// (or default `target/docs-manual`, falling back to `docs/manual` when absent).
pub fn build_sitemap_with_manual(root: &Path, manual_dir: Option<&Path>) -> Result<String, String> {
    let resolved_manual = match manual_dir {
        Some(dir) => Some(dir.to_path_buf()),
        None => {
            let default_dir = crate::pages_site::default_manual_dir(root);
            if is_real_dir(&default_dir) {
                Some(default_dir)
            } else {
                let legacy_dir = root.join("docs/manual");
                is_real_dir(&legacy_dir).then_some(legacy_dir)
            }
        }
    };
    if let Some(manual_root) = resolved_manual {
        let latest_dir = manual_root.join("latest");
        if let Ok(meta) = std::fs::symlink_metadata(&latest_dir)
            && meta.file_type().is_symlink()
        {
            return Err(format!("{}: symlinked `latest/` directory is forbidden in sitemap", latest_dir.display()));
        }
        let latest_manifest = latest_dir.join("manifest.json");
        if let Ok(meta) = std::fs::symlink_metadata(&latest_manifest)
            && meta.file_type().is_symlink()
        {
            return Err(format!("{}: symlinked `manifest.json` is forbidden in sitemap", latest_manifest.display()));
        }
        let snapshot_sub = if is_real_dir(&latest_dir) && is_regular_file(&latest_manifest) {
            Some(String::from("latest"))
        } else {
            newest_snapshot_dir(&manual_root)?
        };
        if let Some(snapshot) = snapshot_sub {
            let snap_root = manual_root.join(&snapshot);
            let mut snap_pages = Vec::new();
            walk_rel_files(&snap_root, &snap_root, &|_| false, &|p| p.extension().is_some_and(|e| e == "html"), &mut snap_pages)?;
            return build_sitemap_for_manual_pages(root, &snapshot, &snap_pages);
        }
        if manual_dir.is_some() {
            return Err(format!("manual directory `{}` has no `latest/` or `v<major>.<minor>/` snapshot for sitemap", manual_root.display()));
        }
    }
    build_sitemap_for_manual_pages(root, "latest", &[])
}

/// Regenerate `sitemap.xml` inside `out_dir` (e.g. `<manual_dir>/_root/`) using `manual_dir`.
pub fn write_sitemap_into(root: &Path, manual_dir: Option<&Path>, out_dir: &Path) -> Result<usize, String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;
    let xml = build_sitemap_with_manual(root, manual_dir)?;
    let path = out_dir.join("sitemap.xml");
    write_atomic(&path, xml.as_bytes()).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(xml.matches("<url>").count())
}

/// Regenerate `docs/404.html` (`cargo xtask docs-site --sitemap`, and after `--release`).
pub fn write_not_found_page(root: &Path) -> Result<(), String> {
    let html = super::template::render_not_found_page(crate::pages_site::site_base_path());
    let path = root.join(NOT_FOUND_REL_PATH);
    write_atomic(&path, html.as_bytes()).map_err(|e| format!("write {NOT_FOUND_REL_PATH}: {e}"))
}

/// `--check` gate validating `<manual_dir>/_root/{sitemap.xml,llms.txt,llms-full.txt}`, committed
/// `docs/404.html`, and landing-page canonical/Open Graph metadata.
pub fn check_search_metadata_with_manual(root: &Path, manual_dir: Option<&Path>) -> Result<(), String> {
    validate_site_config(SITE_BASE_URL, CUSTOM_DOMAIN)?;

    let resolved_manual = manual_dir.map(Path::to_path_buf).unwrap_or_else(|| crate::pages_site::default_manual_dir(root));
    let generated_root = resolved_manual.join(crate::pages_site::MANUAL_ROOT_SUBDIR);

    for (name, expected, reason) in [
        ("sitemap.xml", build_sitemap_with_manual(root, manual_dir)?, "is out of date with the published pages"),
        ("llms.txt", build_llms_txt(root)?, "is out of date"),
        ("llms-full.txt", build_llms_full_txt(root)?, "is out of date"),
    ] {
        let path = generated_root.join(name);
        let actual = std::fs::read_to_string(&path)
            .map_err(|e| format!("read {}: {e} (run `cargo run -p xtask --features docs-site -- docs-site --build-manual`)", path.display()))?;
        if actual != expected {
            return Err(format!("{} {reason}; rebuild with `docs-site --build-manual`", path.display()));
        }
    }

    let expected_404 = super::template::render_not_found_page(crate::pages_site::site_base_path());
    let committed_404 = std::fs::read_to_string(root.join(NOT_FOUND_REL_PATH))
        .map_err(|e| format!("read {NOT_FOUND_REL_PATH}: {e} (run `cargo xtask docs-site --sitemap`)"))?;
    if committed_404 != expected_404 {
        return Err(format!("{NOT_FOUND_REL_PATH} is out of date; run `cargo xtask docs-site --sitemap`"));
    }

    for page in LANDING_PAGES {
        let html = std::fs::read_to_string(root.join("docs").join(page)).map_err(|e| format!("read docs/{page}: {e}"))?;
        let tag = canonical_link_tag(page);
        let expected_url = published_url(page);
        let canonical_hrefs = extract_canonical_hrefs(page, &html)?;
        if canonical_hrefs.len() != 1 || canonical_hrefs[0] != expected_url {
            return Err(format!("docs/{page}: expected exactly one canonical link `{tag}`"));
        }
        check_landing_page_social_meta(root, page, &html)?;
    }
    Ok(())
}

fn extract_canonical_hrefs(page: &str, html: &str) -> Result<Vec<String>, String> {
    let mut hrefs = Vec::new();
    let mut rest = html;
    while let Some((_, after_lt)) = rest.split_once('<') {
        if !crate::pages_site::is_html_tag_start(after_lt) {
            rest = after_lt;
            continue;
        }
        if let Some(after_comment) = after_lt.strip_prefix("!--") {
            let Some(close_end) = crate::pages_site::find_html_comment_end(after_comment) else {
                return Err(format!("docs/{page}: unclosed HTML comment `<!--`"));
            };
            rest = after_comment.get(close_end..).ok_or_else(|| format!("docs/{page}: unclosed HTML comment `<!--`"))?;
            continue;
        }
        let Some(tag_end) = find_html_tag_close(after_lt) else {
            return Err(format!("docs/{page}: unclosed HTML tag or attribute quote"));
        };
        let Some((tag_body, tail)) = after_lt.split_at_checked(tag_end) else {
            return Err(format!("docs/{page}: unclosed HTML tag or attribute quote"));
        };
        rest = tail.strip_prefix('>').unwrap_or(tail);
        let trimmed = tag_body.trim();
        if trimmed.starts_with('/') {
            continue;
        }
        let tag_lower = extract_html_tag_name(trimmed).to_ascii_lowercase();
        if matches!(tag_lower.as_str(), "title" | "textarea" | "script" | "style") {
            let (_, after_close) =
                crate::pages_site::skip_html_raw_or_rcdata_element(rest, 0, &tag_lower).map_err(|e| format!("docs/{page}: {e}"))?;
            rest = rest.get(after_close..).unwrap_or("");
            continue;
        }
        if tag_lower != "link" {
            continue;
        }
        let mut rel_val: Option<String> = None;
        let mut href_val: Option<String> = None;
        for (k, v) in parse_html_tag_attributes(trimmed) {
            if k.eq_ignore_ascii_case("rel") {
                if rel_val.is_some() {
                    return Err(format!("docs/{page}: duplicate `rel` attribute on `<link>`"));
                }
                rel_val = Some(v);
            } else if k.eq_ignore_ascii_case("href") {
                if href_val.is_some() {
                    return Err(format!("docs/{page}: duplicate `href` attribute on `<link>`"));
                }
                href_val = Some(v);
            }
        }
        if rel_val.as_deref().is_some_and(|r| r.split_whitespace().any(|tok| tok.eq_ignore_ascii_case("canonical"))) {
            hrefs.push(href_val.unwrap_or_default());
        }
    }
    Ok(hrefs)
}

fn check_landing_page_social_meta(root: &Path, page: &str, html: &str) -> Result<(), String> {
    let metas = extract_meta_map(page, html)?;
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

/// Extract `<meta property="..." content="...">` and `<meta name="..." content="...">` pairs,
/// rejecting duplicate social (`og:*`/`twitter:*`) keys and duplicate attributes on one `<meta>`.
fn extract_meta_map(page: &str, html: &str) -> Result<BTreeMap<String, String>, String> {
    let mut map = BTreeMap::new();
    let mut seen_social: BTreeSet<String> = BTreeSet::new();
    let mut rest = html;
    while let Some((_, after_lt)) = rest.split_once('<') {
        if !crate::pages_site::is_html_tag_start(after_lt) {
            rest = after_lt;
            continue;
        }
        if let Some(after_comment) = after_lt.strip_prefix("!--") {
            let Some(close_end) = crate::pages_site::find_html_comment_end(after_comment) else {
                return Err(format!("docs/{page}: unclosed HTML comment `<!--`"));
            };
            rest = after_comment.get(close_end..).ok_or_else(|| format!("docs/{page}: unclosed HTML comment `<!--`"))?;
            continue;
        }
        let Some(tag_end) = find_html_tag_close(after_lt) else {
            return Err(format!("docs/{page}: unclosed HTML tag or attribute quote"));
        };
        let Some((tag_body, tail)) = after_lt.split_at_checked(tag_end) else {
            return Err(format!("docs/{page}: unclosed HTML tag or attribute quote"));
        };
        rest = tail.strip_prefix('>').unwrap_or(tail);

        let trimmed = tag_body.trim();
        if trimmed.starts_with('/') {
            continue;
        }
        let tag_lower = extract_html_tag_name(trimmed).to_ascii_lowercase();
        if matches!(tag_lower.as_str(), "title" | "textarea" | "script" | "style") {
            let (_, after_close) =
                crate::pages_site::skip_html_raw_or_rcdata_element(rest, 0, &tag_lower).map_err(|e| format!("docs/{page}: {e}"))?;
            rest = rest.get(after_close..).unwrap_or("");
            continue;
        }
        if tag_lower != "meta" {
            continue;
        }
        let mut key: Option<String> = None;
        let mut content: Option<String> = None;
        for (k, v) in parse_html_tag_attributes(trimmed) {
            if k.eq_ignore_ascii_case("property") || k.eq_ignore_ascii_case("name") {
                if key.is_none() {
                    key = Some(v.to_ascii_lowercase());
                }
            } else if k.eq_ignore_ascii_case("content") {
                if content.is_some() {
                    return Err(format!("docs/{page}: duplicate `content` attribute on `<meta>`"));
                }
                content = Some(v);
            }
        }
        if let (Some(k), Some(c)) = (key, content) {
            if (k.starts_with("og:") || k.starts_with("twitter:")) && !seen_social.insert(k.clone()) {
                return Err(format!("docs/{page}: duplicate `<meta>` tag for `{k}`"));
            }
            map.entry(k).or_insert(c);
        }
    }
    Ok(map)
}

/// Newest `docs/manual/v<major>.<minor>/` directory that carries a release `manifest.json`.
fn newest_snapshot_dir(manual_dir: &Path) -> Result<Option<String>, String> {
    if !is_real_dir(manual_dir) {
        return Ok(None);
    }
    let mut best: Option<((u64, u64), String)> = None;
    for path in read_dir_paths_sorted(manual_dir)? {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if let Some(ver) = parse_minor_dir_version(&name) {
            let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("stat {}: {e}", path.display()))?;
            if meta.file_type().is_symlink() {
                return Err(format!("{}: symlinked snapshot directory is forbidden in sitemap", path.display()));
            }
            let manifest = path.join("manifest.json");
            if let Ok(m_meta) = std::fs::symlink_metadata(&manifest)
                && m_meta.file_type().is_symlink()
            {
                return Err(format!("{}: symlinked `manifest.json` is forbidden in sitemap", manifest.display()));
            }
            if meta.file_type().is_dir() && is_regular_file(&manifest) && best.as_ref().is_none_or(|(b, _)| ver > *b) {
                best = Some((ver, name));
            }
        }
    }
    Ok(best.map(|(_, name)| name))
}

#[cfg(test)]
mod tests {
    use super::super::llms::write_llms_files_into;
    use super::*;
    use crate::scan_util::test_support::TestDir;

    fn valid_landing_html(page: &str) -> String {
        let url = published_url(page);
        let img = format!("{SITE_BASE_URL}images/workspace-split-fit.png");
        format!(
            "<html><head>{}<meta property=\"og:type\" content=\"website\"><meta property=\"og:title\" content=\"CurveBook\"><meta property=\"og:description\" content=\"CurveBook biostatistics.\"><meta property=\"og:url\" content=\"{url}\"><meta property=\"og:image\" content=\"{img}\"><meta name=\"twitter:card\" content=\"summary_large_image\"></head></html>",
            canonical_link_tag(page),
        )
    }

    fn seed_landing_pages(root: &TestDir) {
        root.write("docs/images/workspace-split-fit.png", "png");
        for page in LANDING_PAGES {
            root.write(&format!("docs/{page}"), &valid_landing_html(page));
        }
    }

    #[test]
    fn site_config_and_custom_domain_helpers_stay_consistent() {
        assert!(validate_site_config(SITE_BASE_URL, CUSTOM_DOMAIN).is_ok());
        assert!(validate_site_config("https://curvebook.org/", Some("curvebook.org")).is_ok());
        assert!(validate_site_config("https://hausergroup.github.io/curvebook/", None).is_ok());
        assert!(validate_site_config("https://hausergroup.github.io/curvebook/", Some("curvebook.org")).is_err());
        assert!(validate_site_config("https://curvebook.org/", Some("https://curvebook.org")).is_err());
        assert!(validate_site_config("http://curvebook.org/", None).is_err());
        assert_eq!(manual_canonical_url("analyses.html"), "https://curvebook.org/manual/latest/analyses.html");
    }

    #[test]
    fn sitemap_prefers_latest_and_falls_back_to_newest_versioned_snapshot() {
        let root = TestDir::new("docs-seo-newest");
        seed_landing_pages(&root);
        for (rel, body) in [
            ("docs/get-started.html", "shim"),
            ("target/docs-manual/index.html", "redirect"),
            ("target/docs-manual/v0.1/manifest.json", "{}"),
            ("target/docs-manual/v0.1/index.html", "old"),
            ("target/docs-manual/v0.2/manifest.json", "{}"),
            ("target/docs-manual/v0.2/index.html", "new"),
            ("target/docs-manual/v0.2/registry/formats.html", "new"),
            ("target/docs-manual/v0.3/index.html", "no manifest"),
        ] {
            root.write(rel, body);
        }
        let extract_locs = |xml: &str| -> Vec<String> {
            xml.lines().filter_map(|l| l.trim().strip_prefix("<url><loc>")?.strip_suffix("</loc></url>").map(str::to_owned)).collect()
        };
        let xml = build_sitemap_with_manual(root.path(), None).unwrap();
        assert_eq!(
            extract_locs(&xml),
            [
                "https://curvebook.org/",
                "https://curvebook.org/open-pzfx-files.html",
                "https://curvebook.org/linux.html",
                "https://curvebook.org/faq.html",
                "https://curvebook.org/privacy.html",
                "https://curvebook.org/manual/v0.2/index.html",
                "https://curvebook.org/manual/v0.2/registry/formats.html",
            ]
        );
        assert!(!xml.contains("lastmod"), "sitemap must stay deterministic");

        root.write("target/docs-manual/latest/manifest.json", "{}");
        root.write("target/docs-manual/latest/index.html", "latest");
        root.write("target/docs-manual/latest/registry/formats.html", "latest");
        assert_eq!(
            extract_locs(&build_sitemap_with_manual(root.path(), None).unwrap()),
            [
                "https://curvebook.org/",
                "https://curvebook.org/open-pzfx-files.html",
                "https://curvebook.org/linux.html",
                "https://curvebook.org/faq.html",
                "https://curvebook.org/privacy.html",
                "https://curvebook.org/manual/latest/index.html",
                "https://curvebook.org/manual/latest/registry/formats.html",
            ]
        );

        let generated_root = root.join("target/docs-manual/_root");
        write_sitemap_into(root.path(), None, &generated_root).unwrap();
        write_not_found_page(root.path()).unwrap();
        write_llms_files_into(root.path(), &generated_root).unwrap();
        assert!(check_search_metadata_with_manual(root.path(), None).is_ok());

        root.write("target/docs-manual/latest/glossary.html", "new");
        assert!(check_search_metadata_with_manual(root.path(), None).unwrap_err().contains("--build-manual"));

        write_sitemap_into(root.path(), None, &generated_root).unwrap();
        root.write(NOT_FOUND_REL_PATH, "stale 404\n");
        assert!(check_search_metadata_with_manual(root.path(), None).unwrap_err().contains("docs/404.html"));
        write_not_found_page(root.path()).unwrap();
        root.write("target/docs-manual/_root/llms.txt", "stale\n");
        assert!(check_search_metadata_with_manual(root.path(), None).unwrap_err().contains("llms.txt"));
    }

    #[test]
    fn landing_pages_validate_canonical_and_open_graph_metadata() {
        let root = TestDir::new("docs-seo-canonical");
        seed_landing_pages(&root);
        let generated_root = root.join("target/docs-manual/_root");
        write_sitemap_into(root.path(), None, &generated_root).unwrap();
        write_not_found_page(root.path()).unwrap();
        write_llms_files_into(root.path(), &generated_root).unwrap();
        assert!(check_search_metadata_with_manual(root.path(), None).is_ok());

        let base_privacy = valid_landing_html("privacy.html");
        let bad_og_url = base_privacy.replace("privacy.html\"", "index.html\"").replacen("index.html\"", "privacy.html\"", 1);
        for (html, needle) in [
            (String::from("<link rel=\"canonical\" href=\"https://curvebook.org/\">"), "docs/privacy.html"),
            (format!("{0}{0}", canonical_link_tag("privacy.html")), "exactly one"),
            (bad_og_url, "og:url"),
            (base_privacy.replace("workspace-split-fit.png", "does-not-exist.png"), "og:image"),
            (base_privacy.replace("twitter:card", "other:card"), "twitter:card"),
        ] {
            root.write("docs/privacy.html", &html);
            let err = check_search_metadata_with_manual(root.path(), None).unwrap_err();
            assert!(err.contains(needle), "expected `{needle}` in `{err}`");
        }

        std::fs::remove_file(root.join("docs/privacy.html")).unwrap();
        assert!(build_sitemap_with_manual(root.path(), None).unwrap_err().contains("LANDING_PAGES"));
    }

    #[test]
    fn aud_12_41_and_44_rejects_commented_canonical_duplicate_og_slash_meta_symlink_latest_and_bad_base_url() {
        // AUD-12-44: reject HTML metacharacters / whitespace in SITE_BASE_URL when CUSTOM_DOMAIN is None.
        for bad in ["https://example.com/\" onclick=\"x/", "https://example.com/<bad>/", "https://example.com/a b/", "https://example.com/a&b/"] {
            assert!(validate_site_config(bad, None).is_err(), "expected rejection for `{bad}`");
        }

        let root = TestDir::new("docs-seo-aud-12-41");
        seed_landing_pages(&root);
        let generated_root = root.join("target/docs-manual/_root");
        write_sitemap_into(root.path(), None, &generated_root).unwrap();
        write_not_found_page(root.path()).unwrap();
        write_llms_files_into(root.path(), &generated_root).unwrap();

        let base_privacy = valid_landing_html("privacy.html");
        let canon_tag = canonical_link_tag("privacy.html");

        // 1. Commented-out canonical link must NOT satisfy canonical check.
        let commented_canon = base_privacy.replace(&canon_tag, &format!("<!-- {canon_tag} -->"));
        root.write("docs/privacy.html", &commented_canon);
        assert!(check_search_metadata_with_manual(root.path(), None).unwrap_err().contains("canonical"));

        // 2. Duplicate og:url (first bad, second valid) must fail closed.
        let dup_og = base_privacy
            .replace("<meta property=\"og:url\"", "<meta property=\"og:url\" content=\"https://evil.example/\"><meta property=\"og:url\"");
        root.write("docs/privacy.html", &dup_og);
        assert!(check_search_metadata_with_manual(root.path(), None).unwrap_err().contains("duplicate `<meta>`"));

        // 3. Slash tag-name delimiter `<meta/property="og:url"...>` is parsed and catches duplicates.
        let slash_dup_og = base_privacy
            .replace("<meta property=\"og:url\"", "<meta/property=\"og:url\"/content=\"https://evil.example/\"><meta property=\"og:url\"");
        root.write("docs/privacy.html", &slash_dup_og);
        assert!(check_search_metadata_with_manual(root.path(), None).unwrap_err().contains("duplicate `<meta>`"));

        // 3b. Quote-desync and abrupt/bang HTML5 comments cannot hide duplicate canonical or og:* meta tags.
        for poisoned_prefix in [
            "<p title=x\"><link rel=\"canonical\" href=\"https://evil.example/\"><b z=1\">",
            "<!--><link rel=\"canonical\" href=\"https://evil.example/\"><!-- -->",
            "<!---><meta property=\"og:url\" content=\"https://evil.example/\"><!-- -->",
            "<!-- c --!><meta property=\"og:url\" content=\"https://evil.example/\"><!-- -->",
            "< a=\"><link rel=\"canonical\" href=\"https://evil.example/\">",
            "<<meta property=\"og:url\" content=\"https://evil.example/\">",
            "<meta =\" property=\"og:url\" content=\"https://evil.example/\" x=\">",
            "<title><!--</title><link rel=\"canonical\" href=\"https://evil.example/\"><!-- -->",
            "<textarea><!--</textarea><meta property=\"og:url\" content=\"https://evil.example/\"><!-- -->",
        ] {
            let poisoned = base_privacy.replace("<head>", &format!("<head>{poisoned_prefix}"));
            root.write("docs/privacy.html", &poisoned);
            assert!(check_search_metadata_with_manual(root.path(), None).is_err(), "expected rejection for `{poisoned_prefix}`");
        }

        // 4. Symlinked latest/ directory must fail closed in build_sitemap_with_manual.
        #[cfg(unix)]
        {
            root.write("docs/privacy.html", &base_privacy);
            let outside = root.join("outside_snap");
            std::fs::create_dir_all(&outside).unwrap();
            std::fs::write(outside.join("manifest.json"), "{}").unwrap();
            std::fs::write(outside.join("index.html"), "<html></html>").unwrap();
            std::fs::create_dir_all(root.join("target/docs-manual")).unwrap();
            std::os::unix::fs::symlink(&outside, root.join("target/docs-manual/latest")).unwrap();
            assert!(build_sitemap_with_manual(root.path(), None).unwrap_err().contains("symlinked `latest/`"));
        }
    }
}
