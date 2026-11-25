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

use std::collections::BTreeMap;
use std::path::Path;

use curvebook_platform::write_atomic;

use super::llms::{build_llms_full_txt, build_llms_txt};
use super::verify::{find_tag_close, parse_minor_dir_version, parse_tag_attributes};
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
        let snapshot_sub = if is_regular_file(&manual_root.join("latest/manifest.json")) {
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
        let Some(tag_end) = find_tag_close(after_lt) else {
            break;
        };
        let (tag_body, tail) = after_lt.split_at(tag_end);
        rest = tail.strip_prefix('>').unwrap_or(tail);

        let trimmed = tag_body.trim();
        let tag_name = trimmed.split_whitespace().next().unwrap_or("");
        if !tag_name.eq_ignore_ascii_case("meta") {
            continue;
        }
        let mut key: Option<String> = None;
        let mut content: Option<String> = None;
        for (k, v) in parse_tag_attributes(trimmed) {
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

/// Newest `docs/manual/v<major>.<minor>/` directory that carries a release `manifest.json`.
fn newest_snapshot_dir(manual_dir: &Path) -> Result<Option<String>, String> {
    if !is_real_dir(manual_dir) {
        return Ok(None);
    }
    let mut best: Option<((u64, u64), String)> = None;
    for path in read_dir_paths_sorted(manual_dir)? {
        if !is_real_dir(&path) {
            continue;
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if let Some(ver) = parse_minor_dir_version(&name)
            && is_regular_file(&path.join("manifest.json"))
            && best.as_ref().is_none_or(|(b, _)| ver > *b)
        {
            best = Some((ver, name));
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
        assert!(validate_site_config(SITE_BASE_URL, Some("curvebook.org")).is_err());
        assert!(validate_site_config("https://curvebook.org/", Some("https://curvebook.org")).is_err());
        assert!(validate_site_config("http://curvebook.org/", None).is_err());
        assert_eq!(manual_canonical_url("analyses.html"), "https://hausergroup.github.io/curvebook/manual/latest/analyses.html");
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

        root.write("target/docs-manual/latest/manifest.json", "{}");
        root.write("target/docs-manual/latest/index.html", "latest");
        root.write("target/docs-manual/latest/registry/formats.html", "latest");
        assert_eq!(
            extract_locs(&build_sitemap_with_manual(root.path(), None).unwrap()),
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
            (String::from("<link rel=\"canonical\" href=\"https://hausergroup.github.io/curvebook/\">"), "docs/privacy.html"),
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
}
