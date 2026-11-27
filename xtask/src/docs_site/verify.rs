//! Static HTML, security/CSP, registry coverage, link/fragment, snapshot manifest,
//! readability lint, and compliance verification for `docs-site --check`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use curvebook_platform::read_regular_file_bytes;

use super::markdown::parse_fence_open;
use super::sitemap::GeneratedSite;
use super::template::STRICT_CSP;
use crate::scan_util::{
    collect_files_no_symlinks, extract_html_tag_name, find_html_comment_end, find_html_tag_close, has_ext_ci, is_html_tag_start, is_real_dir,
    is_regular_file, parse_html_tag_attributes, parse_meta_refresh_url, read_dir_paths_sorted, rel_slash_lossy, skip_html_raw_or_rcdata_element,
    walk_file_paths,
};
use crate::text_decode::unescape_html_attr;

const MAX_SNAPSHOT_FILE_BYTES: usize = 32 * 1024 * 1024;
const DOCS_SYMLINK_ERR: &str = "symlinks are forbidden inside documentation directories";
pub const READABILITY_LINT_EXEMPTIONS: &[&str] = &[];

#[derive(Debug, Default)]
struct PageFacts {
    ids: BTreeSet<String>,
    hrefs: Vec<String>,
    asset_srcs: Vec<String>,
    meta_refresh_targets: Vec<String>,
    meta_descriptions: Vec<String>,
    titles: Vec<String>,
    has_strict_csp: bool,
    has_no_referrer: bool,
    has_skip_link: bool,
    has_main_landmark: bool,
    has_h1: bool,
}

impl PageFacts {
    fn local_targets(&self) -> impl Iterator<Item = &str> {
        self.hrefs
            .iter()
            .filter(|h| !is_external_url(h) && !norm_scheme(h).starts_with("mailto:"))
            .chain(&self.asset_srcs)
            .chain(&self.meta_refresh_targets)
            .map(String::as_str)
    }
}

pub fn verify_site(
    root: &Path,
    preview_out_dir: &Path,
    preview_site: &GeneratedSite,
    manual_dir: &Path,
    manual_site: Option<&GeneratedSite>,
) -> Result<(), String> {
    verify_generated_pages(root, preview_out_dir, preview_site)?;
    run_hygiene_scanners(root, preview_out_dir, false)?;
    if let Some(site) = manual_site {
        verify_generated_pages(root, &manual_dir.join("latest"), site)?;
    }
    check_guide_readability(root)?;
    verify_workspace_and_manual(root, manual_dir)
}

pub fn verify_manual_snapshot_build(root: &Path, manual_dir: &Path, snapshot_sub: &str, manual_site: &GeneratedSite) -> Result<(), String> {
    verify_generated_pages(root, &manual_dir.join(snapshot_sub), manual_site)?;
    check_guide_readability(root)?;
    verify_workspace_and_manual(root, manual_dir)
}

fn verify_generated_pages(root: &Path, out_dir: &Path, site: &GeneratedSite) -> Result<(), String> {
    check_exclusions(site)?;
    let mut facts = BTreeMap::new();
    for (rel, html) in &site.html_pages {
        facts.insert(rel.clone(), audit_single_html(rel, html)?);
    }
    check_unique_titles_and_descriptions(&facts)?;
    check_page_map_links_and_fragments(root, out_dir, site.meta.is_release_snapshot, &facts)?;
    check_registry_coverage(&facts, site)
}

fn verify_workspace_and_manual(root: &Path, manual_dir: &Path) -> Result<(), String> {
    check_no_committed_manual_html(root)?;
    check_no_committed_generated_site_files(root)?;
    check_manual_snapshots(root, manual_dir)?;
    check_root_pages(root, manual_dir)?;
    check_software_citation_sync(root)?;
    super::seo::check_search_metadata_with_manual(root, Some(manual_dir))?;
    crate::pages_site::check_publish_set_with_manual(root, Some(manual_dir))?;
    run_hygiene_scanners(root, manual_dir, true)
}

fn check_software_citation_sync(root: &Path) -> Result<(), String> {
    use crate::scan_util::normalize_newlines;
    use curvebook_doc::CURVEBOOK_CITATION;

    let cargo_toml = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| format!("Cargo.toml: {e}"))?;
    let ws_ver = crate::version::read_workspace_version(&cargo_toml)?;
    let expected_cff = CURVEBOOK_CITATION.render_citation_cff(&ws_ver);
    let actual_cff = std::fs::read_to_string(root.join("CITATION.cff")).map_err(|e| format!("CITATION.cff: {e}"))?;
    if normalize_newlines(&actual_cff) != normalize_newlines(&expected_cff) {
        return Err(String::from("CITATION.cff drifted from curvebook_doc::CURVEBOOK_CITATION (or Cargo.toml version)"));
    }

    let plain_ref = CURVEBOOK_CITATION.plain_text_reference(None);
    let bibtex = CURVEBOOK_CITATION.bibtex(None);
    let index_html = normalize_newlines(&std::fs::read_to_string(root.join("docs/index.html")).map_err(|e| format!("docs/index.html: {e}"))?);
    for needle in [
        "popovertarget=\"cite-popover\"",
        "id=\"cite-popover\"",
        " popover ",
        "CITATION.cff",
        "manual/latest/get-started.html#how-to-cite",
        plain_ref.as_str(),
        bibtex.as_str(),
    ] {
        if !index_html.contains(needle) {
            return Err(format!("docs/index.html: citation popover missing synced token `{needle}`"));
        }
    }

    let get_started = normalize_newlines(
        &std::fs::read_to_string(root.join("docs/reference/get-started.md")).map_err(|e| format!("docs/reference/get-started.md: {e}"))?,
    );
    for needle in ["{#how-to-cite}", "CITATION.cff", plain_ref.as_str(), bibtex.as_str()] {
        if !get_started.contains(needle) {
            return Err(format!("docs/reference/get-started.md: How to cite section missing synced token `{needle}`"));
        }
    }

    let readme = normalize_newlines(&std::fs::read_to_string(root.join("README.md")).map_err(|e| format!("README.md: {e}"))?);
    for needle in ["CITATION.cff", plain_ref.as_str(), bibtex.as_str()] {
        if !readme.contains(needle) {
            return Err(format!("README.md: Citing CurveBook section missing synced token `{needle}`"));
        }
    }

    let faq_html = normalize_newlines(&std::fs::read_to_string(root.join("docs/faq.html")).map_err(|e| format!("docs/faq.html: {e}"))?);
    match CURVEBOOK_CITATION.concept_doi {
        None => {
            for (rel, body) in [
                ("docs/index.html", index_html.as_str()),
                ("docs/faq.html", faq_html.as_str()),
                ("docs/reference/get-started.md", get_started.as_str()),
                ("README.md", readme.as_str()),
            ] {
                let lower = body.to_ascii_lowercase();
                for forbidden in ["10.5281/zenodo.", "easy to cite"] {
                    if lower.contains(forbidden) {
                        return Err(format!("{rel}: `{forbidden}` is forbidden before CURVEBOOK_CITATION.concept_doi is minted and resolving"));
                    }
                }
            }
        }
        Some(doi) => {
            for (rel, body) in
                [("docs/index.html", index_html.as_str()), ("docs/reference/get-started.md", get_started.as_str()), ("README.md", readme.as_str())]
            {
                if !body.contains(doi) {
                    return Err(format!("{rel}: missing minted concept DOI `{doi}`"));
                }
            }
        }
    }
    Ok(())
}

fn check_unique_titles_and_descriptions(facts: &BTreeMap<String, PageFacts>) -> Result<(), String> {
    for (label, is_title) in [("`<title>`", true), ("`<meta name=\"description\">`", false)] {
        let mut seen: BTreeMap<String, &str> = BTreeMap::new();
        for (rel, f) in facts {
            let vals = if is_title { &f.titles } else { &f.meta_descriptions };
            if vals.len() != 1 || vals[0].trim().is_empty() {
                return Err(format!("{rel}: expected exactly one non-empty {label}, found {}", vals.len()));
            }
            let val = vals[0].trim();
            if let Some(prev) = seen.insert(val.to_ascii_lowercase(), rel.as_str()) {
                return Err(format!("{rel}: duplicate {label} already used by `{prev}`: \"{val}\""));
            }
        }
    }
    Ok(())
}

fn check_exclusions(site: &GeneratedSite) -> Result<(), String> {
    for rel in site.html_pages.keys() {
        let l = rel.to_ascii_lowercase();
        if l.contains("roadmap")
            || l.contains("status")
            || ["audit/", "planning/", "evidence/", "history/", "contribute/"].iter().any(|p| l.starts_with(p))
            || l.contains("/contribute/")
            || l.ends_with("-parity.html")
        {
            return Err(format!("excluded internal document `{rel}` must never be published in docs-site output"));
        }
    }
    Ok(())
}

fn audit_single_html(rel_path: &str, html: &str) -> Result<PageFacts, String> {
    let mut facts = PageFacts::default();
    scan_html_tags(rel_path, html, &mut facts)?;
    for (ok, msg) in [
        (facts.has_strict_csp, "missing strict Content-Security-Policy meta tag"),
        (facts.has_no_referrer, "missing `<meta name=\"referrer\" content=\"no-referrer\">`"),
        (facts.has_skip_link, "missing skip link to `#main`"),
        (facts.has_main_landmark, "missing `<main id=\"main\">` destination"),
        (facts.has_h1, "missing `<h1>` heading"),
    ] {
        if !ok {
            return Err(format!("{rel_path}: {msg}"));
        }
    }
    Ok(facts)
}

fn scan_html_tags(rel_path: &str, html: &str, facts: &mut PageFacts) -> Result<(), String> {
    let (mut rest, mut code_depth) = (html, 0usize);
    while let Some((before_lt, after_lt)) = rest.split_once('<') {
        if code_depth == 0 {
            check_unrendered_markup(rel_path, before_lt)?;
        }
        if !is_html_tag_start(after_lt) {
            rest = after_lt;
            continue;
        }
        if let Some(after_c) = after_lt.strip_prefix("!--") {
            rest =
                find_html_comment_end(after_c).and_then(|e| after_c.get(e..)).ok_or_else(|| format!("{rel_path}: unclosed HTML comment `<!--`"))?;
            continue;
        }
        let (tag_body, tail) = find_html_tag_close(after_lt)
            .and_then(|e| after_lt.split_at_checked(e))
            .ok_or_else(|| format!("{rel_path}: unclosed HTML tag or attribute quote"))?;
        rest = tail.strip_prefix('>').unwrap_or(tail);
        let trimmed = tag_body.trim();
        let tag_lower = extract_html_tag_name(trimmed).to_ascii_lowercase();
        if trimmed.starts_with('/') {
            if matches!(tag_lower.as_str(), "code" | "pre") {
                code_depth = code_depth.saturating_sub(1);
            }
            continue;
        }
        if matches!(tag_lower.as_str(), "script" | "style" | "base" | "form" | "iframe" | "object" | "embed") {
            return Err(format!("{rel_path}: forbidden HTML element `<{tag_lower}>`"));
        }
        facts.has_h1 |= tag_lower == "h1";
        code_depth += usize::from(matches!(tag_lower.as_str(), "code" | "pre") && !trimmed.ends_with('/'));
        process_tag_attrs(rel_path, &tag_lower, &parse_html_tag_attributes(trimmed), facts)?;
        if matches!(tag_lower.as_str(), "title" | "textarea") {
            let (body_end, after_close) = skip_html_raw_or_rcdata_element(rest, 0, &tag_lower).map_err(|e| format!("{rel_path}: {e}"))?;
            if tag_lower == "title" {
                facts.titles.push(unescape_html_attr(rest.get(..body_end).unwrap_or("").trim()));
            }
            rest = rest.get(after_close..).unwrap_or("");
        }
    }
    if code_depth == 0 {
        check_unrendered_markup(rel_path, rest)?;
    }
    Ok(())
}

fn check_meta_tag_attrs(rel_path: &str, equiv: Option<&str>, name: Option<&str>, content: Option<&str>, facts: &mut PageFacts) -> Result<(), String> {
    if let Some(eq) = equiv {
        if eq.eq_ignore_ascii_case("content-security-policy") {
            if content.unwrap_or("").trim() != STRICT_CSP {
                return Err(format!("{rel_path}: non-strict Content-Security-Policy meta tag"));
            }
            facts.has_strict_csp = true;
        } else if eq.eq_ignore_ascii_case("refresh")
            && let Some(t) = content.and_then(parse_meta_refresh_url)
        {
            if is_forbidden_uri_scheme(t) || is_external_url(t) {
                return Err(format!("{rel_path}: external or forbidden `<meta http-equiv=\"refresh\">` target `{t}`"));
            }
            facts.meta_refresh_targets.push(t.to_owned());
        }
    }
    if let Some(mn) = name {
        if mn.eq_ignore_ascii_case("referrer") {
            if !content.unwrap_or("").trim().eq_ignore_ascii_case("no-referrer") || facts.has_no_referrer {
                return Err(format!("{rel_path}: invalid or duplicate `<meta name=\"referrer\">`"));
            }
            facts.has_no_referrer = true;
        } else if mn.eq_ignore_ascii_case("description") {
            facts.meta_descriptions.push(content.unwrap_or("").trim().to_owned());
        }
    }
    Ok(())
}

fn process_tag_attrs(rel_path: &str, tag_lower: &str, attrs: &[(String, String)], facts: &mut PageFacts) -> Result<(), String> {
    let (mut seen, mut meta_equiv, mut meta_name, mut meta_content) = (BTreeSet::new(), None, None, None);
    let (mut skip_cls, mut href_main) = (false, false);
    let mut rels = attrs.iter().filter(|(k, _)| k.eq_ignore_ascii_case("rel")).map(|(_, v)| v.trim());
    let is_canonical = tag_lower == "link" && rels.next().is_some_and(|v| v.eq_ignore_ascii_case("canonical")) && rels.next().is_none();

    for (k, v) in attrs {
        if k.contains(['"', '\'', '<']) {
            return Err(format!("{rel_path}: malformed HTML attribute name `{k}` on `<{tag_lower}>`"));
        }
        let kl = k.to_ascii_lowercase();
        if !seen.insert(kl.clone()) {
            return Err(format!("{rel_path}: duplicate HTML attribute `{kl}=` on `<{tag_lower}>`"));
        }
        if kl == "style" || kl == "ping" || (kl.starts_with("on") && kl.len() > 2) {
            return Err(format!("{rel_path}: forbidden attribute `{kl}=` on `<{tag_lower}>`"));
        }
        if kl == "id" && !v.is_empty() {
            if !facts.ids.insert(v.clone()) {
                return Err(format!("{rel_path}: duplicate HTML id `#{v}`"));
            }
            facts.has_main_landmark |= tag_lower == "main" && v == "main";
        }
        skip_cls |= tag_lower == "a" && kl == "class" && v.split_whitespace().any(|c| c == "skip-link");
        match kl.as_str() {
            "href" if is_canonical => {
                if !v.starts_with(super::seo::SITE_BASE_URL) {
                    return Err(format!("{rel_path}: canonical link `{v}` must point inside `{}`", super::seo::SITE_BASE_URL));
                }
            }
            "href" => {
                if is_forbidden_uri_scheme(v) || (tag_lower == "link" && is_external_url(v)) {
                    return Err(format!("{rel_path}: forbidden or external `<{tag_lower} href=\"{v}\">`"));
                }
                href_main |= tag_lower == "a" && v.trim() == "#main";
                facts.hrefs.push(v.clone());
            }
            "src" | "poster" | "action" | "formaction" | "xlink:href" => {
                if is_external_url(v) || is_forbidden_uri_scheme(v) {
                    return Err(format!("{rel_path}: external or forbidden `{kl}` asset (`{v}`)"));
                }
                facts.asset_srcs.push(v.clone());
            }
            "srcset" | "imagesrcset" => {
                if is_forbidden_uri_scheme(v) {
                    return Err(format!("{rel_path}: forbidden `{kl}` asset URI (`{v}`)"));
                }
                for url in v.split(',').filter_map(|c| c.split_whitespace().next()) {
                    if is_external_url(url) || is_forbidden_uri_scheme(url) {
                        return Err(format!("{rel_path}: external or forbidden `{kl}` asset (`{url}`)"));
                    }
                    facts.asset_srcs.push(url.to_owned());
                }
            }
            "http-equiv" if tag_lower == "meta" => meta_equiv = Some(v.trim()),
            "name" if tag_lower == "meta" => meta_name = Some(v.trim()),
            "content" if tag_lower == "meta" => meta_content = Some(v.as_str()),
            _ => {}
        }
    }
    facts.has_skip_link |= skip_cls && href_main;
    if tag_lower == "meta" {
        check_meta_tag_attrs(rel_path, meta_equiv, meta_name, meta_content, facts)?;
    } else if tag_lower == "img" {
        if !attrs.iter().any(|(k, _)| k.eq_ignore_ascii_case("alt")) {
            return Err(format!("{rel_path}: `<img>` must declare an `alt` attribute"));
        }
        if rel_path.starts_with("docs/") {
            let ok_dim = |n: &str| attrs.iter().any(|(k, v)| k.eq_ignore_ascii_case(n) && v.trim().parse::<u32>().is_ok_and(|x| x > 0));
            if !ok_dim("width") || !ok_dim("height") {
                return Err(format!("{rel_path}: root page `<img>` must declare positive integer `width` and `height`"));
            }
        }
    }
    Ok(())
}

fn check_unrendered_markup(rel_path: &str, text: &str) -> Result<(), String> {
    if let Some(m) =
        ["\\frac{", "\\dfrac{", "\\tfrac{", "\\text{", "\\sqrt{", "\\chi", "\\Phi", "\\TeX", "\\ge ", "\\le "].into_iter().find(|m| text.contains(m))
    {
        return Err(format!("{rel_path}: unrendered TeX macro `{m}` leaked outside <code>/<pre>"));
    }
    if let Some(m) = ["[!NOTE]", "[!TIP]", "[!IMPORTANT]", "[!WARNING]", "[!CAUTION]"].into_iter().find(|m| text.contains(m)) {
        return Err(format!("{rel_path}: unrendered GitHub callout marker `{m}` leaked outside <code>/<pre>"));
    }
    Ok(())
}

fn norm_scheme(raw: &str) -> String {
    raw.chars().filter(|c| !c.is_ascii_control() && !c.is_ascii_whitespace()).map(|c| c.to_ascii_lowercase()).collect()
}
fn is_forbidden_uri_scheme(raw: &str) -> bool {
    let n = norm_scheme(raw);
    n.starts_with("javascript:") || n.starts_with("data:") || n.starts_with("vbscript:")
}
fn is_external_url(url: &str) -> bool {
    let n = norm_scheme(url);
    n.starts_with("http://") || n.starts_with("https://") || n.starts_with("//")
}

fn check_page_map_links_and_fragments(
    root: &Path,
    out_dir: &Path,
    is_release_snapshot: bool,
    facts: &BTreeMap<String, PageFacts>,
) -> Result<(), String> {
    let mut cache = BTreeMap::new();
    for (page_rel, pf) in facts {
        for raw in pf.local_targets() {
            if is_external_url(raw) || is_forbidden_uri_scheme(raw) {
                return Err(format!("{page_rel}: external or forbidden link target `{raw}`"));
            }
            verify_target_with_cache(root, out_dir, is_release_snapshot, facts, page_rel, raw, &mut cache)?;
        }
    }
    Ok(())
}

fn check_cached_html_fragment(
    target_path: &Path,
    cache_key: &str,
    scan_label: &str,
    frag: &str,
    cache: &mut BTreeMap<String, BTreeSet<String>>,
    err_msg: impl FnOnce() -> String,
) -> Result<(), String> {
    if !cache.contains_key(cache_key) {
        let html = std::fs::read_to_string(target_path).map_err(|e| format!("read {}: {e}", target_path.display()))?;
        let mut f = PageFacts::default();
        scan_html_tags(scan_label, &html, &mut f)?;
        cache.insert(cache_key.to_owned(), f.ids);
    }
    if cache.get(cache_key).is_some_and(|ids| !ids.contains(frag)) { Err(err_msg()) } else { Ok(()) }
}

fn split_href_fragment(raw_ref: &str) -> (&str, Option<&str>) {
    raw_ref.split_once('#').map_or((raw_ref, None), |(t, f)| (t, Some(f).filter(|s| !s.is_empty())))
}

fn verify_same_page_anchor(label: &str, ids: &BTreeSet<String>, frag: Option<&str>) -> Result<(), String> {
    match frag {
        Some(f) if !ids.contains(f) => Err(format!("{label}: broken same-page anchor `#{f}`")),
        _ => Ok(()),
    }
}

fn verify_target_with_cache(
    root: &Path,
    out_dir: &Path,
    is_release_snapshot: bool,
    facts: &BTreeMap<String, PageFacts>,
    from_rel: &str,
    raw_ref: &str,
    cache: &mut BTreeMap<String, BTreeSet<String>>,
) -> Result<(), String> {
    let (target_part, frag_part) = split_href_fragment(raw_ref);
    if target_part.is_empty() {
        return verify_same_page_anchor(from_rel, &facts[from_rel].ids, frag_part);
    }
    let resolved = resolve_relative_segments(from_rel, target_part)
        .ok_or_else(|| format!("{from_rel}: root-absolute link `{raw_ref}` is forbidden (use relative paths)"))?;
    if is_release_snapshot && let Some(docs_rel) = resolved.strip_prefix("../../") {
        if docs_rel.is_empty() || docs_rel.starts_with('/') || docs_rel.split('/').any(|s| s == "..") {
            return Err(format!("{from_rel}: relative target `{raw_ref}` escapes `docs/` root (`{resolved}`)"));
        }
        let target_path = root.join("docs").join(docs_rel);
        if !is_regular_file(&target_path) {
            return Err(format!("{from_rel}: broken parent-docs asset/page `{raw_ref}` -> `{}`", target_path.display()));
        }
        if let Some(frag) = frag_part
            && docs_rel.ends_with(".html")
        {
            let key = format!("docs/{docs_rel}");
            check_cached_html_fragment(&target_path, &key, &key, frag, cache, || {
                format!("{from_rel}: broken parent-docs fragment `{raw_ref}` (missing `#{frag}` in `{key}`)")
            })?;
        }
        return Ok(());
    }
    if resolved.starts_with("..") {
        return Err(format!("{from_rel}: relative target `{raw_ref}` escapes site root (`{resolved}`)"));
    }
    if let Some(tf) = facts.get(&resolved) {
        return match frag_part {
            Some(frag) if !tf.ids.contains(frag) => Err(format!("{from_rel}: broken fragment `{raw_ref}` (missing `#{frag}` in `{resolved}`)")),
            _ => Ok(()),
        };
    }
    if is_regular_file(&out_dir.join(&resolved)) {
        Ok(())
    } else {
        Err(format!("{from_rel}: broken internal link `{raw_ref}` (resolved `{resolved}` missing from site output)"))
    }
}

fn resolve_relative_segments(from_rel: &str, target: &str) -> Option<String> {
    if target.starts_with(['/', '\\']) {
        return None;
    }
    let mut parts: Vec<String> = Path::new(from_rel)
        .parent()
        .into_iter()
        .flat_map(Path::components)
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str().filter(|x| !x.is_empty()).map(str::to_owned),
            _ => None,
        })
        .collect();
    let mut ups = 0usize;
    for seg in target.replace('\\', "/").split('/') {
        match seg {
            "" | "." => {}
            ".." => ups += usize::from(parts.pop().is_none()),
            other => parts.push(other.to_owned()),
        }
    }
    let mut out = vec![String::from(".."); ups];
    out.extend(parts);
    Some(out.join("/"))
}

fn check_registry_coverage(facts: &BTreeMap<String, PageFacts>, site: &GeneratedSite) -> Result<(), String> {
    let az_set: BTreeSet<&str> = facts.get("az-index.html").ok_or("missing `az-index.html`")?.hrefs.iter().map(String::as_str).collect();
    let e = &site.expected_coverage;
    for (page, ids, label) in [
        ("registry/commands.html", &e.engine_commands, "engine command"),
        ("registry/commands.html", &e.ui_commands, "UI command"),
        ("registry/mcp-tools.html", &e.mcp_tools, "MCP tool"),
        ("registry/mcp-tools.html", &e.mcp_prompts, "MCP prompt"),
        ("registry/cli.html", &e.cli_commands, "CLI subcommand"),
        ("registry/equations.html", &e.fit_equations, "fit equation"),
        ("registry/graphs.html", &e.graph_kinds, "graph kind"),
        ("registry/graphs.html", &e.graph_options, "graph option"),
        ("registry/tables.html", &e.table_kinds, "table kind"),
        ("registry/formats.html", &e.formats, "format descriptor"),
        ("registry/formats.html", &e.preference_keys, "preference schema key"),
    ] {
        let pf = facts.get(page).ok_or_else(|| format!("missing generated page `{page}`"))?;
        for id in ids {
            if !pf.ids.contains(id) {
                return Err(format!("missing {label} anchor `#{id}` in `{page}`"));
            }
            if !az_set.contains(format!("{page}#{id}").as_str()) {
                return Err(format!("missing {label} entry `{page}#{id}` in `az-index.html`"));
            }
        }
    }
    let tables = facts.get("registry/tables.html").ok_or("missing `registry/tables.html`")?;
    for slug in &e.analysis_kinds {
        let (reg_id, doc_id) = (format!("analysis-kind-{slug}"), format!("analysis-{slug}"));
        if !tables.ids.contains(&reg_id) {
            return Err(format!("missing analysis kind anchor `#{reg_id}` in `registry/tables.html`"));
        }
        if az_set.contains(format!("analyses.html#{doc_id}").as_str()) {
            if !facts.get("analyses.html").is_some_and(|a| a.ids.contains(&doc_id)) {
                return Err(format!("missing analysis kind anchor `#{doc_id}` in `analyses.html`"));
            }
        } else if !az_set.contains(format!("registry/tables.html#{reg_id}").as_str()) {
            return Err(format!("missing analysis kind entry for `{slug}` in `az-index.html`"));
        }
    }
    let mcp_html = site.html_pages.get("registry/mcp-tools.html").ok_or("missing `registry/mcp-tools.html`")?;
    for scheme in ["sheet://", "result://"] {
        if !mcp_html.contains(scheme) {
            return Err(format!("`registry/mcp-tools.html` missing MCP resource scheme `{scheme}`"));
        }
    }
    Ok(())
}

#[must_use]
pub fn parse_minor_dir_version(name: &str) -> Option<(u64, u64)> {
    crate::pages_site::parse_minor_dir_version(name)
}

pub fn check_no_committed_manual_html(root: &Path) -> Result<(), String> {
    let manual_dir = root.join("docs/manual");
    let Ok(meta) = std::fs::symlink_metadata(&manual_dir) else { return Ok(()) };
    if meta.file_type().is_symlink() {
        return Err(String::from("docs/manual must not be a symlink"));
    }
    if !meta.file_type().is_dir() {
        return Ok(());
    }
    let mut files = Vec::new();
    collect_files_no_symlinks(&manual_dir, &manual_dir, DOCS_SYMLINK_ERR, &mut files)?;
    let mut htmls: Vec<String> = files.into_iter().filter(|r| has_ext_ci(Path::new(r), &["html", "htm"])).collect();
    if htmls.is_empty() {
        return Ok(());
    }
    htmls.sort();
    Err(format!(
        "docs/manual/ contains {} HTML file(s) (e.g. {}); manual HTML must not be committed under docs/manual/",
        htmls.len(),
        htmls.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
    ))
}

pub fn check_no_committed_generated_site_files(root: &Path) -> Result<(), String> {
    let found: Vec<String> = crate::pages_site::GENERATED_SITE_FILES
        .iter()
        .filter(|rel| std::fs::symlink_metadata(root.join("docs").join(rel)).is_ok())
        .map(|rel| format!("docs/{rel}"))
        .collect();
    if found.is_empty() { Ok(()) } else { Err(format!("generated site file(s) {} must not be committed under docs/", found.join(", "))) }
}

fn audit_snapshot_dir_on_disk(root: &Path, snap_dir: &Path, snap_name: &str) -> Result<BTreeMap<String, Vec<u8>>, String> {
    if !is_real_dir(snap_dir) {
        return Err(format!("{}: manual snapshot must be a regular non-symlink directory", snap_dir.display()));
    }
    let (mut rel_files, mut file_bytes, mut facts) = (Vec::new(), BTreeMap::new(), BTreeMap::new());
    collect_files_no_symlinks(snap_dir, snap_dir, DOCS_SYMLINK_ERR, &mut rel_files)?;
    for rel in rel_files {
        let bytes = read_regular_file_bytes(&snap_dir.join(&rel), MAX_SNAPSHOT_FILE_BYTES).map_err(|e| format!("read {rel}: {e}"))?;
        if has_ext_ci(Path::new(&rel), &["html", "htm"]) {
            let html = String::from_utf8(bytes.clone()).map_err(|_| format!("{snap_name}/{rel}: snapshot HTML is not valid UTF-8"))?;
            facts.insert(rel.clone(), audit_single_html(&format!("{snap_name}/{rel}"), &html)?);
        }
        file_bytes.insert(rel, bytes);
    }
    if facts.is_empty() {
        return Err(format!("{}: manual snapshot contains no HTML pages", snap_dir.display()));
    }
    check_unique_titles_and_descriptions(&facts)?;
    check_page_map_links_and_fragments(root, snap_dir, true, &facts)?;
    Ok(file_bytes)
}

fn check_snapshot_manifest(root: &Path, manifest_path: &Path) -> Result<String, String> {
    if !is_regular_file(manifest_path) {
        return Err(format!("{}: snapshot manifest must be a regular non-symlink file", manifest_path.display()));
    }
    let raw = std::fs::read_to_string(manifest_path).map_err(|e| format!("read {}: {e}", manifest_path.display()))?;
    let val: serde_json::Value = serde_json::from_str(&raw).map_err(|e| format!("parse {}: {e}", manifest_path.display()))?;
    let version = val.get("version").and_then(serde_json::Value::as_str).ok_or_else(|| format!("{}: missing `version`", manifest_path.display()))?;
    let is_ws = val.get("source").and_then(serde_json::Value::as_str) == Some("workspace");
    let commit = match val.get("commit") {
        Some(serde_json::Value::String(s)) => Some(s.as_str()),
        Some(serde_json::Value::Null) | None if is_ws => None,
        _ => return Err(format!("{}: missing `commit`", manifest_path.display())),
    };
    if let Some(sha) = commit {
        if sha.len() != 40 || !sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(format!("{}: `commit` `{sha}` must be a 40-hex SHA", manifest_path.display()));
        }
        if !is_ws {
            for tag in [format!("v{version}"), version.to_owned()] {
                if let Some(tag_sha) = resolve_git_tag_commit(root, &tag)?
                    && tag_sha != sha
                {
                    return Err(format!(
                        "{}: manifest commit `{sha}` does not match release tag `{tag}` commit `{tag_sha}`",
                        manifest_path.display()
                    ));
                }
            }
        }
    }
    Ok(raw)
}

pub fn check_manual_snapshots(root: &Path, manual_dir: &Path) -> Result<(), String> {
    if !is_real_dir(manual_dir) {
        return Err(format!("manual build directory `{}` does not exist", manual_dir.display()));
    }
    let mut snapshots = Vec::new();
    for path in read_dir_paths_sorted(manual_dir)? {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if let Some(ver) = parse_minor_dir_version(&name) {
            let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("stat {}: {e}", path.display()))?;
            if meta.file_type().is_symlink() {
                return Err(format!("{}: symlinked snapshot directory is forbidden", path.display()));
            }
            let mp = path.join("manifest.json");
            if std::fs::symlink_metadata(&mp).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(format!("{}: symlinked snapshot manifest is forbidden", mp.display()));
            }
            if meta.file_type().is_dir() && is_regular_file(&mp) {
                snapshots.push((ver, name));
            }
        }
    }
    snapshots.sort_by_key(|(v, _)| *v);
    let Some((_, newest_dir)) = snapshots.last() else {
        return Err(format!("manual build directory `{}` contains no `v<major>.<minor>/manifest.json` snapshot", manual_dir.display()));
    };
    let (mut newest_tree, mut newest_manifest) = (None, String::new());
    for (_, snap) in &snapshots {
        let raw = check_snapshot_manifest(root, &manual_dir.join(snap).join("manifest.json"))?;
        let tree = audit_snapshot_dir_on_disk(root, &manual_dir.join(snap), snap)?;
        if snap == newest_dir {
            (newest_manifest, newest_tree) = (raw, Some(tree));
        }
    }
    let latest_dir = manual_dir.join("latest");
    let expected_refresh = if std::fs::symlink_metadata(&latest_dir).is_ok() {
        if !is_real_dir(&latest_dir) {
            return Err(format!("{}: `latest/` must be a regular non-symlink directory", latest_dir.display()));
        }
        if check_snapshot_manifest(root, &latest_dir.join("manifest.json"))? != newest_manifest {
            return Err(format!("{} does not match newest snapshot", latest_dir.join("manifest.json").display()));
        }
        if newest_tree.as_ref() != Some(&audit_snapshot_dir_on_disk(root, &latest_dir, "latest")?) {
            return Err(format!("{} file tree does not match newest snapshot byte-for-byte", latest_dir.display()));
        }
        String::from("latest/index.html")
    } else {
        format!("{newest_dir}/index.html")
    };
    let index_path = manual_dir.join("index.html");
    if !is_regular_file(&index_path) {
        return Err(format!("{} must be a regular non-symlink file", index_path.display()));
    }
    let idx_facts =
        audit_single_html("manual/index.html", &std::fs::read_to_string(&index_path).map_err(|e| format!("read {}: {e}", index_path.display()))?)?;
    if !idx_facts.meta_refresh_targets.iter().any(|t| t == &expected_refresh) {
        return Err(format!("{} must redirect via meta-refresh to `{expected_refresh}`", index_path.display()));
    }
    Ok(())
}

pub fn check_root_pages(root: &Path, manual_dir: &Path) -> Result<(), String> {
    let (docs_dir, required) = (root.join("docs"), ["index.html", "privacy.html", "manual/index.html"]);
    let mut pages: Vec<&str> = required.to_vec();
    pages.extend(
        super::seo::LANDING_PAGES
            .iter()
            .copied()
            .filter(|p| !required.contains(p))
            .chain(["404.html", "get-started.html", "troubleshooting.html"])
            .filter(|s| is_regular_file(&docs_dir.join(s))),
    );
    let mut audited = BTreeMap::new();
    for rel in pages {
        let full = rel.strip_prefix("manual/").map_or_else(|| docs_dir.join(rel), |m| manual_dir.join(m));
        audited.insert(
            rel.to_owned(),
            audit_single_html(&format!("docs/{rel}"), &std::fs::read_to_string(&full).map_err(|e| format!("read {}: {e}", full.display()))?)?,
        );
    }
    check_unique_titles_and_descriptions(&audited)?;
    let mut cache = BTreeMap::new();
    for (rel, pf) in &audited {
        for raw in pf.local_targets() {
            if is_external_url(raw) || is_forbidden_uri_scheme(raw) {
                return Err(format!("docs/{rel}: external or forbidden link target `{raw}`"));
            }
            verify_docs_relative_target(&docs_dir, manual_dir, rel, raw, pf, &audited, &mut cache)?;
        }
    }
    Ok(())
}

fn verify_docs_relative_target(
    docs_dir: &Path,
    manual_dir: &Path,
    from_rel: &str,
    raw_ref: &str,
    from_facts: &PageFacts,
    audited: &BTreeMap<String, PageFacts>,
    cache: &mut BTreeMap<String, BTreeSet<String>>,
) -> Result<(), String> {
    let (target_part, frag_part) = split_href_fragment(raw_ref);
    if target_part.is_empty() {
        return verify_same_page_anchor(&format!("docs/{from_rel}"), &from_facts.ids, frag_part);
    }
    let target_part = if from_rel == "404.html" && target_part.starts_with('/') {
        let bp = crate::pages_site::site_base_path();
        let stripped =
            target_part.strip_prefix(bp).ok_or_else(|| format!("docs/404.html: root-absolute target `{raw_ref}` must start with `{bp}`"))?;
        if stripped.is_empty() { "index.html" } else { stripped }
    } else {
        target_part
    };
    let resolved =
        resolve_relative_segments(from_rel, target_part).ok_or_else(|| format!("docs/{from_rel}: root-absolute target `{raw_ref}` is forbidden"))?;
    if resolved.starts_with("..") {
        return Err(format!("docs/{from_rel}: relative target `{raw_ref}` escapes `docs/` root (`{resolved}`)"));
    }
    let target_path = if let Some(m) = resolved.strip_prefix("manual/") {
        manual_dir.join(m)
    } else if crate::pages_site::GENERATED_SITE_FILES.contains(&resolved.as_str()) {
        manual_dir.join(crate::pages_site::MANUAL_ROOT_SUBDIR).join(&resolved)
    } else {
        docs_dir.join(&resolved)
    };
    if !is_regular_file(&target_path) {
        return Err(format!("docs/{from_rel}: broken relative target `{raw_ref}` (missing `{}`)", target_path.display()));
    }
    if let Some(frag) = frag_part
        && resolved.ends_with(".html")
    {
        if let Some(known) = audited.get(&resolved) {
            return if known.ids.contains(frag) {
                Ok(())
            } else {
                Err(format!("docs/{from_rel}: broken fragment `{raw_ref}` (missing `#{frag}` in `docs/{resolved}`)"))
            };
        }
        let label = if resolved.starts_with("manual/") { resolved.clone() } else { format!("docs/{resolved}") };
        check_cached_html_fragment(&target_path, &resolved, &label, frag, cache, || {
            format!("docs/{from_rel}: broken fragment `{raw_ref}` (missing `#{frag}` in `{resolved}`)")
        })?;
    }
    Ok(())
}

fn resolve_git_tag_commit(root: &Path, tag: &str) -> Result<Option<String>, String> {
    let refspec = format!("refs/tags/{tag}^{{commit}}");
    let out = crate::dev_tool_command("git")
        .current_dir(root)
        .args(["rev-parse", "--verify", "--quiet", &refspec])
        .wall_timeout(Duration::from_secs(30))
        .idle_timeout(Duration::from_secs(10))
        .run()
        .map_err(|e| format!("git rev-parse {refspec}: {e}"))?;
    Ok(out.success.then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned()).filter(|s| !s.is_empty()))
}

fn run_hygiene_scanners(root: &Path, out_dir: &Path, is_manual_build_dir: bool) -> Result<(), String> {
    let brand_hits = if is_manual_build_dir { crate::brands::check_manual_build_dir(out_dir)? } else { crate::brands::check_docs_site_dir(out_dir)? };
    if !brand_hits.is_empty() {
        let preview: Vec<String> = brand_hits.iter().take(20).map(|h| format!("{}:{}: `{}`", h.path, h.line, h.brand)).collect();
        return Err(format!("brands scanner found {} violation(s) in generated docs-site: {}", brand_hits.len(), preview.join("; ")));
    }
    let cleanroom_hits = crate::cleanroom::scan_dist_artefacts(out_dir)?;
    if let Some(first) = cleanroom_hits.first() {
        return Err(format!(
            "cleanroom scanner found {} violation(s) in generated docs-site (first: {}:{}: `{}`)",
            cleanroom_hits.len(),
            first.path,
            first.line,
            first.signature
        ));
    }
    let (_, asset_violations) = crate::assets::scan_dist_artefacts(&crate::assets::load_repo_asset_catalog(root)?, out_dir)?;
    if let Some(first) = asset_violations.first() {
        return Err(format!("assets scanner found {} violation(s) in generated docs-site (first: {first})", asset_violations.len()));
    }
    let (mut secret_hits, mut text_files) = (Vec::new(), Vec::new());
    walk_file_paths(out_dir, &|_| false, &|p| has_ext_ci(p, &["html", "htm", "css", "json", "txt", "xml"]), &mut text_files)?;
    for path in text_files {
        let text = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        crate::secrets::scan_text(&rel_slash_lossy(out_dir, &path), &text, &mut secret_hits);
    }
    if let Some(first) = secret_hits.first() {
        return Err(format!("secrets scanner found {} violation(s) in generated docs-site (first: {first})", secret_hits.len()));
    }
    Ok(())
}

const TIER1_MAX_AVG_SENTENCE: f64 = 21.0;
const TIER1_MAX_CODE_PER_100W: f64 = 10.0;
const TIER1_MAX_HTML_WORDS: usize = 900;

const TIER1_MD_GUIDES: &[(&str, &str)] = &[
    ("docs/reference/get-started.md", "docs/reference/files.md or docs/reference/troubleshooting.md"),
    ("docs/reference/desktop-gui.md", "docs/reference/tables.md, analyses.md, or graphs.md"),
    ("docs/mcp-for-beginners.md", "docs/reference/automation/mcp.md"),
];

const TIER1_HTML_GUIDES: &[(&str, &str)] = &[
    ("docs/index.html", "manual/latest/index.html"),
    ("docs/open-pzfx-files.html", "docs/pzfx-format-specification.html or docs/reference/files.md"),
    ("docs/linux.html", "docs/reference/get-started.md"),
    ("docs/faq.html", "docs/reference/files.md or manual/latest/reference.html"),
];

pub fn check_guide_readability(root: &Path) -> Result<(), String> {
    let mut violations = Vec::new();
    for guide in super::sitemap::MANUAL_GUIDES {
        let full = root.join(guide.repo_path);
        if !is_regular_file(&full) {
            continue;
        }
        let raw = std::fs::read_to_string(&full).map_err(|e| format!("read {}: {e}", guide.repo_path))?;
        if !READABILITY_LINT_EXEMPTIONS.contains(&guide.repo_path) {
            for (idx, line) in raw.lines().enumerate() {
                if let Some(tok) = find_forbidden_guide_id(line) {
                    violations.push(format!("{}:{}: forbidden internal ID `{tok}` in user guide", guide.repo_path, idx + 1));
                }
            }
        }
        let (avg_w, code_100, rs_n) = compute_guide_readability_stats(&raw);
        let tier1_target = TIER1_MD_GUIDES.iter().find(|(p, _)| *p == guide.repo_path).map(|(_, t)| *t);
        let tier_tag = if tier1_target.is_some() { "tier1" } else { "tier2" };
        println!(
            "docs-site readability ({tier_tag}): {}: avg_sentence={avg_w:.1} words, code_per_100w={code_100:.1}, rs_refs={rs_n}",
            guide.repo_path
        );
        check_audience_banner(guide.repo_path, &raw, tier1_target.is_some(), &mut violations);
        if let Some(target) = tier1_target {
            enforce_tier1_ceilings(guide.repo_path, None, avg_w, code_100, rs_n, target, &mut violations);
        } else if rs_n > 0 {
            violations.push(format!("{}: Tier 2 user reference contains {rs_n} `.rs` source reference(s)", guide.repo_path));
        }
    }
    for &page in super::seo::LANDING_PAGES {
        let repo_path = format!("docs/{page}");
        let full = root.join(&repo_path);
        if !is_regular_file(&full) {
            continue;
        }
        let raw = std::fs::read_to_string(&full).map_err(|e| format!("read {repo_path}: {e}"))?;
        let (words, avg_w, code_100, rs_n, forbidden_id) = compute_html_readability_stats(&raw);
        if let Some(tok) = forbidden_id {
            violations.push(format!("{repo_path}: forbidden internal ID `{tok}` in visible page prose"));
        }
        let tier1_target = TIER1_HTML_GUIDES.iter().find(|(p, _)| *p == repo_path).map(|(_, t)| *t);
        let tier_tag = if tier1_target.is_some() { "tier1" } else { "tier2" };
        println!(
            "docs-site readability ({tier_tag}): {repo_path}: words={words}, avg_sentence={avg_w:.1} words, code_per_100w={code_100:.1}, rs_refs={rs_n}"
        );
        check_audience_banner(&repo_path, &raw, tier1_target.is_some(), &mut violations);
        if let Some(target) = tier1_target {
            enforce_tier1_ceilings(&repo_path, Some(words), avg_w, code_100, rs_n, target, &mut violations);
        } else if rs_n > 0 {
            violations.push(format!("{repo_path}: Tier 2 user reference contains {rs_n} `.rs` source reference(s)"));
        }
    }
    if let Some(first) = violations.first() {
        return Err(format!("user-guide readability lint found {} violation(s) (first: {first})", violations.len()));
    }
    Ok(())
}

fn check_audience_banner(repo_path: &str, raw: &str, is_tier1: bool, violations: &mut Vec<String>) {
    let expected = if is_tier1 { "<!-- Audience: Tier 1" } else { "<!-- Audience: Tier 2" };
    if !raw.lines().take(35).any(|line| line.trim().starts_with(expected)) {
        violations.push(format!("{repo_path}: missing `{expected} ... -->` audience banner near top of file (expected within first 35 lines)"));
    }
}

fn enforce_tier1_ceilings(
    repo_path: &str,
    html_words: Option<usize>,
    avg_w: f64,
    code_100: f64,
    rs_n: usize,
    tier2_target: &str,
    violations: &mut Vec<String>,
) {
    let mut exceeded = Vec::new();
    if let Some(w) = html_words
        && w > TIER1_MAX_HTML_WORDS
    {
        exceeded.push(format!("words={w} > {TIER1_MAX_HTML_WORDS}"));
    }
    if avg_w > TIER1_MAX_AVG_SENTENCE {
        exceeded.push(format!("avg_sentence={avg_w:.1} > {TIER1_MAX_AVG_SENTENCE:.1}"));
    }
    if code_100 > TIER1_MAX_CODE_PER_100W {
        exceeded.push(format!("code_per_100w={code_100:.1} > {TIER1_MAX_CODE_PER_100W:.1}"));
    }
    if rs_n > 0 {
        exceeded.push(format!("rs_refs={rs_n} > 0"));
    }
    if !exceeded.is_empty() {
        violations.push(format!(
            "{repo_path}: Tier 1 scientist guide exceeds readability ceiling ({}); keep guides short and plain, and move technical detail to {tier2_target}",
            exceeded.join(", ")
        ));
    }
}

fn count_rs_refs(text: &str) -> usize {
    let lb = text.as_bytes();
    text.match_indices(".rs")
        .filter(|&(d, _)| d > 0 && !is_word_boundary_byte(lb.get(d - 1).copied()) && is_word_boundary_byte(lb.get(d + 3).copied()))
        .count()
}

fn count_sentence_terminators(text: &str, in_code: bool) -> usize {
    if in_code {
        return 0;
    }
    let cb = text.as_bytes();
    cb.iter()
        .enumerate()
        .filter(|&(i, &c)| {
            matches!(c, b'.' | b'!' | b'?')
                && !(c == b'.' && cb.get(i + 1).is_some_and(u8::is_ascii_digit))
                && matches!(cb.get(i + 1), None | Some(b' ' | b'\t' | b'\r' | b'\n' | b')' | b']' | b'"' | b'\'' | b'*'))
        })
        .count()
}

fn compute_html_readability_stats(raw: &str) -> (usize, f64, f64, usize, Option<String>) {
    let main_slice = raw
        .find("<main")
        .and_then(|s| raw.get(s..))
        .and_then(|s| s.find('>').and_then(|gt| s.get(gt + 1..)))
        .map_or(raw, |s| s.find("</main>").and_then(|e| s.get(..e)).unwrap_or(s));
    let (mut rest, mut skip_depth, mut code_depth) = (main_slice, 0usize, 0usize);
    let (mut words, mut sents, mut code_spans, mut rs_refs) = (0usize, 0usize, 0usize, 0usize);
    let (mut block_words, mut block_sents, mut forbidden) = (0usize, 0usize, None::<String>);

    let flush_block = |words: &mut usize, sents: &mut usize, bw: &mut usize, bs: &mut usize| {
        if *bw > 0 {
            *words += *bw;
            *sents += (*bs).max(usize::from(*bw >= 4));
            *bw = 0;
            *bs = 0;
        }
    };

    while let Some((before_lt, after_lt)) = rest.split_once('<') {
        if skip_depth == 0 {
            let trimmed = before_lt.trim();
            if !trimmed.is_empty() {
                if forbidden.is_none()
                    && let Some(tok) = find_forbidden_guide_id(trimmed)
                {
                    forbidden = Some(tok.to_owned());
                }
                rs_refs += count_rs_refs(trimmed);
                block_words += trimmed.split_whitespace().count();
                block_sents += count_sentence_terminators(trimmed, code_depth > 0);
            }
        }
        if !is_html_tag_start(after_lt) {
            rest = after_lt;
            continue;
        }
        if let Some(after_c) = after_lt.strip_prefix("!--") {
            rest = find_html_comment_end(after_c).and_then(|e| after_c.get(e..)).unwrap_or("");
            continue;
        }
        let Some((tag_body, tail)) = find_html_tag_close(after_lt).and_then(|e| after_lt.split_at_checked(e)) else {
            break;
        };
        rest = tail.strip_prefix('>').unwrap_or(tail);
        let trimmed = tag_body.trim();
        let is_close = trimmed.starts_with('/');
        let tag = extract_html_tag_name(trimmed).to_ascii_lowercase();
        let is_skipped = matches!(tag.as_str(), "pre" | "nav" | "table" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
        let is_block =
            is_skipped || matches!(tag.as_str(), "p" | "li" | "div" | "section" | "aside" | "article" | "ul" | "ol" | "figure" | "figcaption");
        if is_block {
            flush_block(&mut words, &mut sents, &mut block_words, &mut block_sents);
        }
        if is_skipped {
            if is_close {
                skip_depth = skip_depth.saturating_sub(1);
            } else if !trimmed.ends_with('/') {
                skip_depth += 1;
            }
        } else if tag == "code" {
            if is_close {
                code_depth = code_depth.saturating_sub(1);
            } else if !trimmed.ends_with('/') {
                if skip_depth == 0 {
                    code_spans += 1;
                }
                code_depth += 1;
            }
        }
    }
    if skip_depth == 0 {
        let trimmed = rest.trim();
        if !trimmed.is_empty() {
            rs_refs += count_rs_refs(trimmed);
            block_words += trimmed.split_whitespace().count();
            block_sents += count_sentence_terminators(trimmed, code_depth > 0);
        }
    }
    flush_block(&mut words, &mut sents, &mut block_words, &mut block_sents);
    let eff = sents.max(usize::from(words > 0));
    let avg = if eff == 0 { 0.0 } else { words as f64 / eff as f64 };
    let code_100 = if words == 0 { 0.0 } else { code_spans as f64 * 100.0 / words as f64 };
    (words, avg, code_100, rs_refs, forbidden)
}

fn is_word_boundary_byte(b: Option<u8>) -> bool {
    !matches!(b, Some(b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'_'))
}

fn find_forbidden_guide_id(line: &str) -> Option<&str> {
    let b = line.as_bytes();
    for i in 0..b.len() {
        if !is_word_boundary_byte(i.checked_sub(1).and_then(|p| b.get(p).copied())) {
            continue;
        }
        if b[i] == b'R' {
            let d1 = b.get(i + 1..).map_or(0, |s| s.iter().take_while(|c| c.is_ascii_digit()).count());
            let d2 = if d1 > 0 && b.get(i + 1 + d1) == Some(&b'.') {
                b.get(i + 2 + d1..).map_or(0, |s| s.iter().take_while(|c| c.is_ascii_digit()).count())
            } else {
                0
            };
            if d2 > 0 && is_word_boundary_byte(b.get(i + 2 + d1 + d2).copied()) {
                return line.get(i..i + 2 + d1 + d2);
            }
        } else if line.get(i..).is_some_and(|s| s.starts_with("QOL-")) {
            let d = b.get(i + 4..).map_or(0, |s| s.iter().take_while(|c| c.is_ascii_digit()).count());
            if d > 0 && is_word_boundary_byte(b.get(i + 4 + d).copied()) {
                return line.get(i..i + 4 + d);
            }
        }
    }
    None
}

fn compute_guide_readability_stats(raw: &str) -> (f64, f64, usize) {
    let (mut fence, mut words, mut sents, mut code_spans, mut rs_refs) = (None::<(u8, usize)>, 0usize, 0usize, 0usize, 0usize);
    for line in raw.lines() {
        let t = line.trim();
        if let Some((ch, n)) = fence {
            let cnt = t.bytes().take_while(|&b| b == ch).count();
            if cnt >= n && t.get(cnt..).is_some_and(|r| r.trim().is_empty()) {
                fence = None;
            }
            continue;
        }
        if let Some(f) = parse_fence_open(t) {
            fence = Some(f);
            continue;
        }
        let lb = line.as_bytes();
        rs_refs += line
            .match_indices(".rs")
            .filter(|&(d, _)| d > 0 && !is_word_boundary_byte(lb.get(d - 1).copied()) && is_word_boundary_byte(lb.get(d + 3).copied()))
            .count();
        code_spans += lb.iter().filter(|&&b| b == b'`').count() / 2;
        if t.is_empty()
            || t.starts_with(['|', '#'])
            || matches!(t, "---" | "***" | "___" | "$$")
            || t.starts_with("![")
            || t.starts_with("> [!")
            || (t.starts_with("<!--") && t.ends_with("-->"))
        {
            continue;
        }
        let cleaned = t.trim_start_matches('>').trim();
        let w = cleaned.split_whitespace().count();
        if w > 0 {
            words += w;
            let (cb, mut in_code) = (cleaned.as_bytes(), false);
            for (i, &c) in cb.iter().enumerate() {
                if c == b'`' {
                    in_code = !in_code;
                } else if !in_code
                    && matches!(c, b'.' | b'!' | b'?')
                    && !(c == b'.' && cb.get(i + 1).is_some_and(u8::is_ascii_digit))
                    && matches!(cb.get(i + 1), None | Some(b' ' | b'\t' | b')' | b']' | b'"' | b'\'' | b'*'))
                {
                    sents += 1;
                }
            }
        }
    }
    let eff = sents.max(usize::from(words > 0));
    (if eff == 0 { 0.0 } else { words as f64 / eff as f64 }, if words == 0 { 0.0 } else { code_spans as f64 * 100.0 / words as f64 }, rs_refs)
}

#[cfg(test)]
mod tests {
    use super::super::template::{PageSpec, SiteMeta, render_page, render_root_redirect_shim};
    use super::*;
    use crate::scan_util::test_support::TestDir;

    fn test_meta() -> SiteMeta {
        SiteMeta {
            version_label: "v0.1.0-test".into(),
            commit_sha: Some("0123456789abcdef0123456789abcdef01234567".into()),
            release_date: Some("2026-11-18".into()),
            is_release_snapshot: false,
            is_tagged_release: false,
        }
    }
    fn render_test_page(rel_path: &str, title: &str, body_html: &str) -> String {
        let meta = test_meta();
        render_page(&PageSpec {
            title,
            description: title,
            rel_path,
            section_title: "Concepts",
            section_rel_path: None,
            nav_group: Some(super::super::sitemap::NavGroup::Reference),
            active_section_id: Some("concepts"),
            sidebar_groups: &[],
            h1: title,
            toc_headings: &[],
            body_html,
            body_has_h1: false,
            prev: None,
            next: None,
            meta: &meta,
        })
    }

    #[test]
    fn html_csp_uri_obfuscation_and_links() {
        let good = render_test_page("index.html", "Test", "<p>Clean body.</p>");
        assert!(audit_single_html("index.html", &good).is_ok());
        for bad in [
            "<script>alert(1)</script>",
            "<script/src=\"https://evil.example/x.js\"></script>",
            "<form action=\"/x\"></form>",
            "<div style=\"color:red\">x</div>",
            "<p onclick=\"x()\">x</p>",
            "<link rel=\"stylesheet\" href=\"https://evil.example/a.css\">",
            "<img src=\"https://evil.example/a.png\" alt=\"\">",
            "<a href=\"&#x6a;avascript:alert(1)\">x</a>",
            "<a href=\" java\tscript:alert(1)\">x</a>",
            "<meta http-equiv=\"refresh\" content=\"0; url=https://evil.example/\">",
            "<a title=\"unclosed><script>x()</script>",
            "<!-- unclosed <script>x()</script>",
            "<p title=x\"><script>alert(1)</script><b z=1\">",
            "<!--><script>alert(1)</script><!-- -->",
            "<title><!--</title><script>alert(1)</script><!-- -->",
            "<p>Raw \\frac{1}{2}</p>",
            "<p>[!CAUTION] oops</p>",
        ] {
            assert!(audit_single_html("bad.html", &good.replace("<p>Clean body.</p>", bad)).is_err(), "{bad}");
        }
        assert!(audit_single_html("ok.html", &good.replace("<p>Clean body.</p>", "<p><code>\\frac{1}{2}</code> <code>[!NOTE]</code></p>")).is_ok());
        for f in ["ROADMAP.html", "contribute/agents.html"] {
            assert!(
                check_exclusions(&GeneratedSite {
                    html_pages: BTreeMap::from([(f.into(), "<h1>x</h1>".into())]),
                    expected_coverage: Default::default(),
                    meta: test_meta()
                })
                .is_err()
            );
        }
        let (pa, pb) =
            (render_test_page("a.html", "A", "<p><a href=\"b.html#missing\">x</a></p>"), render_test_page("b.html", "B", "<h2 id=\"ok\">Ok</h2>"));
        let facts = BTreeMap::from([
            ("a.html".into(), audit_single_html("a.html", &pa).unwrap()),
            ("b.html".into(), audit_single_html("b.html", &pb).unwrap()),
        ]);
        let mut cache = BTreeMap::new();
        assert!(verify_target_with_cache(Path::new("."), Path::new("."), false, &facts, "a.html", "b.html#missing", &mut cache).is_err());
        assert!(verify_target_with_cache(Path::new("."), Path::new("."), false, &facts, "a.html", "b.html#ok", &mut cache).is_ok());
        assert!(verify_target_with_cache(Path::new("."), Path::new("."), false, &facts, "a.html", "/privacy.html", &mut cache).is_err());
        assert!(verify_target_with_cache(Path::new("."), Path::new("."), false, &facts, "a.html", "#", &mut cache).is_ok());
    }

    #[test]
    fn snapshots_and_readability() {
        let dir = TestDir::new("docs-snap-root");
        let manifest = "{\"version\":\"0.1.0\",\"commit\":\"0123456789abcdef0123456789abcdef01234567\"}\n";
        let minimal_snap = format!(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"description\" content=\"Snap desc\"><meta name=\"referrer\" content=\"no-referrer\"><meta http-equiv=\"Content-Security-Policy\" content=\"{STRICT_CSP}\"><title>Snap</title><link rel=\"stylesheet\" href=\"../../site.css\"></head><body><a class=\"skip-link\" href=\"#main\">Skip</a><main id=\"main\"><h1>Snap</h1><a href=\"#\">Top</a></main></body></html>"
        );
        let redirect_ok = super::super::template::render_release_redirect_index("latest", "v0.1.0", &[String::from("v0.1")]);
        dir.write("docs/site.css", "body{}\n");
        dir.write_bytes("docs/images/curvebook.png", b"png");
        for (rel, body) in [
            ("target/docs-manual/v0.1/manifest.json", manifest),
            ("target/docs-manual/v0.1/index.html", &minimal_snap),
            ("target/docs-manual/latest/manifest.json", manifest),
            ("target/docs-manual/latest/index.html", &minimal_snap),
            ("target/docs-manual/index.html", &redirect_ok),
        ] {
            dir.write(rel, body);
        }
        let manual = dir.join("target/docs-manual");
        assert!(check_no_committed_manual_html(dir.path()).is_ok() && check_manual_snapshots(dir.path(), &manual).is_ok());
        dir.write("target/docs-manual/latest/index.html", &minimal_snap.replace("<h1>Snap</h1>", "<h1>Stale</h1>"));
        assert!(check_manual_snapshots(dir.path(), &manual).unwrap_err().contains("byte-for-byte"));
        dir.write("docs/manual/latest/index.html", "<html></html>");
        assert!(check_no_committed_manual_html(dir.path()).is_err());
        assert!(
            audit_single_html("docs/get-started.html", &render_root_redirect_shim("GS", "GS desc", "manual/v0.1/get-started.html", "v0.1.0")).is_ok()
        );
        assert_eq!(
            (find_forbidden_guide_id("Added in R12.11 and QOL-7."), find_forbidden_guide_id("Fitted $R^2$ with R 4.5.2.")),
            (Some("R12.11"), None)
        );
        assert_eq!(compute_guide_readability_stats("# H\n\nUse `a.rs`.\n```rust\n// b.rs\n```\n| `c.rs` |\n").2, 2);
        let html_ok = "<main><!-- R12.8 ok --><h1>Title</h1><p>Short plain sentence. Another clear line.</p><pre><code>a.rs</code></pre></main>";
        let (w, avg, c100, rs, bad_id) = compute_html_readability_stats(html_ok);
        assert!(w == 6 && avg <= 4.0 && c100 == 0.0 && rs == 0 && bad_id.is_none());
        let mut tier_v = Vec::new();
        enforce_tier1_ceilings("docs/open-pzfx-files.html", Some(1200), 25.0, 12.0, 1, "docs/pzfx-format-specification.html", &mut tier_v);
        check_audience_banner("docs/a.md", "# Title\n\n<!-- Audience: Tier 1 Scientist Guide -->\n", true, &mut tier_v);
        check_audience_banner("docs/b.md", "# Title\n\nNo banner\n", false, &mut tier_v);
        assert!(
            tier_v.len() == 2
                && tier_v[0].contains("Tier 1 scientist guide")
                && tier_v[0].contains("pzfx-format-specification.html")
                && tier_v[1].contains("<!-- Audience: Tier 2")
        );
    }
}
