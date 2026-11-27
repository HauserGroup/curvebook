//! Search-engine and social metadata for the GitHub Pages site (`R12.8`, `R12.10`):
//! `sitemap.xml`, `llms.txt`, `llms-full.txt`, canonical links, Open Graph tags, and custom-domain helpers.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use curvebook_platform::write_atomic;

use super::llms::{build_llms_full_txt, build_llms_txt};
use crate::pages_site::parse_minor_dir_version;
use crate::scan_util::{is_real_dir, is_regular_file, parse_html_tag_attributes, read_dir_paths_sorted, walk_html_open_tags, walk_rel_files};

pub use crate::pages_site::{CUSTOM_DOMAIN, LANDING_PAGES, SITE_BASE_URL};

pub const NOT_FOUND_REL_PATH: &str = "docs/404.html";

#[must_use]
pub fn published_url(docs_rel: &str) -> String {
    if docs_rel == "index.html" { SITE_BASE_URL.to_owned() } else { format!("{SITE_BASE_URL}{docs_rel}") }
}

#[must_use]
pub fn manual_canonical_url(rel_path: &str) -> String {
    format!("{SITE_BASE_URL}manual/latest/{}", rel_path.trim_start_matches('/'))
}

#[must_use]
pub fn canonical_link_tag(docs_rel: &str) -> String {
    format!("<link rel=\"canonical\" href=\"{}\">", published_url(docs_rel))
}

pub fn validate_site_config(site_base_url: &str, custom_domain: Option<&str>) -> Result<(), String> {
    let has_bad = |s: &str| s.chars().any(|c| c.is_whitespace() || c.is_ascii_control() || matches!(c, '"' | '\'' | '<' | '>' | '&'));
    if !site_base_url.starts_with("https://") || !site_base_url.ends_with('/') || has_bad(site_base_url) {
        return Err(format!("SITE_BASE_URL `{site_base_url}` must start with `https://`, end with `/`, and contain no metacharacters"));
    }
    if let Some(domain) = custom_domain {
        let trimmed = domain.trim();
        if trimmed.is_empty() || trimmed.contains('/') || trimmed.contains("://") || has_bad(trimmed) {
            return Err(format!("CUSTOM_DOMAIN `{domain}` must be a bare hostname without scheme, path, or HTML metacharacters"));
        }
        let expected_base = format!("https://{trimmed}/");
        if site_base_url != expected_base {
            return Err(format!("CUSTOM_DOMAIN `{trimmed}` requires SITE_BASE_URL `{expected_base}`, found `{site_base_url}`"));
        }
    }
    Ok(())
}

pub fn build_sitemap_for_manual_pages(root: &Path, snapshot_sub: &str, manual_rel_pages: &[String]) -> Result<String, String> {
    let docs_dir = root.join("docs");
    let mut rel_paths = Vec::new();
    for &page in LANDING_PAGES {
        if !is_regular_file(&docs_dir.join(page)) {
            return Err(format!("docs/{page} is listed in LANDING_PAGES but missing"));
        }
        rel_paths.push(page.to_owned());
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
        out.push_str(&format!("  <url><loc>{}</loc></url>\n", published_url(rel)));
    }
    out.push_str("</urlset>\n");
    Ok(out)
}

pub fn build_sitemap_with_manual(root: &Path, manual_dir: Option<&Path>) -> Result<String, String> {
    let resolved = manual_dir.map(Path::to_path_buf).or_else(|| {
        let def = crate::pages_site::default_manual_dir(root);
        if is_real_dir(&def) {
            Some(def)
        } else {
            let leg = root.join("docs/manual");
            is_real_dir(&leg).then_some(leg)
        }
    });
    if let Some(manual_root) = resolved {
        let (latest, latest_m) = (manual_root.join("latest"), manual_root.join("latest/manifest.json"));
        for (p, label) in [(&latest, "`latest/` directory"), (&latest_m, "`manifest.json`")] {
            if std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(format!("{}: symlinked {label} is forbidden in sitemap", p.display()));
            }
        }
        let snapshot_sub =
            if is_real_dir(&latest) && is_regular_file(&latest_m) { Some(String::from("latest")) } else { newest_snapshot_dir(&manual_root)? };
        if let Some(snapshot) = snapshot_sub {
            let (snap_root, mut snap_pages) = (manual_root.join(&snapshot), Vec::new());
            walk_rel_files(&snap_root, &snap_root, &|_| false, &|p| p.extension().is_some_and(|e| e == "html"), &mut snap_pages)?;
            return build_sitemap_for_manual_pages(root, &snapshot, &snap_pages);
        }
        if manual_dir.is_some() {
            return Err(format!("manual directory `{}` has no `latest/` or `v<major>.<minor>/` snapshot for sitemap", manual_root.display()));
        }
    }
    build_sitemap_for_manual_pages(root, "latest", &[])
}

pub fn write_sitemap_into(root: &Path, manual_dir: Option<&Path>, out_dir: &Path) -> Result<usize, String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;
    let xml = build_sitemap_with_manual(root, manual_dir)?;
    let path = out_dir.join("sitemap.xml");
    write_atomic(&path, xml.as_bytes()).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(xml.matches("<url>").count())
}

pub fn write_not_found_page(root: &Path) -> Result<(), String> {
    let html = super::template::render_not_found_page(crate::pages_site::site_base_path());
    write_atomic(&root.join(NOT_FOUND_REL_PATH), html.as_bytes()).map_err(|e| format!("write {NOT_FOUND_REL_PATH}: {e}"))
}

pub fn check_search_metadata_with_manual(root: &Path, manual_dir: Option<&Path>) -> Result<(), String> {
    validate_site_config(SITE_BASE_URL, CUSTOM_DOMAIN)?;
    let generated_root =
        manual_dir.map(Path::to_path_buf).unwrap_or_else(|| crate::pages_site::default_manual_dir(root)).join(crate::pages_site::MANUAL_ROOT_SUBDIR);
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
    for &page in LANDING_PAGES {
        let html = std::fs::read_to_string(root.join("docs").join(page)).map_err(|e| format!("read docs/{page}: {e}"))?;
        let expected_url = published_url(page);
        let (canonical_hrefs, metas) = extract_seo_tags(page, &html)?;
        if canonical_hrefs.len() != 1 || canonical_hrefs[0] != expected_url {
            return Err(format!("docs/{page}: expected exactly one canonical link `{}`", canonical_link_tag(page)));
        }
        check_landing_page_social_meta(root, page, &expected_url, &metas)?;
    }
    Ok(())
}

fn extract_seo_tags(page: &str, html: &str) -> Result<(Vec<String>, BTreeMap<String, String>), String> {
    let (mut canonicals, mut metas, mut seen_social) = (Vec::new(), BTreeMap::new(), BTreeSet::new());
    walk_html_open_tags(html, |tag_lower, trimmed| {
        if tag_lower == "link" {
            let (mut rel, mut href) = (None, None);
            for (k, v) in parse_html_tag_attributes(trimmed) {
                if k.eq_ignore_ascii_case("rel") {
                    if rel.replace(v).is_some() {
                        return Err(format!("docs/{page}: duplicate `rel` attribute on `<link>`"));
                    }
                } else if k.eq_ignore_ascii_case("href") && href.replace(v).is_some() {
                    return Err(format!("docs/{page}: duplicate `href` attribute on `<link>`"));
                }
            }
            if rel.as_deref().is_some_and(|r| r.split_whitespace().any(|t| t.eq_ignore_ascii_case("canonical"))) {
                canonicals.push(href.unwrap_or_default());
            }
        } else if tag_lower == "meta" {
            let (mut key, mut content) = (None, None);
            for (k, v) in parse_html_tag_attributes(trimmed) {
                if k.eq_ignore_ascii_case("property") || k.eq_ignore_ascii_case("name") {
                    if key.is_none() {
                        key = Some(v.to_ascii_lowercase());
                    }
                } else if k.eq_ignore_ascii_case("content") && content.replace(v).is_some() {
                    return Err(format!("docs/{page}: duplicate `content` attribute on `<meta>`"));
                }
            }
            if let (Some(k), Some(c)) = (key, content) {
                if (k.starts_with("og:") || k.starts_with("twitter:")) && !seen_social.insert(k.clone()) {
                    return Err(format!("docs/{page}: duplicate `<meta>` tag for `{k}`"));
                }
                metas.entry(k).or_insert(c);
            }
        }
        Ok(())
    })
    .map_err(|e| format!("docs/{page}: {e}"))?;
    Ok((canonicals, metas))
}

fn check_landing_page_social_meta(root: &Path, page: &str, expected_url: &str, metas: &BTreeMap<String, String>) -> Result<(), String> {
    for req in ["og:type", "og:title", "og:description", "og:url", "og:image", "og:image:alt", "og:image:width", "og:image:height", "twitter:card"] {
        if metas.get(req).is_none_or(|v| v.trim().is_empty()) {
            return Err(format!("docs/{page}: missing or empty `<meta>` tag for `{req}`"));
        }
    }
    for dim in ["og:image:width", "og:image:height"] {
        let raw = metas.get(dim).map(String::as_str).unwrap_or("");
        if !raw.trim().parse::<u32>().is_ok_and(|n| n > 0) {
            return Err(format!("docs/{page}: `{dim}` must be a positive integer, found `{raw}`"));
        }
    }
    let actual_url = metas.get("og:url").map(String::as_str).unwrap_or("");
    if actual_url != expected_url {
        return Err(format!("docs/{page}: `og:url` is `{actual_url}`, expected `{expected_url}`"));
    }
    let og_image = metas.get("og:image").map(String::as_str).unwrap_or("");
    let Some(rel_img) = og_image.strip_prefix(SITE_BASE_URL) else {
        return Err(format!("docs/{page}: `og:image` `{og_image}` must start with `{SITE_BASE_URL}`"));
    };
    let img_path = root.join("docs").join(rel_img);
    if !rel_img.starts_with("images/") || rel_img.split('/').any(|s| s.is_empty() || s == "..") || !is_regular_file(&img_path) {
        return Err(format!("docs/{page}: `og:image` `{og_image}` must point to an existing file under `docs/images/`"));
    }
    // The declared dimensions must describe the actual image, or link previews crop or reject it (`R0.36`).
    if rel_img.to_ascii_lowercase().ends_with(".png") {
        let bytes = std::fs::read(&img_path).map_err(|e| format!("docs/{page}: read `docs/{rel_img}`: {e}"))?;
        let (width, height) = crate::ico::png_dimensions(&bytes).map_err(|e| format!("docs/{page}: `og:image` `docs/{rel_img}`: {e}"))?;
        for (dim, actual) in [("og:image:width", width), ("og:image:height", height)] {
            let declared = metas.get(dim).map(String::as_str).unwrap_or("").trim();
            if declared != actual.to_string() {
                return Err(format!("docs/{page}: `{dim}` is `{declared}` but `docs/{rel_img}` is {width}x{height}"));
            }
        }
    } else {
        return Err(format!("docs/{page}: `og:image` `docs/{rel_img}` must be a PNG so its declared dimensions can be checked"));
    }
    Ok(())
}

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
            if std::fs::symlink_metadata(&manifest).is_ok_and(|m| m.file_type().is_symlink()) {
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

    fn valid_landing(page: &str) -> String {
        format!(
            "<html><head>{}<meta property=\"og:type\" content=\"website\"><meta property=\"og:title\" content=\"CurveBook\"><meta property=\"og:description\" content=\"CurveBook.\"><meta property=\"og:url\" content=\"{}\"><meta property=\"og:image\" content=\"{SITE_BASE_URL}images/a.png\"><meta property=\"og:image:alt\" content=\"Preview.\"><meta property=\"og:image:width\" content=\"1440\"><meta property=\"og:image:height\" content=\"900\"><meta name=\"twitter:card\" content=\"summary_large_image\"></head></html>",
            canonical_link_tag(page),
            published_url(page),
        )
    }

    /// A PNG signature plus a leading IHDR chunk is all the dimension check reads.
    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&width.to_be_bytes());
        png.extend_from_slice(&height.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        png
    }

    #[test]
    fn site_config_sitemap_snapshots_and_social_meta_guards() {
        assert!(validate_site_config(SITE_BASE_URL, CUSTOM_DOMAIN).is_ok());
        for (u, d) in [("http://curvebook.org/", None), ("https://a.org/", Some("b.org")), ("https://a.org/\"x/", None)] {
            assert!(validate_site_config(u, d).is_err());
        }
        let root = TestDir::new("docs-seo");
        std::fs::create_dir_all(root.join("docs/images")).unwrap();
        std::fs::write(root.join("docs/images/a.png"), png_header(1440, 900)).unwrap();
        for page in LANDING_PAGES {
            root.write(&format!("docs/{page}"), &valid_landing(page));
        }
        for (rel, body) in [
            ("target/docs-manual/v0.1/manifest.json", "{}"),
            ("target/docs-manual/v0.1/index.html", "old"),
            ("target/docs-manual/v0.2/manifest.json", "{}"),
            ("target/docs-manual/v0.2/index.html", "new"),
        ] {
            root.write(rel, body);
        }
        assert!(build_sitemap_with_manual(root.path(), None).unwrap().contains("/manual/v0.2/index.html"));
        root.write("target/docs-manual/latest/manifest.json", "{}");
        root.write("target/docs-manual/latest/index.html", "latest");
        assert!(build_sitemap_with_manual(root.path(), None).unwrap().contains("/manual/latest/index.html"));

        let out_dir = root.join("target/docs-manual/_root");
        write_sitemap_into(root.path(), None, &out_dir).unwrap();
        write_not_found_page(root.path()).unwrap();
        write_llms_files_into(root.path(), &out_dir).unwrap();
        assert!(check_search_metadata_with_manual(root.path(), None).is_ok());

        let base = valid_landing("privacy.html");
        for bad in [
            base.replace(&canonical_link_tag("privacy.html"), &format!("<!-- {} -->", canonical_link_tag("privacy.html"))),
            base.replace("<meta property=\"og:url\"", "<meta/property=\"og:url\"/content=\"https://evil.example/\"><meta property=\"og:url\""),
            base.replace("a.png", "missing.png"),
            base.replace("content=\"1440\"", "content=\"0\""),
            base.replace("content=\"1440\"", "content=\"1200\""),
            base.replace("content=\"900\"", "content=\"630\""),
            base.replace("<head>", "<head><!--><link rel=\"canonical\" href=\"https://evil.example/\"><!-- -->"),
            base.replace("<head>", "<head><title><!--</title><meta property=\"og:url\" content=\"https://evil.example/\"><!-- -->"),
        ] {
            root.write("docs/privacy.html", &bad);
            assert!(check_search_metadata_with_manual(root.path(), None).is_err());
        }
    }
}
