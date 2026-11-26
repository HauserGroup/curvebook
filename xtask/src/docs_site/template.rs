//! Classic coral static HTML page template shared by every generated manual page.
//!
//! Enforces strict CSP (`default-src 'none'`), `no-referrer`, zero scripts, zero forms,
//! zero inline `style=` attributes, skip link, breadcrumbs, optional table of contents,
//! version banner (plus optional live-`main` contributor notice), and prev/next navigation.

use super::markdown::{TocHeading, push_html_escaped};

/// Strict Content-Security-Policy matching `docs/index.html` and `docs/privacy.html`.
pub const STRICT_CSP: &str = "default-src 'none'; style-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; connect-src 'none'";

/// Release / build context rendered into every manual page banner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteMeta {
    /// Human-readable documented version label (e.g., `"v0.1.0-alpha.2 (local preview)"` or `"v0.1.0"`).
    pub version_label: String,
    /// Optional git commit SHA for release snapshots.
    pub commit_sha: Option<String>,
    /// Optional release date (`YYYY-MM-DD`).
    pub release_date: Option<String>,
    /// True when emitting a committed release snapshot under `docs/manual/v<major>.<minor>/`.
    pub is_release_snapshot: bool,
}

/// Previous/Next sequential navigation target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavLink {
    pub label: String,
    pub rel_path: String,
}

/// Complete page specification passed to [`render_page`].
pub struct PageSpec<'a> {
    /// Page `<title>` prefix (`" — CurveBook Documentation"` is appended).
    pub title: &'a str,
    /// `<meta name="description">` summary.
    pub description: &'a str,
    /// Output path relative to generated site root (e.g. `"automation/mcp.html"`).
    pub rel_path: &'a str,
    /// Section label shown in window status & breadcrumbs.
    pub section_title: &'a str,
    /// Section overview path relative to site root (`"section-automation.html"`).
    pub section_rel_path: Option<&'a str>,
    /// Primary `<h1>` heading (skipped if `body_has_h1` is true).
    pub h1: &'a str,
    /// Optional headings extracted from Markdown or registry sections for the per-page TOC.
    pub toc_headings: &'a [TocHeading],
    /// Safe HTML fragment inside `<article>`.
    pub body_html: &'a str,
    /// When true, `body_html` already opens with `<h1 ...>` (e.g. rendered from Markdown `# Title`).
    pub body_has_h1: bool,
    /// Previous page in deterministic reading order.
    pub prev: Option<&'a NavLink>,
    /// Next page in deterministic reading order.
    pub next: Option<&'a NavLink>,
    /// Site release metadata.
    pub meta: &'a SiteMeta,
}

/// Compute relative path prefix (`""`, `"../"`, ...) from `from_rel` to site root or sibling `to_rel`.
#[must_use]
pub fn rel_href(from_rel: &str, to_rel: &str) -> String {
    format!("{}{to_rel}", "../".repeat(from_rel.matches('/').count()))
}

/// Compute relative prefix from a generated page back to shared `site.css` / `images/`.
#[must_use]
pub fn asset_prefix(from_rel: &str, is_release_snapshot: bool) -> String {
    "../".repeat(from_rel.matches('/').count() + usize::from(is_release_snapshot) * 2)
}

struct DocHead<'a> {
    description: &'a str,
    title: &'a str,
    title_suffix: &'a str,
    assets: &'a str,
    robots: Option<&'a str>,
    refresh: Option<&'a str>,
    canonical_url: Option<&'a str>,
}

fn write_doc_open(out: &mut String, head: &DocHead<'_>) {
    out.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n  <meta charset=\"utf-8\">\n  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n  <meta name=\"description\" content=\"");
    push_html_escaped(head.description, out);
    out.push_str("\">\n");
    if let Some(robots) = head.robots {
        out.push_str("  <meta name=\"robots\" content=\"");
        out.push_str(robots);
        out.push_str("\">\n");
    }
    out.push_str("  <meta name=\"referrer\" content=\"no-referrer\">\n  <meta http-equiv=\"Content-Security-Policy\" content=\"");
    out.push_str(STRICT_CSP);
    out.push_str("\">\n");
    if let Some(target) = head.refresh {
        out.push_str("  <meta http-equiv=\"refresh\" content=\"0; url=");
        push_html_escaped(target, out);
        out.push_str("\">\n");
    }
    out.push_str("  <meta name=\"color-scheme\" content=\"light\">\n  <title>");
    push_html_escaped(head.title, out);
    out.push_str(head.title_suffix);
    out.push_str("</title>\n");
    if let Some(url) = head.canonical_url {
        out.push_str("  <link rel=\"canonical\" href=\"");
        push_html_escaped(url, out);
        out.push_str("\">\n  <meta property=\"og:type\" content=\"article\">\n  <meta property=\"og:site_name\" content=\"CurveBook\">\n  <meta property=\"og:title\" content=\"");
        push_html_escaped(head.title, out);
        out.push_str(head.title_suffix);
        out.push_str("\">\n  <meta property=\"og:description\" content=\"");
        push_html_escaped(head.description, out);
        out.push_str("\">\n  <meta property=\"og:url\" content=\"");
        push_html_escaped(url, out);
        out.push_str("\">\n  <meta property=\"og:image\" content=\"");
        push_html_escaped(super::seo::SITE_BASE_URL, out);
        out.push_str("images/workspace-layout-dark.png\">\n  <meta name=\"twitter:card\" content=\"summary_large_image\">\n");
    }
    out.push_str("  <link rel=\"icon\" type=\"image/png\" href=\"");
    push_html_escaped(head.assets, out);
    out.push_str("images/curvebook.png\">\n  <link rel=\"stylesheet\" href=\"");
    push_html_escaped(head.assets, out);
    out.push_str(
        "site.css\">\n</head>\n<body>\n  <a class=\"skip-link\" href=\"#main\">Skip to content</a>\n  <div class=\"desktop-window wrap\">\n",
    );
}

fn write_main_site_header(out: &mut String, assets: &str, home_href: &str, brand: &str, status: &str, docs_href: &str, docs_current: bool) {
    out.push_str("    <header class=\"site-header\">\n      <a class=\"title-bar\" href=\"");
    push_html_escaped(home_href, out);
    out.push_str("\">\n        <span class=\"wordmark\"><img src=\"");
    push_html_escaped(assets, out);
    out.push_str("images/curvebook.png\" width=\"64\" height=\"64\" alt=\"\"><span>");
    out.push_str(brand);
    out.push_str("</span></span>\n        <span class=\"window-status\">");
    push_html_escaped(status, out);
    out.push_str("</span>\n      </a>\n      <nav aria-label=\"Main navigation\">\n        <a href=\"");
    push_html_escaped(home_href, out);
    out.push_str("\">Home</a>\n        <a href=\"");
    push_html_escaped(docs_href, out);
    out.push_str(if docs_current { "\" aria-current=\"page\">Documentation</a>\n" } else { "\">Documentation</a>\n" });
    for (file, label) in [("open-pzfx-files.html", ".pzfx &amp; .prism"), ("linux.html", "Linux"), ("faq.html", "FAQ"), ("privacy.html", "Privacy")] {
        out.push_str("        <a href=\"");
        push_html_escaped(assets, out);
        out.push_str(file);
        out.push_str("\">");
        out.push_str(label);
        out.push_str("</a>\n");
    }
    out.push_str(
        "        <a href=\"https://github.com/HauserGroup/curvebook\">GitHub <span aria-hidden=\"true\">↗</span></a>\n      </nav>\n    </header>\n",
    );
}

fn write_doc_close(out: &mut String, privacy_lead: &str, privacy_href: &str, privacy_label: &str) {
    out.push_str("    <footer class=\"site-footer\">\n      <p><strong>License:</strong> Dual-licensed under <strong>MIT OR Apache-2.0</strong>.</p>\n      <p><strong>Clean-room development:</strong> Independently implemented from published scientific references. No commercial statistical or graphing software was used to develop CurveBook, and no proprietary code or assets were copied.</p>\n      <p class=\"privacy\">CurveBook runs locally and collects no telemetry. ");
    out.push_str(privacy_lead);
    out.push_str("<a href=\"");
    push_html_escaped(privacy_href, out);
    out.push_str("\">");
    out.push_str(privacy_label);
    out.push_str("</a> · <a href=\"https://github.com/HauserGroup/curvebook\">Source code &amp; credits <span aria-hidden=\"true\">↗</span></a></p>\n    </footer>\n    <div class=\"window-statusbar\" aria-label=\"Application summary\">\n      <span class=\"status-cell is-grow\">Ready · Offline Desktop · CLI · Local MCP</span>\n      <span class=\"status-cell\">Pure Rust</span>\n      <span class=\"status-cell\">Zero Telemetry</span>\n      <span class=\"status-cell\">MIT / Apache-2.0</span>\n    </div>\n  </div>\n</body>\n</html>\n");
}

/// Render one complete Classic coral HTML5 document.
#[must_use]
pub fn render_page(spec: &PageSpec<'_>) -> String {
    let root = rel_href(spec.rel_path, "");
    let assets = asset_prefix(spec.rel_path, spec.meta.is_release_snapshot);
    let canonical_url = super::seo::manual_canonical_url(spec.rel_path);
    let mut out = String::with_capacity(spec.body_html.len() + 3584);

    write_doc_open(
        &mut out,
        &DocHead {
            description: spec.description,
            title: spec.title,
            title_suffix: " — CurveBook Documentation",
            assets: &assets,
            robots: None,
            refresh: None,
            canonical_url: Some(&canonical_url),
        },
    );
    render_header(spec, &root, &assets, &mut out);
    render_version_banner(spec, &mut out);

    out.push_str("    <main id=\"main\" tabindex=\"-1\">\n      <article class=\"overview doc-body\">\n");
    render_breadcrumbs(spec, &root, &mut out);
    if !spec.body_has_h1 {
        out.push_str("        <h1 id=\"page-title\" tabindex=\"-1\">");
        push_html_escaped(spec.h1, &mut out);
        out.push_str("</h1>\n");
    }
    render_toc(spec.toc_headings, &mut out);
    out.push_str(spec.body_html);
    if !spec.body_html.ends_with('\n') {
        out.push('\n');
    }
    render_pager(spec, &mut out);

    out.push_str("      </article>\n    </main>\n");
    let privacy_href = if spec.meta.is_release_snapshot { format!("{assets}privacy.html") } else { format!("{root}privacy-security.html") };
    write_doc_close(&mut out, "Read ", &privacy_href, "Privacy, Licences &amp; Security");
    out
}

fn active_nav_target(spec: &PageSpec<'_>) -> &'static str {
    match spec.rel_path {
        "index.html" => "index.html",
        "az-index.html" => "az-index.html",
        "registry/commands.html" => "registry/commands.html",
        _ => match spec.section_rel_path {
            Some("section-get-started.html") => "section-get-started.html",
            Some("section-automation.html") => "section-automation.html",
            Some("section-contribute.html") => "section-contribute.html",
            _ => "section-analyses.html",
        },
    }
}

fn render_header(spec: &PageSpec<'_>, root: &str, assets: &str, out: &mut String) {
    out.push_str("    <header class=\"site-header\">\n      <a class=\"title-bar\" href=\"");
    push_html_escaped(assets, out);
    out.push_str("index.html\">\n        <span class=\"wordmark\">\n          <img src=\"");
    push_html_escaped(assets, out);
    out.push_str("images/curvebook.png\" width=\"64\" height=\"64\" alt=\"\">\n          <span>CurveBook · Reference Manual</span>\n        </span>\n        <span class=\"window-status\">");
    push_html_escaped(spec.section_title, out);
    out.push_str("</span>\n      </a>\n      <nav aria-label=\"Manual navigation\">\n");

    let active_target = active_nav_target(spec);
    for (label, target) in [
        ("Manual Home", "index.html"),
        ("Get Started", "section-get-started.html"),
        ("Reference", "section-analyses.html"),
        ("Automation", "section-automation.html"),
        ("Commands &amp; Tools", "registry/commands.html"),
        ("A–Z Index", "az-index.html"),
        ("Contribute", "section-contribute.html"),
    ] {
        out.push_str("        <a href=\"");
        push_html_escaped(root, out);
        out.push_str(target);
        out.push_str(if target == active_target { "\" aria-current=\"page\">" } else { "\">" });
        out.push_str(label);
        out.push_str("</a>\n");
    }
    if spec.meta.is_release_snapshot {
        for (file, label) in [("index.html", "Product Home"), ("privacy.html", "Privacy Policy")] {
            out.push_str("        <a href=\"");
            push_html_escaped(assets, out);
            out.push_str(file);
            out.push_str("\">");
            out.push_str(label);
            out.push_str("</a>\n");
        }
    }
    out.push_str("      </nav>\n    </header>\n");
}

fn render_version_banner(spec: &PageSpec<'_>, out: &mut String) {
    out.push_str(
        "    <div class=\"version-banner\" role=\"note\" aria-label=\"Documentation version\">\n      <span><strong>Documented release:</strong> ",
    );
    push_html_escaped(&spec.meta.version_label, out);
    if let Some(date) = &spec.meta.release_date {
        out.push_str(" · ");
        push_html_escaped(date, out);
    }
    if let Some(sha) = &spec.meta.commit_sha {
        let short = sha.char_indices().nth(10).and_then(|(idx, _)| sha.get(..idx)).unwrap_or(sha);
        out.push_str(" (commit <code>");
        push_html_escaped(short, out);
        out.push_str("</code>)");
    }
    out.push_str("</span>\n      <span>No scripts · No telemetry · <a href=\"");
    push_html_escaped(&rel_href(spec.rel_path, "az-index.html"), out);
    out.push_str("\">A–Z Index</a></span>\n    </div>\n");
}

fn render_breadcrumbs(spec: &PageSpec<'_>, root: &str, out: &mut String) {
    if spec.rel_path == "index.html" {
        return;
    }
    let mut pos = 2usize;
    out.push_str("        <nav class=\"breadcrumbs\" aria-label=\"Breadcrumb\" itemscope itemtype=\"https://schema.org/BreadcrumbList\">\n          <span itemprop=\"itemListElement\" itemscope itemtype=\"https://schema.org/ListItem\"><a itemprop=\"item\" href=\"");
    push_html_escaped(root, out);
    out.push_str("index.html\"><span itemprop=\"name\">Manual</span></a><meta itemprop=\"position\" content=\"1\"></span>\n");
    if let Some(sec_rel) = spec.section_rel_path
        && sec_rel != spec.rel_path
    {
        out.push_str("          <span aria-hidden=\"true\">/</span>\n          <span itemprop=\"itemListElement\" itemscope itemtype=\"https://schema.org/ListItem\"><a itemprop=\"item\" href=\"");
        push_html_escaped(root, out);
        push_html_escaped(sec_rel, out);
        out.push_str("\"><span itemprop=\"name\">");
        push_html_escaped(spec.section_title, out);
        out.push_str("</span></a><meta itemprop=\"position\" content=\"2\"></span>\n");
        pos += 1;
    }
    out.push_str("          <span aria-hidden=\"true\">/</span>\n          <span itemprop=\"itemListElement\" itemscope itemtype=\"https://schema.org/ListItem\"><span itemprop=\"name\" aria-current=\"page\">");
    push_html_escaped(spec.title, out);
    out.push_str("</span><meta itemprop=\"position\" content=\"");
    out.push_str(&pos.to_string());
    out.push_str("\"></span>\n        </nav>\n");
}

fn render_toc(headings: &[TocHeading], out: &mut String) {
    let entries: Vec<&TocHeading> = headings.iter().filter(|h| matches!(h.level, 2 | 3)).collect();
    if entries.len() < 2 {
        return;
    }
    let (open, close) = if entries.len() > 8 {
        (
            "        <details open class=\"toc\">\n          <summary class=\"toc-title\">On this page</summary>\n          <ul>\n",
            "          </ul>\n        </details>\n",
        )
    } else {
        (
            "        <nav class=\"toc\" aria-label=\"On this page\">\n          <p class=\"toc-title\">On this page</p>\n          <ul>\n",
            "          </ul>\n        </nav>\n",
        )
    };
    out.push_str(open);
    for item in entries {
        out.push_str("            <li><a href=\"#");
        push_html_escaped(&item.id, out);
        out.push_str("\">");
        if item.level == 3 {
            out.push_str("— ");
        }
        push_html_escaped(&item.text, out);
        out.push_str("</a></li>\n");
    }
    out.push_str(close);
}

fn render_pager(spec: &PageSpec<'_>, out: &mut String) {
    if spec.prev.is_none() && spec.next.is_none() {
        return;
    }
    out.push_str("        <nav class=\"pager\" aria-label=\"Previous and next pages\">\n");
    for (link, prefix, suffix) in [(spec.prev, "← ", ""), (spec.next, "", " →")] {
        if let Some(target) = link {
            out.push_str("          <a href=\"");
            push_html_escaped(&rel_href(spec.rel_path, &target.rel_path), out);
            out.push_str("\">");
            out.push_str(prefix);
            push_html_escaped(&target.label, out);
            out.push_str(suffix);
            out.push_str("</a>\n");
        } else {
            out.push_str("          <span></span>\n");
        }
    }
    out.push_str("        </nav>\n");
}

/// Deterministic helper to render a static script-free `docs/manual/index.html` meta-refresh page
/// when `--release <version>` snapshots a release.
#[must_use]
pub fn render_release_redirect_index(version_dir: &str, version_label: &str, available_versions: &[String]) -> String {
    let target = format!("{version_dir}/index.html");
    let mut out = String::with_capacity(2048);
    write_doc_open(
        &mut out,
        &DocHead {
            description: "CurveBook reference manual entry redirecting to the latest frozen release snapshot.",
            title: "Documentation",
            title_suffix: " — CurveBook",
            assets: "../",
            robots: None,
            refresh: Some(&target),
            canonical_url: None,
        },
    );
    write_main_site_header(
        &mut out,
        "../",
        "../index.html",
        "CurveBook · Documentation",
        &format!("Latest: {version_label}"),
        "latest/index.html",
        true,
    );
    out.push_str("    <main id=\"main\" tabindex=\"-1\">\n      <div class=\"overview doc-body\">\n        <h1>CurveBook Reference Manual</h1>\n        <p>Redirecting to the latest frozen release snapshot: <a href=\"");
    push_html_escaped(&target, &mut out);
    out.push_str("\">");
    push_html_escaped(version_label, &mut out);
    out.push_str("</a>.</p>\n");
    if !available_versions.is_empty() {
        out.push_str("        <h2>Published snapshots</h2>\n        <ul>\n");
        for ver in available_versions {
            out.push_str("          <li><a href=\"");
            push_html_escaped(ver, &mut out);
            out.push_str("/index.html\">");
            push_html_escaped(ver, &mut out);
            out.push_str("</a></li>\n");
        }
        out.push_str("        </ul>\n");
    }
    out.push_str("      </div>\n    </main>\n");
    write_doc_close(&mut out, "Read the ", "../privacy.html", "Privacy Policy");
    out
}

/// Render a strict-CSP, script-free Classic coral root redirect shim (e.g., `docs/get-started.html`
/// or `docs/troubleshooting.html`) forwarding legacy root URLs into the versioned manual snapshot.
#[must_use]
pub fn render_root_redirect_shim(title: &str, description: &str, target_href: &str, version_label: &str) -> String {
    let mut out = String::with_capacity(2048);
    write_doc_open(
        &mut out,
        &DocHead { description, title, title_suffix: " — CurveBook", assets: "", robots: None, refresh: Some(target_href), canonical_url: None },
    );
    write_main_site_header(
        &mut out,
        "",
        "index.html",
        "CurveBook · Documentation",
        &format!("Redirect · {version_label}"),
        "manual/latest/index.html",
        false,
    );
    out.push_str(
        "    <main id=\"main\" tabindex=\"-1\">\n      <article class=\"overview doc-body\">\n        <h1 id=\"page-title\" tabindex=\"-1\">",
    );
    push_html_escaped(title, &mut out);
    out.push_str("</h1>\n        <p>This page now lives in the versioned reference manual. Redirecting automatically to <a href=\"");
    push_html_escaped(target_href, &mut out);
    out.push_str("\"><code>");
    push_html_escaped(target_href, &mut out);
    out.push_str("</code></a>.</p>\n        <p class=\"primary-action\"><a class=\"button primary\" href=\"");
    push_html_escaped(target_href, &mut out);
    out.push_str("\">Open manual page <span aria-hidden=\"true\">→</span></a></p>\n      </article>\n    </main>\n");
    write_doc_close(&mut out, "Read the ", "privacy.html", "Privacy Policy");
    out
}

/// Render a strict-CSP, script-free Classic coral `docs/404.html` page using `base_path`
/// (e.g. `"/curvebook/"` or `"/"`) so assets and links work at any nested missing URL depth.
#[must_use]
pub fn render_not_found_page(base_path: &str) -> String {
    let mut out = String::with_capacity(2560);
    write_doc_open(
        &mut out,
        &DocHead {
            description: "The requested CurveBook page could not be found. Return to the home page, online manual, or FAQ.",
            title: "Page not found (404)",
            title_suffix: " — CurveBook",
            assets: base_path,
            robots: Some("noindex"),
            refresh: None,
            canonical_url: None,
        },
    );
    write_main_site_header(&mut out, base_path, base_path, "CurveBook", "404 · Not found", &format!("{base_path}manual/latest/index.html"), false);
    out.push_str("    <main id=\"main\" tabindex=\"-1\">\n      <article class=\"overview doc-body\">\n        <p class=\"eyebrow\">HTTP 404</p>\n        <h1 id=\"page-title\" tabindex=\"-1\">Page not found.</h1>\n        <p class=\"lead\">The page you requested does not exist on the CurveBook site, or it moved when documentation URLs were updated.</p>\n        <div class=\"action-row\">\n          <a class=\"button primary\" href=\"");
    push_html_escaped(base_path, &mut out);
    out.push_str("\">CurveBook Home</a>\n          <a class=\"button secondary\" href=\"");
    push_html_escaped(base_path, &mut out);
    out.push_str("manual/latest/index.html\">Read the Manual <span aria-hidden=\"true\">→</span></a>\n        </div>\n        <h2>Helpful links</h2>\n        <ul>\n");
    for (rel, link_html, desc) in [
        ("manual/latest/get-started.html", "Get Started &amp; Installation", "installers, checksums, and first run."),
        ("open-pzfx-files.html", "Open <code>.pzfx</code> and <code>.prism</code> files", "inspect tables and export to CSV."),
        ("faq.html", "FAQ &amp; Quick Facts", "licence, platforms, formats, and offline guarantees."),
    ] {
        out.push_str("          <li><a href=\"");
        push_html_escaped(base_path, &mut out);
        out.push_str(rel);
        out.push_str("\">");
        out.push_str(link_html);
        out.push_str("</a> — ");
        out.push_str(desc);
        out.push_str("</li>\n");
    }
    out.push_str("          <li><a href=\"https://github.com/HauserGroup/curvebook\">GitHub repository <span aria-hidden=\"true\">↗</span></a> — source code, issue tracker, and contributor documentation.</li>\n        </ul>\n      </article>\n    </main>\n");
    write_doc_close(&mut out, "Read the ", &format!("{base_path}privacy.html"), "Privacy Policy");
    out
}

#[cfg(test)]
mod tests {
    use super::{PageSpec, SiteMeta, render_not_found_page, render_page};

    #[test]
    fn aud_12_44_escapes_base_path_and_section_rel_in_html_attributes() {
        let html_404 = render_not_found_page("/curvebook/\" onclick=\"alert(1)/");
        assert!(!html_404.contains("\" onclick=\"alert(1)"), "attribute breakout in 404: {html_404}");
        assert!(html_404.contains("&quot; onclick=&quot;alert(1)"));

        let meta = SiteMeta { version_label: String::from("v0.1.0"), commit_sha: None, release_date: None, is_release_snapshot: false };
        let page = render_page(&PageSpec {
            title: "T",
            description: "D",
            rel_path: "sub.html",
            section_title: "Sec",
            section_rel_path: Some("sec\" onclick=\"x.html"),
            h1: "T",
            toc_headings: &[],
            body_html: "<p>Hi</p>",
            body_has_h1: false,
            prev: None,
            next: None,
            meta: &meta,
        });
        assert!(!page.contains("sec\" onclick=\"x.html"), "unescaped sec_rel in breadcrumbs: {page}");
        assert!(page.contains("sec&quot; onclick=&quot;x.html"));
    }
}
