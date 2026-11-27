//! Flat two-surface static HTML5 page template shared by every generated manual page
//! and root redirect/404 page.

use std::fmt::Write as _;

use super::markdown::{TocHeading, escape_html as eh, extract_leading_h1, push_html_escaped};
use super::sitemap::NavGroup;

pub const STRICT_CSP: &str = "default-src 'none'; style-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; connect-src 'none'";
pub const SOCIAL_IMAGE_ALT: &str = "CurveBook dark-theme Page Layout Composer showing a two-panel publication figure with a 4PL dose-response curve, potency bar chart with significance brackets, embedded summary table, and millimeter rulers.";
pub const SOCIAL_IMAGE_WIDTH: &str = "1440";
pub const SOCIAL_IMAGE_HEIGHT: &str = "900";
const FOOTER_TEXT: &str = "<strong>License:</strong> MIT OR Apache-2.0. Clean-room implementation from published scientific references without commercial software or copied proprietary assets. CurveBook runs locally with zero telemetry; GitHub Pages logs visitor IPs for security.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteMeta {
    pub version_label: String,
    pub commit_sha: Option<String>,
    pub release_date: Option<String>,
    pub is_release_snapshot: bool,
    pub is_tagged_release: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarChild {
    pub title: String,
    pub rel_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarSection {
    pub id: &'static str,
    pub title: String,
    pub href: String,
    pub children: Vec<SidebarChild>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarGroup {
    pub group: NavGroup,
    pub sections: Vec<SidebarSection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavLink {
    pub label: String,
    pub rel_path: String,
}

pub struct PageSpec<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub rel_path: &'a str,
    pub section_title: &'a str,
    pub section_rel_path: Option<&'a str>,
    pub nav_group: Option<NavGroup>,
    pub active_section_id: Option<&'a str>,
    pub sidebar_groups: &'a [SidebarGroup],
    pub h1: &'a str,
    pub toc_headings: &'a [TocHeading],
    pub body_html: &'a str,
    pub body_has_h1: bool,
    pub prev: Option<&'a NavLink>,
    pub next: Option<&'a NavLink>,
    pub meta: &'a SiteMeta,
}

#[must_use]
pub fn rel_href(from_rel: &str, to_rel: &str) -> String {
    format!("{}{to_rel}", "../".repeat(from_rel.matches('/').count()))
}

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

fn write_doc_open(out: &mut String, h: &DocHead<'_>) {
    let (desc, title, assets) = (eh(h.description), eh(h.title), eh(h.assets));
    let _ = write!(
        out,
        "<!doctype html>\n<html lang=\"en\">\n<head>\n  <meta charset=\"utf-8\">\n  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n  <meta name=\"description\" content=\"{desc}\">\n"
    );
    if let Some(robots) = h.robots {
        let _ = writeln!(out, "  <meta name=\"robots\" content=\"{robots}\">");
    }
    let _ =
        writeln!(out, "  <meta name=\"referrer\" content=\"no-referrer\">\n  <meta http-equiv=\"Content-Security-Policy\" content=\"{STRICT_CSP}\">");
    if let Some(target) = h.refresh {
        let _ = writeln!(out, "  <meta http-equiv=\"refresh\" content=\"0; url={}\">", eh(target));
    }
    let _ = writeln!(out, "  <meta name=\"color-scheme\" content=\"light\">\n  <title>{title}{}</title>", h.title_suffix);
    if let Some(url) = h.canonical_url {
        let u = eh(url);
        let _ = writeln!(out, "  <link rel=\"canonical\" href=\"{u}\">");
        if h.emit_social_meta {
            let _ = write!(
                out,
                "  <meta property=\"og:type\" content=\"article\">\n  <meta property=\"og:site_name\" content=\"CurveBook\">\n  <meta property=\"og:title\" content=\"{title}{}\">\n  <meta property=\"og:description\" content=\"{desc}\">\n  <meta property=\"og:url\" content=\"{u}\">\n  <meta property=\"og:image\" content=\"{}images/workspace-layout-dark.png\">\n  <meta property=\"og:image:alt\" content=\"{}\">\n  <meta property=\"og:image:width\" content=\"{SOCIAL_IMAGE_WIDTH}\">\n  <meta property=\"og:image:height\" content=\"{SOCIAL_IMAGE_HEIGHT}\">\n  <meta name=\"twitter:card\" content=\"summary_large_image\">\n",
                h.title_suffix,
                eh(super::seo::SITE_BASE_URL),
                eh(SOCIAL_IMAGE_ALT),
            );
        }
    }
    let _ = write!(
        out,
        "  <link rel=\"icon\" type=\"image/png\" href=\"{assets}images/curvebook.png\">\n  <link rel=\"stylesheet\" href=\"{assets}site.css\">\n</head>\n<body>\n  <a class=\"skip-link\" href=\"#main\">Skip to content</a>\n  <div class=\"{}\">\n",
        h.shell_class
    );
}

fn write_main_site_header(out: &mut String, assets: &str, home_href: &str, docs_href: &str, docs_current: bool) {
    let (a, h, d, cur) = (eh(assets), eh(home_href), eh(docs_href), if docs_current { " aria-current=\"page\"" } else { "" });
    let _ = write!(
        out,
        "    <header class=\"site-header\">\n      <div class=\"header-inner\">\n        <a class=\"wordmark\" href=\"{h}\">\n          <img src=\"{a}images/curvebook.png\" width=\"32\" height=\"32\" alt=\"\">\n          <span>CurveBook</span>\n        </a>\n        <nav class=\"site-nav\" aria-label=\"Main navigation\">\n          <a href=\"{h}\">Home</a>\n          <a href=\"{d}\"{cur}>Documentation</a>\n"
    );
    for (file, label) in [("open-pzfx-files.html", ".pzfx &amp; .prism"), ("linux.html", "Linux"), ("faq.html", "FAQ")] {
        let _ = writeln!(out, "          <a href=\"{a}{file}\">{label}</a>");
    }
    out.push_str("          <a href=\"https://github.com/HauserGroup/curvebook\">GitHub <span aria-hidden=\"true\">↗</span></a>\n        </nav>\n      </div>\n    </header>\n");
}

fn write_doc_close(out: &mut String, privacy_href: &str, privacy_label: &str, extra_links: &str, commit_sha: Option<&str>) {
    let _ = write!(
        out,
        "    <footer class=\"site-footer\">\n      <p>{FOOTER_TEXT}</p>\n      <p class=\"footer-links\">\n        <a href=\"{}\">{privacy_label}</a>\n{extra_links}        <a href=\"https://github.com/HauserGroup/curvebook\">Source code &amp; credits <span aria-hidden=\"true\">↗</span></a>\n",
        eh(privacy_href)
    );
    if let Some(sha) = commit_sha {
        let _ = writeln!(out, "        <span>Commit <code>{}</code></span>", eh(short_commit_sha(sha)));
    }
    out.push_str("      </p>\n    </footer>\n  </div>\n</body>\n</html>\n");
}

fn short_commit_sha(sha: &str) -> &str {
    sha.char_indices().nth(10).and_then(|(idx, _)| sha.get(..idx)).unwrap_or(sha)
}

#[must_use]
pub fn render_page(spec: &PageSpec<'_>) -> String {
    let (root, assets, canonical) =
        (rel_href(spec.rel_path, ""), asset_prefix(spec.rel_path, spec.meta.is_release_snapshot), super::seo::manual_canonical_url(spec.rel_path));
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
            canonical_url: Some(&canonical),
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
    let (p_href, p_label) = if spec.meta.is_release_snapshot {
        (format!("{assets}privacy.html"), "Privacy Policy")
    } else {
        (format!("{root}privacy-security.html"), "Privacy, Licences &amp; Security")
    };
    let mut extra = format!("        <a href=\"{}section-contribute.html\">Contribute</a>\n", eh(&root));
    if spec.meta.is_release_snapshot {
        let _ = writeln!(extra, "        <a href=\"{}index.html\">Home</a>", eh(&assets));
    }
    write_doc_close(&mut out, &p_href, p_label, &extra, spec.meta.commit_sha.as_deref());
    out
}

fn render_article_heading_toc_body(spec: &PageSpec<'_>, out: &mut String) {
    let push_nl = |s: &str, out: &mut String| {
        out.push_str(s);
        if !s.is_empty() && !s.ends_with('\n') {
            out.push('\n');
        }
    };
    if spec.body_has_h1 {
        let (pre_h1, h1_opt, post_h1) = extract_leading_h1(spec.body_html);
        push_nl(pre_h1, out);
        match h1_opt {
            Some(h1_html) => push_nl(h1_html, out),
            None => {
                let _ = writeln!(out, "        <h1 id=\"page-title\" tabindex=\"-1\">{}</h1>", eh(spec.h1));
            }
        }
        render_toc(spec.toc_headings, out);
        push_nl(post_h1, out);
    } else {
        let _ = writeln!(out, "        <h1 id=\"page-title\" tabindex=\"-1\">{}</h1>", eh(spec.h1));
        render_toc(spec.toc_headings, out);
        push_nl(spec.body_html, out);
    }
}

fn render_header(spec: &PageSpec<'_>, root: &str, assets: &str, out: &mut String) {
    let (logo_base, a, r) = (eh(if spec.meta.is_release_snapshot { assets } else { root }), eh(assets), eh(root));
    let _ = write!(
        out,
        "    <header class=\"site-header\">\n      <div class=\"header-inner\">\n        <a class=\"wordmark\" href=\"{logo_base}index.html\">\n          <img src=\"{a}images/curvebook.png\" width=\"32\" height=\"32\" alt=\"\">\n          <span>CurveBook <span class=\"wordmark-sub\">Manual</span></span>\n        </a>\n        <nav class=\"site-nav\" aria-label=\"Manual navigation\">\n"
    );
    let active = match spec.rel_path {
        "index.html" | "reference.html" | "support.html" | "az-index.html" => spec.rel_path,
        _ => spec.nav_group.map_or("index.html", NavGroup::nav_target),
    };
    for (label, target) in [
        ("Manual", "index.html"),
        ("Getting started", "section-get-started.html"),
        ("Reference", "reference.html"),
        ("Automation", "section-automation.html"),
        ("Support", "support.html"),
        ("A–Z Index", "az-index.html"),
    ] {
        let cur = if target == active { " aria-current=\"page\"" } else { "" };
        let _ = writeln!(out, "          <a href=\"{r}{target}\"{cur}>{label}</a>");
    }
    out.push_str("        </nav>\n");
    if spec.meta.is_release_snapshot {
        let _ = writeln!(out, "        <a class=\"header-utility\" href=\"{a}index.html\">Home</a>");
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
            let _ = write!(out, " @ <code>{}</code>", eh(short_commit_sha(sha)));
        }
    }
    if let Some(date) = &spec.meta.release_date {
        let _ = write!(out, " · {}", eh(date));
    }
    out.push_str("</span>\n    </div>\n");
}

fn render_sidebar(spec: &PageSpec<'_>, root: &str, out: &mut String) {
    let r = eh(root);
    out.push_str("      <aside class=\"manual-sidebar\" aria-label=\"Manual sections\">\n        <nav class=\"sidebar-nav\" aria-label=\"Section navigation\">\n");
    for group in spec.sidebar_groups {
        let _ = write!(
            out,
            "          <div class=\"sidebar-group\">\n            <p class=\"sidebar-group-title\">{}</p>\n            <ul class=\"sidebar-list\">\n",
            eh(group.group.label())
        );
        for sec in &group.sections {
            let is_active = spec.active_section_id == Some(sec.id);
            let attr = if spec.rel_path == sec.href {
                " aria-current=\"page\""
            } else if is_active {
                " class=\"is-active\""
            } else {
                ""
            };
            let _ = write!(out, "              <li><a href=\"{r}{}\"{attr}>{}</a>", eh(&sec.href), eh(&sec.title));
            if is_active && sec.children.len() > 1 {
                out.push_str("\n                <ul class=\"sidebar-pages\">\n");
                for child in &sec.children {
                    let cur = if spec.rel_path == child.rel_path { " aria-current=\"page\"" } else { "" };
                    let _ = writeln!(out, "                  <li><a href=\"{r}{}\"{cur}>{}</a></li>", eh(&child.rel_path), eh(&child.title));
                }
                out.push_str("                </ul>\n              ");
            }
            out.push_str("</li>\n");
        }
        out.push_str("            </ul>\n          </div>\n");
    }
    out.push_str("        </nav>\n      </aside>\n");
}

fn render_breadcrumbs(spec: &PageSpec<'_>, root: &str, out: &mut String) {
    if matches!(spec.rel_path, "index.html" | "reference.html" | "support.html")
        || (spec.rel_path.starts_with("section-") && spec.rel_path.ends_with(".html"))
    {
        return;
    }
    let (r, mut pos) = (eh(root), 2usize);
    let _ = writeln!(
        out,
        "        <nav class=\"breadcrumbs\" aria-label=\"Breadcrumb\" itemscope itemtype=\"https://schema.org/BreadcrumbList\">\n          <span itemprop=\"itemListElement\" itemscope itemtype=\"https://schema.org/ListItem\"><a itemprop=\"item\" href=\"{r}index.html\"><span itemprop=\"name\">Manual</span></a><meta itemprop=\"position\" content=\"1\"></span>"
    );
    if let Some(sec_rel) = spec.section_rel_path
        && sec_rel != spec.rel_path
    {
        let _ = writeln!(
            out,
            "          <span aria-hidden=\"true\">/</span>\n          <span itemprop=\"itemListElement\" itemscope itemtype=\"https://schema.org/ListItem\"><a itemprop=\"item\" href=\"{r}{}\"><span itemprop=\"name\">{}</span></a><meta itemprop=\"position\" content=\"2\"></span>",
            eh(sec_rel),
            eh(spec.section_title)
        );
        pos += 1;
    }
    let _ = writeln!(
        out,
        "          <span aria-hidden=\"true\">/</span>\n          <span itemprop=\"itemListElement\" itemscope itemtype=\"https://schema.org/ListItem\"><span itemprop=\"name\" aria-current=\"page\">{}</span><meta itemprop=\"position\" content=\"{pos}\"></span>\n        </nav>",
        eh(spec.title)
    );
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
    let (mut in_h2_li, mut in_sub_ul) = (false, false);
    for item in entries {
        if item.level == 3 && in_h2_li {
            if !in_sub_ul {
                out.push_str("\n              <ul>\n");
                in_sub_ul = true;
            }
            let _ = writeln!(out, "                <li><a href=\"#{}\">{}</a></li>", eh(&item.id), eh(&item.text));
            continue;
        }
        if in_sub_ul {
            out.push_str("              </ul>\n            </li>\n");
            (in_sub_ul, in_h2_li) = (false, false);
        } else if in_h2_li {
            out.push_str("</li>\n");
            in_h2_li = false;
        }
        let _ = write!(out, "            <li><a href=\"#{}\">{}</a>", eh(&item.id), eh(&item.text));
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
        match link {
            Some(t) => {
                let _ = writeln!(out, "          <a href=\"{}\">{prefix}{}{suffix}</a>", eh(&rel_href(spec.rel_path, &t.rel_path)), eh(&t.label));
            }
            None => out.push_str("          <span></span>\n"),
        }
    }
    out.push_str("        </nav>\n");
}

#[must_use]
pub fn render_release_redirect_index(version_dir: &str, version_label: &str, available_versions: &[String]) -> String {
    let (target, canonical, mut out) =
        (format!("{version_dir}/index.html"), super::seo::manual_canonical_url("index.html"), String::with_capacity(2048));
    write_doc_open(
        &mut out,
        &DocHead {
            description: "CurveBook reference manual entry redirecting to the latest manual build.",
            title: "Documentation",
            title_suffix: " — CurveBook",
            assets: "../",
            robots: None,
            refresh: Some(&target),
            canonical_url: Some(&canonical),
            emit_social_meta: false,
            shell_class: "site-shell",
        },
    );
    write_main_site_header(&mut out, "../", "../index.html", &target, true);
    let _ = writeln!(
        out,
        "    <main id=\"main\" tabindex=\"-1\">\n      <article class=\"overview doc-body\">\n        <h1 id=\"page-title\" tabindex=\"-1\">CurveBook Reference Manual</h1>\n        <p>Redirecting to the latest manual snapshot: <a href=\"{}\">{}</a>.</p>",
        eh(&target),
        eh(version_label)
    );
    if !available_versions.is_empty() {
        out.push_str("        <h2>Published snapshots</h2>\n        <ul>\n");
        for ver in available_versions {
            let v = eh(ver);
            let _ = writeln!(out, "          <li><a href=\"{v}/index.html\">{v}</a></li>");
        }
        out.push_str("        </ul>\n");
    }
    out.push_str("      </article>\n    </main>\n");
    write_doc_close(&mut out, "../privacy.html", "Privacy Policy", "", None);
    out
}

#[must_use]
pub fn render_root_redirect_shim(title: &str, description: &str, target_href: &str, _version_label: &str) -> String {
    let canonical = super::seo::published_url(target_href.split(['#', '?']).next().unwrap_or(target_href).trim_start_matches('/'));
    let mut out = String::with_capacity(2048);
    write_doc_open(
        &mut out,
        &DocHead {
            description,
            title,
            title_suffix: " — CurveBook",
            assets: "",
            robots: None,
            refresh: Some(target_href),
            canonical_url: Some(&canonical),
            emit_social_meta: false,
            shell_class: "site-shell",
        },
    );
    write_main_site_header(&mut out, "", "index.html", "manual/latest/index.html", false);
    let (t, h) = (eh(title), eh(target_href));
    let _ = write!(
        out,
        "    <main id=\"main\" tabindex=\"-1\">\n      <article class=\"overview doc-body\">\n        <h1 id=\"page-title\" tabindex=\"-1\">{t}</h1>\n        <p>This page now lives in the reference manual. Redirecting automatically to <a href=\"{h}\"><code>{h}</code></a>.</p>\n        <p class=\"primary-action\"><a class=\"button primary\" href=\"{h}\">Open manual page <span aria-hidden=\"true\">→</span></a></p>\n      </article>\n    </main>\n"
    );
    write_doc_close(&mut out, "privacy.html", "Privacy Policy", "", None);
    out
}

#[must_use]
pub fn render_not_found_page(base_path: &str) -> String {
    let (b, mut out) = (eh(base_path), String::with_capacity(2560));
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
    let _ = write!(
        out,
        "    <main id=\"main\" tabindex=\"-1\">\n      <article class=\"overview doc-body\">\n        <p class=\"eyebrow\">HTTP 404</p>\n        <h1 id=\"page-title\" tabindex=\"-1\">Page not found.</h1>\n        <p class=\"lead\">The page you requested does not exist on the CurveBook site, or it moved when documentation URLs were updated.</p>\n        <div class=\"action-row\">\n          <a class=\"button primary\" href=\"{b}\">CurveBook Home</a>\n          <a class=\"button secondary\" href=\"{b}manual/latest/get-started.html\">Get started <span aria-hidden=\"true\">→</span></a>\n        </div>\n        <h2>Helpful links</h2>\n        <ul>\n"
    );
    for (rel, link_html, desc) in [
        ("manual/latest/get-started.html", "Get Started &amp; Installation", "installers, checksums, and first run."),
        ("open-pzfx-files.html", "Open <code>.pzfx</code> and <code>.prism</code> files", "inspect tables and export to CSV."),
        ("faq.html", "FAQ &amp; Quick Facts", "licence, platforms, formats, and offline guarantees."),
    ] {
        let _ = writeln!(out, "          <li><a href=\"{b}{rel}\">{link_html}</a> — {desc}</li>");
    }
    out.push_str("          <li><a href=\"https://github.com/HauserGroup/curvebook\">GitHub repository <span aria-hidden=\"true\">↗</span></a> — source code, issue tracker, and contributor documentation.</li>\n        </ul>\n      </article>\n    </main>\n");
    write_doc_close(&mut out, &format!("{base_path}privacy.html"), "Privacy Policy", "", None);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs_site::markdown::{IdentityResolver, TocHeading, render_markdown};

    #[test]
    fn redirects_escaping_toc_placement_and_release_banners() {
        // Redirect shims must not pair `noindex` with a canonical pointing elsewhere (`R0.36`).
        let idx = render_release_redirect_index("latest", "v0.1.0", &[String::from("v0.1")]);
        assert!(!idx.contains("name=\"robots\"") && idx.contains("https://curvebook.org/manual/latest/index.html") && !idx.contains("og:image"));
        let shim = render_root_redirect_shim("Troubleshooting", "Desc", "manual/latest/troubleshooting.html#known-issues", "v0.1.0");
        assert!(
            !shim.contains("name=\"robots\"")
                && shim.contains("href=\"https://curvebook.org/manual/latest/troubleshooting.html\"")
                && shim.contains("url=manual/latest/troubleshooting.html#known-issues")
        );
        assert!(render_not_found_page("/curvebook/\" onclick=\"x/").contains("&quot; onclick=&quot;x"));

        let dev_meta = SiteMeta {
            version_label: "v0.1.0".into(),
            commit_sha: Some("0123456789abcdef0123456789abcdef01234567".into()),
            release_date: Some("2026-11-26".into()),
            is_release_snapshot: false,
            is_tagged_release: false,
        };
        let ren = render_markdown("Intro.\n\n# CLI (`curvebook-cli`) {#custom}\n\n## Opts\n\n### Global\n\n## Cmds\n", &IdentityResolver);
        let page = render_page(&PageSpec {
            title: "CLI",
            description: "CLI",
            rel_path: "automation/cli.html",
            section_title: "Auto",
            section_rel_path: Some("sec\" onclick=\"x.html"),
            nav_group: Some(NavGroup::Automation),
            active_section_id: Some("automation"),
            sidebar_groups: &[],
            h1: "CLI",
            toc_headings: &ren.headings,
            body_html: &ren.html,
            body_has_h1: true,
            prev: None,
            next: None,
            meta: &dev_meta,
        });
        assert!(page.contains("sec&quot; onclick=&quot;x.html") && page.contains("Development build · main @ <code>0123456789</code> · 2026-11-26"));
        let (p0, p1, p2) = (page.find("<p>Intro.</p>").unwrap(), page.find("<h1 id=\"custom\"").unwrap(), page.find("<nav class=\"toc\"").unwrap());
        assert!(p0 < p1 && p1 < p2 && page.contains("<ul>\n                <li><a href=\"#global\">Global</a></li>\n              </ul>"));

        let many: Vec<TocHeading> = (1..=9).map(|n| TocHeading { level: 2, id: format!("h-{n}"), text: format!("H {n}") }).collect();
        let tag_meta = SiteMeta { is_tagged_release: true, is_release_snapshot: true, ..dev_meta };
        let page_c = render_page(&PageSpec {
            title: "C",
            description: "C",
            rel_path: "registry/commands.html",
            section_title: "Auto",
            section_rel_path: None,
            nav_group: Some(NavGroup::Automation),
            active_section_id: None,
            sidebar_groups: &[],
            h1: "C",
            toc_headings: &many,
            body_html: "<h2 id=\"h-1\">H</h2>",
            body_has_h1: false,
            prev: None,
            next: None,
            meta: &tag_meta,
        });
        assert!(page_c.contains("<details class=\"toc\">") && !page_c.contains("<details open") && page_c.contains("Documents v0.1.0 · 2026-11-26"));
    }
}
