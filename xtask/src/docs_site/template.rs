//! Flat two-surface static HTML5 page template shared by every generated manual page
//! and root redirect/404 page.
//!
//! Enforces strict CSP (`default-src 'none'`), `no-referrer`, zero scripts, zero forms,
//! zero inline `style=` attributes, skip link, collapsible section sidebar, breadcrumbs,
//! optional table of contents, honest version banner, and prev/next navigation.

use super::markdown::{TocHeading, extract_leading_h1, push_html_escaped};
use super::sitemap::NavGroup;

/// Strict Content-Security-Policy matching `docs/index.html` and `docs/privacy.html`.
pub const STRICT_CSP: &str = "default-src 'none'; style-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; connect-src 'none'";

/// Shared social preview image metadata matching the landing pages (`docs/images/workspace-layout-dark.png`).
pub const SOCIAL_IMAGE_ALT: &str = "CurveBook dark-theme Page Layout Composer showing a two-panel publication figure with a 4PL dose-response curve, potency bar chart with significance brackets, embedded summary table, and millimeter rulers.";
pub const SOCIAL_IMAGE_WIDTH: &str = "1440";
pub const SOCIAL_IMAGE_HEIGHT: &str = "900";

/// Release / build context rendered into every manual page banner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteMeta {
    /// Human-readable documented version label (e.g., `"v0.1.0-alpha.2 (local preview)"` or `"v0.1.0"`).
    pub version_label: String,
    /// Optional git commit SHA for release snapshots and workspace builds.
    pub commit_sha: Option<String>,
    /// Optional release/commit date (`YYYY-MM-DD`).
    pub release_date: Option<String>,
    /// True when emitting a manual snapshot tree whose assets live two levels up (`../../`).
    pub is_release_snapshot: bool,
    /// True ONLY when building a tagged release (`--release <ver>`), false for local preview and workspace CI builds.
    pub is_tagged_release: bool,
}

/// Child page entry shown under an expanded multi-page section in the manual sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarChild {
    pub title: String,
    pub rel_path: String,
}

/// Section item rendered in the manual sidebar, linking directly for 1-page sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarSection {
    pub id: &'static str,
    pub title: String,
    pub href: String,
    pub children: Vec<SidebarChild>,
}

/// Top-level group of sections rendered in the manual sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarGroup {
    pub group: NavGroup,
    pub sections: Vec<SidebarSection>,
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
    /// Section label shown in breadcrumbs.
    pub section_title: &'a str,
    /// Section overview path relative to site root (`"section-automation.html"`).
    pub section_rel_path: Option<&'a str>,
    /// Primary top-level manual navigation group owning this page (`None` for Manual Home / A–Z).
    pub nav_group: Option<NavGroup>,
    /// Active section ID used to expand multi-page child links in the left sidebar.
    pub active_section_id: Option<&'a str>,
    /// Precomputed section groups rendered into the left sidebar.
    pub sidebar_groups: &'a [SidebarGroup],
    /// Primary `<h1>` heading (used as fallback or when `body_has_h1` is false).
    pub h1: &'a str,
    /// Optional headings extracted from Markdown or registry sections for the per-page TOC.
    pub toc_headings: &'a [TocHeading],
    /// Safe HTML fragment inside `<article>`.
    pub body_html: &'a str,
    /// When true, `body_html` already contains a top-level `<h1 ...>` (e.g. rendered from Markdown `# Title`).
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
    emit_social_meta: bool,
    shell_class: &'a str,
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
        out.push_str("\">\n");
        if head.emit_social_meta {
            out.push_str("  <meta property=\"og:type\" content=\"article\">\n  <meta property=\"og:site_name\" content=\"CurveBook\">\n  <meta property=\"og:title\" content=\"");
            push_html_escaped(head.title, out);
            out.push_str(head.title_suffix);
            out.push_str("\">\n  <meta property=\"og:description\" content=\"");
            push_html_escaped(head.description, out);
            out.push_str("\">\n  <meta property=\"og:url\" content=\"");
            push_html_escaped(url, out);
            out.push_str("\">\n  <meta property=\"og:image\" content=\"");
            push_html_escaped(super::seo::SITE_BASE_URL, out);
            out.push_str("images/workspace-layout-dark.png\">\n  <meta property=\"og:image:alt\" content=\"");
            push_html_escaped(SOCIAL_IMAGE_ALT, out);
            out.push_str("\">\n  <meta property=\"og:image:width\" content=\"");
            out.push_str(SOCIAL_IMAGE_WIDTH);
            out.push_str("\">\n  <meta property=\"og:image:height\" content=\"");
            out.push_str(SOCIAL_IMAGE_HEIGHT);
            out.push_str("\">\n  <meta name=\"twitter:card\" content=\"summary_large_image\">\n");
        }
    }
    out.push_str("  <link rel=\"icon\" type=\"image/png\" href=\"");
    push_html_escaped(head.assets, out);
    out.push_str("images/curvebook.png\">\n  <link rel=\"stylesheet\" href=\"");
    push_html_escaped(head.assets, out);
    out.push_str("site.css\">\n</head>\n<body>\n  <a class=\"skip-link\" href=\"#main\">Skip to content</a>\n  <div class=\"");
    out.push_str(head.shell_class);
    out.push_str("\">\n");
}

fn write_main_site_header(out: &mut String, assets: &str, home_href: &str, docs_href: &str, docs_current: bool) {
    out.push_str("    <header class=\"site-header\">\n      <div class=\"header-inner\">\n        <a class=\"wordmark\" href=\"");
    push_html_escaped(home_href, out);
    out.push_str("\">\n          <img src=\"");
    push_html_escaped(assets, out);
    out.push_str("images/curvebook.png\" width=\"32\" height=\"32\" alt=\"\">\n          <span>CurveBook</span>\n        </a>\n        <nav class=\"site-nav\" aria-label=\"Main navigation\">\n          <a href=\"");
    push_html_escaped(home_href, out);
    out.push_str("\">Home</a>\n          <a href=\"");
    push_html_escaped(docs_href, out);
    out.push_str(if docs_current { "\" aria-current=\"page\">Documentation</a>\n" } else { "\">Documentation</a>\n" });
    for (file, label) in [("open-pzfx-files.html", ".pzfx &amp; .prism"), ("linux.html", "Linux"), ("faq.html", "FAQ")] {
        out.push_str("          <a href=\"");
        push_html_escaped(assets, out);
        out.push_str(file);
        out.push_str("\">");
        out.push_str(label);
        out.push_str("</a>\n");
    }
    out.push_str(
        "          <a href=\"https://github.com/HauserGroup/curvebook\">GitHub <span aria-hidden=\"true\">↗</span></a>\n        </nav>\n      </div>\n    </header>\n",
    );
}

fn write_doc_close(out: &mut String, privacy_href: &str, privacy_label: &str) {
    out.push_str("    <footer class=\"site-footer\">\n      <p><strong>License:</strong> MIT OR Apache-2.0. Clean-room implementation from published scientific references without commercial software or copied proprietary assets. CurveBook runs locally with zero telemetry; GitHub Pages logs visitor IPs for security.</p>\n      <p class=\"footer-links\">\n        <a href=\"");
    push_html_escaped(privacy_href, out);
    out.push_str("\">");
    out.push_str(privacy_label);
    out.push_str(
        "</a>\n        <a href=\"https://github.com/HauserGroup/curvebook\">Source code &amp; credits <span aria-hidden=\"true\">↗</span></a>\n      </p>\n    </footer>\n  </div>\n</body>\n</html>\n",
    );
}

fn short_commit_sha(sha: &str) -> &str {
    sha.char_indices().nth(10).and_then(|(idx, _)| sha.get(..idx)).unwrap_or(sha)
}

fn render_manual_footer(spec: &PageSpec<'_>, root: &str, assets: &str, out: &mut String) {
    let (privacy_href, privacy_label) = if spec.meta.is_release_snapshot {
        (format!("{assets}privacy.html"), "Privacy Policy")
    } else {
        (format!("{root}privacy-security.html"), "Privacy, Licences &amp; Security")
    };
    out.push_str("    <footer class=\"site-footer\">\n      <p><strong>License:</strong> MIT OR Apache-2.0. Clean-room implementation from published scientific references without commercial software or copied proprietary assets. CurveBook runs locally with zero telemetry; GitHub Pages logs visitor IPs for security.</p>\n      <p class=\"footer-links\">\n        <a href=\"");
    push_html_escaped(&privacy_href, out);
    out.push_str("\">");
    out.push_str(privacy_label);
    out.push_str("</a>\n        <a href=\"");
    push_html_escaped(root, out);
    out.push_str("section-contribute.html\">Contribute</a>\n");
    if spec.meta.is_release_snapshot {
        out.push_str("        <a href=\"");
        push_html_escaped(assets, out);
        out.push_str("index.html\">Home</a>\n");
    }
    out.push_str("        <a href=\"https://github.com/HauserGroup/curvebook\">Source code &amp; credits <span aria-hidden=\"true\">↗</span></a>\n");
    if let Some(sha) = &spec.meta.commit_sha {
        out.push_str("        <span>Commit <code>");
        push_html_escaped(short_commit_sha(sha), out);
        out.push_str("</code></span>\n");
    }
    out.push_str("      </p>\n    </footer>\n  </div>\n</body>\n</html>\n");
}

/// Render one complete flat two-surface HTML5 manual document.
#[must_use]
pub fn render_page(spec: &PageSpec<'_>) -> String {
    let root = rel_href(spec.rel_path, "");
    let assets = asset_prefix(spec.rel_path, spec.meta.is_release_snapshot);
    let canonical_url = super::seo::manual_canonical_url(spec.rel_path);
    let mut out = String::with_capacity(spec.body_html.len() + 4096);

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
            emit_social_meta: true,
            shell_class: "site-shell is-manual",
        },
    );
    render_header(spec, &root, &assets, &mut out);
    render_version_banner(spec, &mut out);

    let has_sidebar = !spec.sidebar_groups.is_empty();
    if has_sidebar {
        out.push_str("    <div class=\"manual-layout\">\n");
        render_sidebar(spec, &root, &mut out);
    }
    out.push_str("    <main id=\"main\" tabindex=\"-1\">\n      <article class=\"overview doc-body\">\n");
    render_breadcrumbs(spec, &root, &mut out);
    render_article_heading_toc_body(spec, &mut out);
    render_pager(spec, &mut out);

    out.push_str("      </article>\n    </main>\n");
    if has_sidebar {
        out.push_str("    </div>\n");
    }
    render_manual_footer(spec, &root, &assets, &mut out);
    out
}

fn render_article_heading_toc_body(spec: &PageSpec<'_>, out: &mut String) {
    if spec.body_has_h1 {
        let (pre_h1, h1_opt, post_h1) = extract_leading_h1(spec.body_html);
        if !pre_h1.is_empty() {
            out.push_str(pre_h1);
            if !pre_h1.ends_with('\n') {
                out.push('\n');
            }
        }
        if let Some(h1_html) = h1_opt {
            out.push_str(h1_html);
            if !h1_html.ends_with('\n') {
                out.push('\n');
            }
        } else {
            out.push_str("        <h1 id=\"page-title\" tabindex=\"-1\">");
            push_html_escaped(spec.h1, out);
            out.push_str("</h1>\n");
        }
        render_toc(spec.toc_headings, out);
        out.push_str(post_h1);
        if !post_h1.is_empty() && !post_h1.ends_with('\n') {
            out.push('\n');
        }
    } else {
        out.push_str("        <h1 id=\"page-title\" tabindex=\"-1\">");
        push_html_escaped(spec.h1, out);
        out.push_str("</h1>\n");
        render_toc(spec.toc_headings, out);
        out.push_str(spec.body_html);
        if !spec.body_html.ends_with('\n') {
            out.push('\n');
        }
    }
}

fn active_nav_target(spec: &PageSpec<'_>) -> &'static str {
    match spec.rel_path {
        "index.html" => "index.html",
        "reference.html" => "reference.html",
        "support.html" => "support.html",
        "az-index.html" => "az-index.html",
        _ => spec.nav_group.map_or("index.html", NavGroup::nav_target),
    }
}

fn render_header(spec: &PageSpec<'_>, root: &str, assets: &str, out: &mut String) {
    out.push_str("    <header class=\"site-header\">\n      <div class=\"header-inner\">\n        <a class=\"wordmark\" href=\"");
    // Published snapshots sit two levels below the site home, so the logo always returns there;
    // a local preview build has no site home beside it and falls back to the manual home.
    push_html_escaped(if spec.meta.is_release_snapshot { assets } else { root }, out);
    out.push_str("index.html\">\n          <img src=\"");
    push_html_escaped(assets, out);
    out.push_str("images/curvebook.png\" width=\"32\" height=\"32\" alt=\"\">\n          <span>CurveBook <span class=\"wordmark-sub\">Manual</span></span>\n        </a>\n        <nav class=\"site-nav\" aria-label=\"Manual navigation\">\n");

    let active_target = active_nav_target(spec);
    for (label, target) in [
        ("Manual", "index.html"),
        ("Getting started", "section-get-started.html"),
        ("Reference", "reference.html"),
        ("Automation", "section-automation.html"),
        ("Support", "support.html"),
        ("A–Z Index", "az-index.html"),
    ] {
        out.push_str("          <a href=\"");
        push_html_escaped(root, out);
        out.push_str(target);
        out.push_str(if target == active_target { "\" aria-current=\"page\">" } else { "\">" });
        out.push_str(label);
        out.push_str("</a>\n");
    }
    out.push_str("        </nav>\n");
    if spec.meta.is_release_snapshot {
        out.push_str("        <a class=\"header-utility\" href=\"");
        push_html_escaped(assets, out);
        out.push_str("index.html\">Home</a>\n");
    }
    out.push_str("      </div>\n    </header>\n");
}

fn render_version_banner(spec: &PageSpec<'_>, out: &mut String) {
    out.push_str("    <div class=\"version-banner\" role=\"note\" aria-label=\"Documentation version\">\n      <span>");
    if spec.meta.is_tagged_release {
        out.push_str("Documents ");
        push_html_escaped(&spec.meta.version_label, out);
    } else {
        out.push_str("Development build · main");
        if let Some(sha) = &spec.meta.commit_sha {
            out.push_str(" @ <code>");
            push_html_escaped(short_commit_sha(sha), out);
            out.push_str("</code>");
        }
    }
    if let Some(date) = &spec.meta.release_date {
        out.push_str(" · ");
        push_html_escaped(date, out);
    }
    out.push_str("</span>\n    </div>\n");
}

fn render_sidebar(spec: &PageSpec<'_>, root: &str, out: &mut String) {
    out.push_str("      <aside class=\"manual-sidebar\" aria-label=\"Manual sections\">\n        <nav class=\"sidebar-nav\" aria-label=\"Section navigation\">\n");
    for group in spec.sidebar_groups {
        out.push_str("          <div class=\"sidebar-group\">\n            <p class=\"sidebar-group-title\">");
        push_html_escaped(group.group.label(), out);
        out.push_str("</p>\n            <ul class=\"sidebar-list\">\n");
        for sec in &group.sections {
            let is_active_sec = spec.active_section_id == Some(sec.id);
            let is_current_sec = spec.rel_path == sec.href;
            out.push_str("              <li><a href=\"");
            push_html_escaped(root, out);
            push_html_escaped(&sec.href, out);
            out.push('"');
            if is_current_sec {
                out.push_str(" aria-current=\"page\"");
            } else if is_active_sec {
                out.push_str(" class=\"is-active\"");
            }
            out.push('>');
            push_html_escaped(&sec.title, out);
            out.push_str("</a>");
            if is_active_sec && sec.children.len() > 1 {
                out.push_str("\n                <ul class=\"sidebar-pages\">\n");
                for child in &sec.children {
                    out.push_str("                  <li><a href=\"");
                    push_html_escaped(root, out);
                    push_html_escaped(&child.rel_path, out);
                    out.push('"');
                    if spec.rel_path == child.rel_path {
                        out.push_str(" aria-current=\"page\"");
                    }
                    out.push('>');
                    push_html_escaped(&child.title, out);
                    out.push_str("</a></li>\n");
                }
                out.push_str("                </ul>\n              ");
            }
            out.push_str("</li>\n");
        }
        out.push_str("            </ul>\n          </div>\n");
    }
    out.push_str("        </nav>\n      </aside>\n");
}

fn is_top_level_hub(rel_path: &str) -> bool {
    matches!(rel_path, "index.html" | "reference.html" | "support.html") || (rel_path.starts_with("section-") && rel_path.ends_with(".html"))
}

fn render_breadcrumbs(spec: &PageSpec<'_>, root: &str, out: &mut String) {
    if is_top_level_hub(spec.rel_path) {
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
            "        <details class=\"toc\">\n          <summary class=\"toc-title\">On this page</summary>\n          <ul>\n",
            "          </ul>\n        </details>\n",
        )
    } else {
        (
            "        <nav class=\"toc\" aria-label=\"On this page\">\n          <p class=\"toc-title\">On this page</p>\n          <ul>\n",
            "          </ul>\n        </nav>\n",
        )
    };
    out.push_str(open);
    let mut in_h2_li = false;
    let mut in_sub_ul = false;
    for item in entries {
        if item.level == 3 && in_h2_li {
            if !in_sub_ul {
                out.push_str("\n              <ul>\n");
                in_sub_ul = true;
            }
            out.push_str("                <li><a href=\"#");
            push_html_escaped(&item.id, out);
            out.push_str("\">");
            push_html_escaped(&item.text, out);
            out.push_str("</a></li>\n");
            continue;
        }
        if in_sub_ul {
            out.push_str("              </ul>\n            </li>\n");
            in_sub_ul = false;
            in_h2_li = false;
        } else if in_h2_li {
            out.push_str("</li>\n");
            in_h2_li = false;
        }
        out.push_str("            <li><a href=\"#");
        push_html_escaped(&item.id, out);
        out.push_str("\">");
        push_html_escaped(&item.text, out);
        out.push_str("</a>");
        if item.level == 2 {
            in_h2_li = true;
        } else {
            out.push_str("</li>\n");
        }
    }
    if in_sub_ul {
        out.push_str("              </ul>\n            </li>\n");
    } else if in_h2_li {
        out.push_str("</li>\n");
    }
    out.push_str(close);
}

fn render_pager(spec: &PageSpec<'_>, out: &mut String) {
    let prev = spec.prev.filter(|l| !l.label.eq_ignore_ascii_case(spec.title) && !l.label.eq_ignore_ascii_case(spec.h1));
    let next = spec.next.filter(|l| !l.label.eq_ignore_ascii_case(spec.title) && !l.label.eq_ignore_ascii_case(spec.h1));
    if prev.is_none() && next.is_none() {
        return;
    }
    out.push_str("        <nav class=\"pager\" aria-label=\"Previous and next pages\">\n");
    for (link, prefix, suffix) in [(prev, "← ", ""), (next, "", " →")] {
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
/// when `--release <version>` or `--build-manual` builds a manual tree.
#[must_use]
pub fn render_release_redirect_index(version_dir: &str, version_label: &str, available_versions: &[String]) -> String {
    let target = format!("{version_dir}/index.html");
    let canonical = super::seo::manual_canonical_url("index.html");
    let mut out = String::with_capacity(2048);
    write_doc_open(
        &mut out,
        &DocHead {
            description: "CurveBook reference manual entry redirecting to the latest manual build.",
            title: "Documentation",
            title_suffix: " — CurveBook",
            assets: "../",
            robots: Some("noindex, follow"),
            refresh: Some(&target),
            canonical_url: Some(&canonical),
            emit_social_meta: false,
            shell_class: "site-shell",
        },
    );
    write_main_site_header(&mut out, "../", "../index.html", &target, true);
    out.push_str("    <main id=\"main\" tabindex=\"-1\">\n      <article class=\"overview doc-body\">\n        <h1 id=\"page-title\" tabindex=\"-1\">CurveBook Reference Manual</h1>\n        <p>Redirecting to the latest manual snapshot: <a href=\"");
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
    out.push_str("      </article>\n    </main>\n");
    write_doc_close(&mut out, "../privacy.html", "Privacy Policy");
    out
}

/// Render a strict-CSP, script-free flat root redirect shim (e.g., `docs/get-started.html`
/// or `docs/troubleshooting.html`) forwarding legacy root URLs into the manual snapshot.
#[must_use]
pub fn render_root_redirect_shim(title: &str, description: &str, target_href: &str, _version_label: &str) -> String {
    let canonical_rel = target_href.split(['#', '?']).next().unwrap_or(target_href).trim_start_matches('/');
    let canonical = super::seo::published_url(canonical_rel);
    let mut out = String::with_capacity(2048);
    write_doc_open(
        &mut out,
        &DocHead {
            description,
            title,
            title_suffix: " — CurveBook",
            assets: "",
            robots: Some("noindex, follow"),
            refresh: Some(target_href),
            canonical_url: Some(&canonical),
            emit_social_meta: false,
            shell_class: "site-shell",
        },
    );
    write_main_site_header(&mut out, "", "index.html", "manual/latest/index.html", false);
    out.push_str(
        "    <main id=\"main\" tabindex=\"-1\">\n      <article class=\"overview doc-body\">\n        <h1 id=\"page-title\" tabindex=\"-1\">",
    );
    push_html_escaped(title, &mut out);
    out.push_str("</h1>\n        <p>This page now lives in the reference manual. Redirecting automatically to <a href=\"");
    push_html_escaped(target_href, &mut out);
    out.push_str("\"><code>");
    push_html_escaped(target_href, &mut out);
    out.push_str("</code></a>.</p>\n        <p class=\"primary-action\"><a class=\"button primary\" href=\"");
    push_html_escaped(target_href, &mut out);
    out.push_str("\">Open manual page <span aria-hidden=\"true\">→</span></a></p>\n      </article>\n    </main>\n");
    write_doc_close(&mut out, "privacy.html", "Privacy Policy");
    out
}

/// Render a strict-CSP, script-free flat `docs/404.html` page using `base_path`
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
            emit_social_meta: false,
            shell_class: "site-shell",
        },
    );
    write_main_site_header(&mut out, base_path, base_path, &format!("{base_path}manual/latest/index.html"), false);
    out.push_str("    <main id=\"main\" tabindex=\"-1\">\n      <article class=\"overview doc-body\">\n        <p class=\"eyebrow\">HTTP 404</p>\n        <h1 id=\"page-title\" tabindex=\"-1\">Page not found.</h1>\n        <p class=\"lead\">The page you requested does not exist on the CurveBook site, or it moved when documentation URLs were updated.</p>\n        <div class=\"action-row\">\n          <a class=\"button primary\" href=\"");
    push_html_escaped(base_path, &mut out);
    out.push_str("\">CurveBook Home</a>\n          <a class=\"button secondary\" href=\"");
    push_html_escaped(base_path, &mut out);
    out.push_str("manual/latest/get-started.html\">Get started <span aria-hidden=\"true\">→</span></a>\n        </div>\n        <h2>Helpful links</h2>\n        <ul>\n");
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
    write_doc_close(&mut out, &format!("{base_path}privacy.html"), "Privacy Policy");
    out
}

#[cfg(test)]
mod tests {
    use super::{NavGroup, PageSpec, SiteMeta, render_not_found_page, render_page, render_release_redirect_index, render_root_redirect_shim};
    use crate::docs_site::markdown::{IdentityResolver, TocHeading, render_markdown};

    #[test]
    fn r12_8_redirect_shims_emit_noindex_follow_and_fragment_free_canonical() {
        let manual_idx = render_release_redirect_index("latest", "v0.1.0", &[String::from("v0.1")]);
        assert!(manual_idx.contains("<meta name=\"robots\" content=\"noindex, follow\">"));
        assert!(manual_idx.contains("<link rel=\"canonical\" href=\"https://curvebook.org/manual/latest/index.html\">"));
        assert!(!manual_idx.contains("og:image"));

        let shim = render_root_redirect_shim("Troubleshooting", "Redirect desc", "manual/latest/troubleshooting.html#known-issues", "v0.1.0");
        assert!(shim.contains("<meta name=\"robots\" content=\"noindex, follow\">"));
        assert!(shim.contains("<link rel=\"canonical\" href=\"https://curvebook.org/manual/latest/troubleshooting.html\">"));
        assert!(!shim.contains("rel=\"canonical\" href=\"https://curvebook.org/manual/latest/troubleshooting.html#known-issues\""));
        assert!(shim.contains("<meta http-equiv=\"refresh\" content=\"0; url=manual/latest/troubleshooting.html#known-issues\">"));
    }

    #[test]
    fn aud_12_44_escapes_base_path_and_section_rel_in_html_attributes() {
        let html_404 = render_not_found_page("/curvebook/\" onclick=\"alert(1)/");
        assert!(!html_404.contains("\" onclick=\"alert(1)"), "attribute breakout in 404: {html_404}");
        assert!(html_404.contains("&quot; onclick=&quot;alert(1)"));

        let meta = SiteMeta {
            version_label: String::from("v0.1.0"),
            commit_sha: None,
            release_date: None,
            is_release_snapshot: false,
            is_tagged_release: false,
        };
        let page = render_page(&PageSpec {
            title: "T",
            description: "D",
            rel_path: "sub.html",
            section_title: "Sec",
            section_rel_path: Some("sec\" onclick=\"x.html"),
            nav_group: Some(NavGroup::Reference),
            active_section_id: None,
            sidebar_groups: &[],
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

    #[test]
    fn r12_14_and_r12_13_toc_renders_below_h1_for_markdown_pre_h1_and_prerendered_pages() {
        let dev_meta = SiteMeta {
            version_label: String::from("v0.1.0"),
            commit_sha: Some(String::from("0123456789abcdef0123456789abcdef01234567")),
            release_date: Some(String::from("2026-11-26")),
            is_release_snapshot: false,
            is_tagged_release: false,
        };

        // (a) Markdown H1 with inline <code> and explicit {#custom-id}
        let md_a = "# CLI Reference (`curvebook-cli`) {#custom-cli}\n\n## Options\n\n### Global\n\nText.\n\n## Commands\n\nMore.\n";
        let ren_a = render_markdown(md_a, &IdentityResolver);
        let page_a = render_page(&PageSpec {
            title: ren_a.title.as_deref().unwrap_or(""),
            description: "CLI ref",
            rel_path: "automation/cli.html",
            section_title: "Automation",
            section_rel_path: Some("section-automation.html"),
            nav_group: Some(NavGroup::Automation),
            active_section_id: Some("automation"),
            sidebar_groups: &[],
            h1: ren_a.title.as_deref().unwrap_or(""),
            toc_headings: &ren_a.headings,
            body_html: &ren_a.html,
            body_has_h1: true,
            prev: None,
            next: None,
            meta: &dev_meta,
        });
        let h1_a = "<h1 id=\"custom-cli\" tabindex=\"-1\">CLI Reference (<code>curvebook-cli</code>)</h1>";
        assert!(page_a.contains(h1_a), "{page_a}");
        assert_eq!(page_a.matches("<h1").count(), 1);
        let h1_pos = page_a.find(h1_a).unwrap();
        let toc_pos = page_a.find("<nav class=\"toc\"").unwrap();
        let body_pos = page_a.find("<h2 id=\"options\"").unwrap();
        assert!(h1_pos < toc_pos && toc_pos < body_pos, "expected H1 -> TOC -> body: {page_a}");
        assert!(
            page_a.contains("<ul>\n                <li><a href=\"#global\">Global</a></li>\n              </ul>"),
            "nested H3 TOC missing: {page_a}"
        );
        assert!(!page_a.contains("— Global"), "flat em-dash H3 prefix must be gone");
        assert!(page_a.contains("Development build · main @ <code>0123456789</code> · 2026-11-26"));
        assert!(!page_a.contains("desktop-window") && !page_a.contains("title-bar") && !page_a.contains("window-statusbar"));

        // (b) Markdown with intro paragraph before first H1
        let md_b = "Introductory paragraph.\n\n# Guide Title\n\n## Alpha\n\n## Beta\n";
        let ren_b = render_markdown(md_b, &IdentityResolver);
        let page_b = render_page(&PageSpec {
            title: "Guide Title",
            description: "Guide",
            rel_path: "concepts.html",
            section_title: "Concepts",
            section_rel_path: Some("section-concepts.html"),
            nav_group: Some(NavGroup::Reference),
            active_section_id: Some("concepts"),
            sidebar_groups: &[],
            h1: "Guide Title",
            toc_headings: &ren_b.headings,
            body_html: &ren_b.html,
            body_has_h1: true,
            prev: None,
            next: None,
            meta: &dev_meta,
        });
        let intro_pos = page_b.find("<p>Introductory paragraph.</p>").unwrap();
        let h1_b_pos = page_b.find("<h1 id=\"guide-title\"").unwrap();
        let toc_b_pos = page_b.find("<nav class=\"toc\"").unwrap();
        let h2_b_pos = page_b.find("<h2 id=\"alpha\"").unwrap();
        assert!(intro_pos < h1_b_pos && h1_b_pos < toc_b_pos && toc_b_pos < h2_b_pos, "{page_b}");
        assert_eq!(page_b.matches("<h1").count(), 1);

        // (c) Pre-rendered !body_has_h1 page with >8 headings collapses TOC without `open`
        let many: Vec<TocHeading> = (1..=9).map(|n| TocHeading { level: 2, id: format!("h-{n}"), text: format!("Heading {n}") }).collect();
        let tag_meta = SiteMeta { is_tagged_release: true, is_release_snapshot: true, ..dev_meta.clone() };
        let page_c = render_page(&PageSpec {
            title: "Commands",
            description: "Cmds",
            rel_path: "registry/commands.html",
            section_title: "Automation",
            section_rel_path: Some("section-automation.html"),
            nav_group: Some(NavGroup::Automation),
            active_section_id: Some("automation"),
            sidebar_groups: &[],
            h1: "Commands",
            toc_headings: &many,
            body_html: "<h2 id=\"h-1\">Heading 1</h2>",
            body_has_h1: false,
            prev: None,
            next: None,
            meta: &tag_meta,
        });
        let h1_c_pos = page_c.find("<h1 id=\"page-title\"").unwrap();
        let toc_c_pos = page_c.find("<details class=\"toc\">").unwrap();
        let h2_c_pos = page_c.find("<h2 id=\"h-1\"").unwrap();
        assert!(h1_c_pos < toc_c_pos && toc_c_pos < h2_c_pos);
        assert!(!page_c.contains("<details open"), "long TOC must be collapsed by default");
        assert!(page_c.contains("Documents v0.1.0 · 2026-11-26"));
        assert!(page_c.contains("class=\"header-utility\" href=\"../../../index.html\">Home</a>"));
        assert!(page_c.contains("class=\"wordmark\" href=\"../../../index.html\""), "release logo must link to the site home");
    }
}
