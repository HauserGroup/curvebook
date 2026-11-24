//! Static HTML, security/CSP, registry coverage, link/fragment, snapshot manifest,
//! and compliance (`brands`, `cleanroom`, `assets`, `secrets`) verification for `docs-site --check`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use super::sitemap::GeneratedSite;
use super::template::STRICT_CSP;

#[derive(Debug, Default)]
struct PageFacts {
    ids: BTreeSet<String>,
    hrefs: Vec<String>,
    asset_srcs: Vec<String>,
    meta_refresh_targets: Vec<String>,
    meta_descriptions: Vec<String>,
}

/// Verify all generated pages, links, fragments, CSP/privacy invariants, registry coverage,
/// snapshot manifests, and security/brand/cleanroom/asset scanners.
pub fn verify_site(root: &Path, out_dir: &Path, site: &GeneratedSite) -> Result<(), String> {
    check_exclusions(site)?;

    let mut facts: BTreeMap<String, PageFacts> = BTreeMap::new();
    for (rel_path, html) in &site.html_pages {
        let page_facts = audit_single_html(rel_path, html)?;
        facts.insert(rel_path.clone(), page_facts);
    }

    check_unique_meta_descriptions(facts.iter().map(|(k, v)| (k.as_str(), v)))?;
    check_internal_links_and_fragments(root, out_dir, site, &facts)?;
    check_registry_coverage(&facts, site)?;
    check_committed_snapshots(root)?;
    check_root_pages(root)?;
    super::seo::check_search_metadata(root)?;
    crate::pages_site::check_publish_set(root)?;
    run_hygiene_scanners(root, out_dir)?;
    Ok(())
}

fn check_unique_meta_descriptions<'a>(pages: impl IntoIterator<Item = (&'a str, &'a PageFacts)>) -> Result<(), String> {
    let mut seen: BTreeMap<String, &str> = BTreeMap::new();
    for (rel, facts) in pages {
        if facts.meta_descriptions.len() != 1 {
            return Err(format!(
                "{rel}: expected exactly one `<meta name=\"description\">` tag, found {}",
                facts.meta_descriptions.len()
            ));
        }
        let desc = facts.meta_descriptions[0].trim();
        if desc.is_empty() {
            return Err(format!("{rel}: `<meta name=\"description\">` content must not be empty"));
        }
        let key = desc.to_ascii_lowercase();
        if let Some(prev) = seen.insert(key, rel) {
            return Err(format!("{rel}: duplicate `<meta name=\"description\">` already used by `{prev}`: \"{desc}\""));
        }
    }
    Ok(())
}

fn check_exclusions(site: &GeneratedSite) -> Result<(), String> {
    for rel in site.html_pages.keys() {
        let lower = rel.to_ascii_lowercase();
        if lower.contains("roadmap")
            || lower.contains("status")
            || lower.starts_with("audit/")
            || lower.starts_with("planning/")
            || lower.starts_with("evidence/")
            || lower.starts_with("history/")
            || lower.starts_with("contribute/")
            || lower.contains("/contribute/")
            || lower.ends_with("-parity.html")
        {
            return Err(format!("excluded internal document `{rel}` must never be published in docs-site output"));
        }
    }
    Ok(())
}

fn audit_single_html(rel_path: &str, html: &str) -> Result<PageFacts, String> {
    let mut facts = PageFacts::default();
    if !html.contains(STRICT_CSP) {
        return Err(format!("{rel_path}: missing strict Content-Security-Policy meta tag"));
    }
    if !html.contains("<meta name=\"referrer\" content=\"no-referrer\">") {
        return Err(format!("{rel_path}: missing `<meta name=\"referrer\" content=\"no-referrer\">`"));
    }
    if !html.contains("<a class=\"skip-link\" href=\"#main\">") {
        return Err(format!("{rel_path}: missing skip link to `#main`"));
    }
    if !html.contains("id=\"main\"") {
        return Err(format!("{rel_path}: missing `<main id=\"main\">` destination"));
    }
    if !html.contains("<h1") {
        return Err(format!("{rel_path}: missing `<h1>` heading"));
    }

    scan_html_tags(rel_path, html, &mut facts)?;
    Ok(facts)
}

fn scan_html_tags(rel_path: &str, html: &str, facts: &mut PageFacts) -> Result<(), String> {
    let mut rest = html;
    let mut inside_code_or_pre = 0usize;

    while let Some((before_lt, after_lt)) = rest.split_once('<') {
        if inside_code_or_pre == 0 {
            check_unrendered_markup(rel_path, before_lt)?;
        }
        if let Some(after_comment) = after_lt.strip_prefix("!--") {
            let Some((_, after_close)) = after_comment.split_once("-->") else {
                return Err(format!("{rel_path}: unclosed HTML comment `<!--`"));
            };
            rest = after_close;
            continue;
        }
        let Some(tag_end_rel) = find_tag_close(after_lt) else {
            return Err(format!("{rel_path}: unclosed HTML tag or attribute quote"));
        };
        let Some((tag_body, after_tag)) = after_lt.split_at_checked(tag_end_rel) else {
            return Err(format!("{rel_path}: unclosed HTML tag or attribute quote"));
        };
        rest = after_tag.strip_prefix('>').unwrap_or(after_tag);

        let trimmed = tag_body.trim();
        let is_closing = trimmed.starts_with('/');
        let name_part = trimmed.trim_start_matches('/').split_whitespace().next().unwrap_or("").trim_end_matches('/');
        let tag_lower = name_part.to_ascii_lowercase();

        if is_closing {
            if matches!(tag_lower.as_str(), "code" | "pre") && inside_code_or_pre > 0 {
                inside_code_or_pre -= 1;
            }
            continue;
        }

        if matches!(tag_lower.as_str(), "script" | "form" | "iframe" | "object" | "embed") {
            return Err(format!("{rel_path}: forbidden HTML element `<{tag_lower}>`"));
        }

        if matches!(tag_lower.as_str(), "code" | "pre") && !trimmed.ends_with('/') {
            inside_code_or_pre += 1;
        }

        let attrs = parse_tag_attributes(trimmed);
        process_tag_attrs(rel_path, &tag_lower, &attrs, facts)?;
    }
    if inside_code_or_pre == 0 {
        check_unrendered_markup(rel_path, rest)?;
    }
    Ok(())
}

fn process_tag_attrs(rel_path: &str, tag_lower: &str, attrs: &[(String, String)], facts: &mut PageFacts) -> Result<(), String> {
    let mut is_meta_refresh = false;
    let mut is_meta_description = false;
    let mut meta_content: Option<&str> = None;
    let mut rel_values = attrs.iter().filter(|(k, _)| k.eq_ignore_ascii_case("rel")).map(|(_, v)| v.trim());
    let is_canonical_link =
        tag_lower == "link" && rel_values.next().is_some_and(|v| v.eq_ignore_ascii_case("canonical")) && rel_values.next().is_none();

    for (k, v) in attrs {
        let key_lower = k.to_ascii_lowercase();
        if key_lower == "style" {
            return Err(format!("{rel_path}: inline `style=` attribute forbidden under strict CSP (`<{tag_lower}>`)"));
        }
        if key_lower.starts_with("on") && key_lower.len() > 2 {
            return Err(format!("{rel_path}: inline event handler `{key_lower}=` forbidden (`<{tag_lower}>`)"));
        }
        if key_lower == "id" && !v.is_empty() && !facts.ids.insert(v.clone()) {
            return Err(format!("{rel_path}: duplicate HTML id `#{v}`"));
        }
        if key_lower == "href" && is_canonical_link {
            // Canonical links name the page's own published URL for search engines; nothing is fetched.
            if !v.starts_with(super::seo::SITE_BASE_URL) {
                return Err(format!("{rel_path}: canonical link `{v}` must point inside `{}`", super::seo::SITE_BASE_URL));
            }
        } else if key_lower == "href" {
            if is_forbidden_uri_scheme(v) {
                return Err(format!("{rel_path}: forbidden `href` URI `{v}`"));
            }
            if matches!(tag_lower, "link" | "base") && is_external_url(v) {
                return Err(format!("{rel_path}: external asset URL forbidden in `<{tag_lower} href=\"{v}\">`"));
            }
            facts.hrefs.push(v.clone());
        } else if matches!(key_lower.as_str(), "src" | "poster" | "action") {
            if is_external_url(v) || is_forbidden_uri_scheme(v) {
                return Err(format!("{rel_path}: external or forbidden `{key_lower}` asset forbidden (`{v}`)"));
            }
            facts.asset_srcs.push(v.clone());
        } else if tag_lower == "meta" && key_lower == "http-equiv" && v.trim().eq_ignore_ascii_case("refresh") {
            is_meta_refresh = true;
        } else if tag_lower == "meta" && key_lower == "name" && v.trim().eq_ignore_ascii_case("description") {
            is_meta_description = true;
        } else if tag_lower == "meta" && key_lower == "content" {
            meta_content = Some(v.as_str());
        }
    }

    if is_meta_description {
        facts.meta_descriptions.push(meta_content.unwrap_or("").trim().to_owned());
    }

    if tag_lower == "img" {
        let has_alt = attrs.iter().any(|(k, _)| k.eq_ignore_ascii_case("alt"));
        if !has_alt {
            return Err(format!("{rel_path}: `<img>` must declare an `alt` attribute (use `alt=\"\"` for decorative images)"));
        }
        if rel_path.starts_with("docs/") {
            let valid_dim = |name: &str| {
                attrs.iter().any(|(k, v)| k.eq_ignore_ascii_case(name) && v.trim().parse::<u32>().is_ok_and(|n| n > 0))
            };
            if !valid_dim("width") || !valid_dim("height") {
                return Err(format!("{rel_path}: root page `<img>` must declare positive integer `width` and `height` attributes"));
            }
        }
    }

    if is_meta_refresh
        && let Some(content) = meta_content
        && let Some(target) = parse_meta_refresh_url(content)
    {
        if is_forbidden_uri_scheme(target) || is_external_url(target) {
            return Err(format!("{rel_path}: external or forbidden `<meta http-equiv=\"refresh\">` target `{target}`"));
        }
        facts.meta_refresh_targets.push(target.to_owned());
    }
    Ok(())
}

fn parse_meta_refresh_url(content: &str) -> Option<&str> {
    for part in content.split(';') {
        let trimmed = part.trim();
        if let Some((lhs, rhs)) = trimmed.split_once('=')
            && lhs.trim().eq_ignore_ascii_case("url")
        {
            let target = rhs.trim().trim_matches('"').trim_matches('\'');
            if !target.is_empty() {
                return Some(target);
            }
        }
    }
    None
}

fn check_unrendered_markup(rel_path: &str, text: &str) -> Result<(), String> {
    const LEAKED_TEX_MACROS: &[&str] = &["\\frac{", "\\dfrac{", "\\tfrac{", "\\text{", "\\sqrt{", "\\chi", "\\Phi", "\\TeX", "\\ge ", "\\le "];
    const LEAKED_CALLOUT_MARKERS: &[&str] = &["[!NOTE]", "[!TIP]", "[!IMPORTANT]", "[!WARNING]", "[!CAUTION]"];

    for marker in LEAKED_TEX_MACROS {
        if text.contains(marker) {
            return Err(format!("{rel_path}: unrendered TeX macro `{marker}` leaked outside <code>/<pre>"));
        }
    }
    for marker in LEAKED_CALLOUT_MARKERS {
        if text.contains(marker) {
            return Err(format!("{rel_path}: unrendered GitHub callout marker `{marker}` leaked outside <code>/<pre>"));
        }
    }
    Ok(())
}

fn find_tag_close(rest: &str) -> Option<usize> {
    let bytes = rest.as_bytes();
    let mut quote: Option<u8> = None;
    for (idx, &b) in bytes.iter().enumerate() {
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

fn parse_tag_attributes(trimmed_tag: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    let bytes = trimmed_tag.as_bytes();
    let mut i = bytes.iter().position(|b| b.is_ascii_whitespace()).unwrap_or(bytes.len());
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
            if !key.is_empty() {
                attrs.push((key, String::new()));
            }
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
            unescape_attr(&raw)
        } else {
            let v_start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            unescape_attr(trimmed_tag.get(v_start..i).unwrap_or(""))
        };
        attrs.push((key, val));
    }
    attrs
}

fn unescape_attr(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let bytes = raw.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'&'
            && let Some((ch, consumed)) = decode_html_entity(raw.get(i..).unwrap_or(""))
        {
            out.push(ch);
            i += consumed;
            continue;
        }
        if let Some(rest) = raw.get(i..)
            && let Some(ch) = rest.chars().next()
        {
            out.push(ch);
            i += ch.len_utf8();
        } else {
            i += 1;
        }
    }
    out
}

fn decode_html_entity(slice: &str) -> Option<(char, usize)> {
    let rest = slice.strip_prefix('&')?;
    if let Some(num) = rest.strip_prefix('#') {
        if let Some(hex_tail) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
            let digits_len = hex_tail.bytes().take_while(u8::is_ascii_hexdigit).count();
            if digits_len == 0 {
                return None;
            }
            let digits = hex_tail.get(..digits_len)?;
            let trimmed_digits = digits.trim_start_matches('0');
            let code = if trimmed_digits.is_empty() {
                0u32
            } else if trimmed_digits.len() > 8 {
                return None;
            } else {
                u32::from_str_radix(trimmed_digits, 16).ok()?
            };
            let ch = char::from_u32(code).unwrap_or('\u{FFFD}');
            let semi = usize::from(hex_tail.as_bytes().get(digits_len) == Some(&b';'));
            return Some((ch, 3 + digits_len + semi));
        }
        let digits_len = num.bytes().take_while(u8::is_ascii_digit).count();
        if digits_len == 0 {
            return None;
        }
        let digits = num.get(..digits_len)?;
        let trimmed_digits = digits.trim_start_matches('0');
        let code = if trimmed_digits.is_empty() {
            0u32
        } else if trimmed_digits.len() > 10 {
            return None;
        } else {
            trimmed_digits.parse::<u32>().ok()?
        };
        let ch = char::from_u32(code).unwrap_or('\u{FFFD}');
        let semi = usize::from(num.as_bytes().get(digits_len) == Some(&b';'));
        return Some((ch, 2 + digits_len + semi));
    }
    let semi_pos = rest.find(';')?;
    if semi_pos == 0 || semi_pos > 16 {
        return None;
    }
    let name = rest.get(..semi_pos)?;
    let ch = match name.to_ascii_lowercase().as_str() {
        "amp" => '&',
        "quot" => '"',
        "apos" => '\'',
        "lt" => '<',
        "gt" => '>',
        "colon" => ':',
        "tab" => '\t',
        "newline" => '\n',
        _ => return None,
    };
    Some((ch, 1 + semi_pos + 1))
}

fn normalize_scheme_prefix(raw: &str) -> String {
    raw.chars().filter(|c| !c.is_ascii_control() && !c.is_ascii_whitespace()).map(|c| c.to_ascii_lowercase()).collect()
}

fn is_forbidden_uri_scheme(raw: &str) -> bool {
    let norm = normalize_scheme_prefix(raw);
    norm.starts_with("javascript:") || norm.starts_with("data:") || norm.starts_with("vbscript:")
}

fn is_external_url(url: &str) -> bool {
    let norm = normalize_scheme_prefix(url);
    norm.starts_with("http://") || norm.starts_with("https://") || norm.starts_with("//")
}

fn is_mailto_url(url: &str) -> bool {
    normalize_scheme_prefix(url).starts_with("mailto:")
}

fn check_internal_links_and_fragments(root: &Path, out_dir: &Path, site: &GeneratedSite, facts: &BTreeMap<String, PageFacts>) -> Result<(), String> {
    let mut extra_html_ids: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (page_rel, page_facts) in facts {
        for raw_href in &page_facts.hrefs {
            if is_external_url(raw_href) || is_mailto_url(raw_href) {
                continue;
            }
            verify_target_with_cache(root, out_dir, site, facts, page_rel, raw_href, &mut extra_html_ids)?;
        }
        for raw_src in &page_facts.asset_srcs {
            verify_target_with_cache(root, out_dir, site, facts, page_rel, raw_src, &mut extra_html_ids)?;
        }
        for raw_refresh in &page_facts.meta_refresh_targets {
            if is_external_url(raw_refresh) || is_forbidden_uri_scheme(raw_refresh) {
                return Err(format!("{page_rel}: external or forbidden `<meta http-equiv=\"refresh\">` target `{raw_refresh}`"));
            }
            verify_target_with_cache(root, out_dir, site, facts, page_rel, raw_refresh, &mut extra_html_ids)?;
        }
    }
    Ok(())
}

#[cfg(test)]
fn verify_target(
    root: &Path,
    out_dir: &Path,
    site: &GeneratedSite,
    facts: &BTreeMap<String, PageFacts>,
    from_rel: &str,
    raw_ref: &str,
) -> Result<(), String> {
    let mut extra_html_ids = BTreeMap::new();
    verify_target_with_cache(root, out_dir, site, facts, from_rel, raw_ref, &mut extra_html_ids)
}

fn verify_target_with_cache(
    root: &Path,
    out_dir: &Path,
    site: &GeneratedSite,
    facts: &BTreeMap<String, PageFacts>,
    from_rel: &str,
    raw_ref: &str,
    extra_html_ids: &mut BTreeMap<String, BTreeSet<String>>,
) -> Result<(), String> {
    let (target_part, frag_part) = match raw_ref.split_once('#') {
        Some((t, f)) => (t, Some(f)),
        None => (raw_ref, None),
    };
    if target_part.is_empty() {
        if let Some(frag) = frag_part {
            let ids = &facts[from_rel].ids;
            if !ids.contains(frag) {
                return Err(format!("{from_rel}: broken same-page anchor `#{frag}`"));
            }
        }
        return Ok(());
    }

    let resolved = resolve_relative_segments(from_rel, target_part);
    if site.meta.is_release_snapshot
        && let Some(docs_rel) = resolved.strip_prefix("../../")
    {
        if docs_rel.is_empty() || docs_rel.starts_with('/') || docs_rel.split('/').any(|seg| seg == "..") {
            return Err(format!("{from_rel}: relative target `{raw_ref}` escapes `docs/` root (`{resolved}`)"));
        }
        let docs_target = root.join("docs").join(docs_rel);
        if !std::fs::symlink_metadata(&docs_target).is_ok_and(|m| !m.file_type().is_symlink() && m.file_type().is_file()) {
            return Err(format!("{from_rel}: broken parent-docs asset/page `{raw_ref}` -> `{}`", docs_target.display()));
        }
        if let Some(frag) = frag_part
            && !frag.is_empty()
            && docs_rel.ends_with(".html")
        {
            let cache_key = format!("docs/{docs_rel}");
            if !extra_html_ids.contains_key(&cache_key) {
                let target_html = std::fs::read_to_string(&docs_target).map_err(|e| format!("read docs/{docs_rel}: {e}"))?;
                let mut target_facts = PageFacts::default();
                scan_html_tags(&cache_key, &target_html, &mut target_facts)?;
                extra_html_ids.insert(cache_key.clone(), target_facts.ids);
            }
            if let Some(ids) = extra_html_ids.get(&cache_key)
                && !ids.contains(frag)
            {
                return Err(format!("{from_rel}: broken parent-docs fragment `{raw_ref}` (missing `#{frag}` in `docs/{docs_rel}`)"));
            }
        }
        return Ok(());
    }
    if resolved.starts_with("..") {
        return Err(format!("{from_rel}: relative target `{raw_ref}` escapes site root (`{resolved}`)"));
    }

    if let Some(target_facts) = facts.get(&resolved) {
        if let Some(frag) = frag_part
            && !frag.is_empty()
            && !target_facts.ids.contains(frag)
        {
            return Err(format!("{from_rel}: broken fragment `{raw_ref}` (missing `#{frag}` in `{resolved}`)"));
        }
        return Ok(());
    }

    let out_target = out_dir.join(&resolved);
    if std::fs::symlink_metadata(&out_target).is_ok_and(|m| !m.file_type().is_symlink() && m.file_type().is_file()) {
        return Ok(());
    }

    Err(format!("{from_rel}: broken internal link `{raw_ref}` (resolved `{resolved}` missing from site output)"))
}

fn resolve_relative_segments(from_rel: &str, target: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(parent) = Path::new(from_rel).parent() {
        for comp in parent.components() {
            if let std::path::Component::Normal(s) = comp
                && let Some(txt) = s.to_str()
                && !txt.is_empty()
            {
                parts.push(txt.to_owned());
            }
        }
    }
    let mut parent_ups = 0usize;
    for seg in target.replace('\\', "/").split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    parent_ups += 1;
                }
            }
            other => parts.push(other.to_owned()),
        }
    }
    let mut out = Vec::with_capacity(parent_ups + parts.len());
    for _ in 0..parent_ups {
        out.push(String::from(".."));
    }
    out.extend(parts);
    out.join("/")
}

fn check_registry_coverage(facts: &BTreeMap<String, PageFacts>, site: &GeneratedSite) -> Result<(), String> {
    let az_ids = facts.get("az-index.html").map(|f| &f.hrefs).ok_or_else(|| String::from("missing `az-index.html`"))?;
    let az_set: BTreeSet<&str> = az_ids.iter().map(String::as_str).collect();

    let exp = &site.expected_coverage;
    assert_ids_present(facts, &az_set, "registry/commands.html", &exp.engine_commands, "engine command")?;
    assert_ids_present(facts, &az_set, "registry/commands.html", &exp.ui_commands, "UI command")?;
    assert_ids_present(facts, &az_set, "registry/mcp-tools.html", &exp.mcp_tools, "MCP tool")?;
    assert_ids_present(facts, &az_set, "registry/mcp-tools.html", &exp.mcp_prompts, "MCP prompt")?;
    assert_ids_present(facts, &az_set, "registry/cli.html", &exp.cli_commands, "CLI subcommand")?;
    assert_analysis_kinds_covered(facts, &az_set, &exp.analysis_kinds)?;
    assert_ids_present(facts, &az_set, "registry/equations.html", &exp.fit_equations, "fit equation")?;
    assert_ids_present(facts, &az_set, "registry/graphs.html", &exp.graph_kinds, "graph kind")?;
    assert_ids_present(facts, &az_set, "registry/graphs.html", &exp.graph_options, "graph option")?;
    assert_ids_present(facts, &az_set, "registry/tables.html", &exp.table_kinds, "table kind")?;
    assert_ids_present(facts, &az_set, "registry/formats.html", &exp.formats, "format descriptor")?;
    assert_ids_present(facts, &az_set, "registry/formats.html", &exp.preference_keys, "preference schema key")?;

    let mcp_html = site.html_pages.get("registry/mcp-tools.html").ok_or_else(|| String::from("missing `registry/mcp-tools.html`"))?;
    for scheme in ["sheet://", "result://"] {
        if !mcp_html.contains(scheme) {
            return Err(format!("`registry/mcp-tools.html` missing MCP resource scheme `{scheme}`"));
        }
    }
    Ok(())
}

fn assert_ids_present(
    facts: &BTreeMap<String, PageFacts>,
    az_hrefs: &BTreeSet<&str>,
    page_rel: &str,
    expected_ids: &[String],
    label: &str,
) -> Result<(), String> {
    let page = facts.get(page_rel).ok_or_else(|| format!("missing generated page `{page_rel}`"))?;
    for id in expected_ids {
        if !page.ids.contains(id) {
            return Err(format!("missing {label} anchor `#{id}` in `{page_rel}`"));
        }
        let target_href = format!("{page_rel}#{id}");
        if !az_hrefs.contains(target_href.as_str()) {
            return Err(format!("missing {label} entry `{target_href}` in `az-index.html`"));
        }
    }
    Ok(())
}

fn assert_analysis_kinds_covered(facts: &BTreeMap<String, PageFacts>, az_hrefs: &BTreeSet<&str>, analysis_slugs: &[String]) -> Result<(), String> {
    let tables_page = facts.get("registry/tables.html").ok_or_else(|| String::from("missing generated page `registry/tables.html`"))?;
    for slug in analysis_slugs {
        let reg_id = format!("analysis-kind-{slug}");
        if !tables_page.ids.contains(&reg_id) {
            return Err(format!("missing analysis kind anchor `#{reg_id}` in `registry/tables.html`"));
        }
        let doc_id = format!("analysis-{slug}");
        let doc_href = format!("analyses.html#{doc_id}");
        let reg_href = format!("registry/tables.html#{reg_id}");
        if az_hrefs.contains(doc_href.as_str()) {
            let analyses_page = facts.get("analyses.html").ok_or_else(|| String::from("missing `analyses.html` referenced by `az-index.html`"))?;
            if !analyses_page.ids.contains(&doc_id) {
                return Err(format!("missing analysis kind anchor `#{doc_id}` in `analyses.html`"));
            }
        } else if !az_hrefs.contains(reg_href.as_str()) {
            return Err(format!("missing analysis kind entry `{reg_href}` (or `{doc_href}`) in `az-index.html`"));
        }
    }
    Ok(())
}

/// Parse `(major, minor)` from `"v0.1"` or `"0.1.0-alpha.2"` snapshot directory names.
pub fn parse_minor_dir_version(name: &str) -> Option<(u64, u64)> {
    let core = name.strip_prefix('v')?;
    let (maj, min) = core.split_once('.')?;
    Some((maj.parse().ok()?, min.parse().ok()?))
}

/// Check ALL committed release snapshots under `docs/manual/v*/manifest.json`.
pub fn check_committed_snapshots(root: &Path) -> Result<(), String> {
    let manual_dir = root.join("docs/manual");
    if !std::fs::symlink_metadata(&manual_dir).is_ok_and(|m| !m.file_type().is_symlink() && m.file_type().is_dir()) {
        return Ok(());
    }
    let entries = std::fs::read_dir(&manual_dir).map_err(|e| format!("read_dir {}: {e}", manual_dir.display()))?;
    let mut snapshots: Vec<((u64, u64), String)> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry under docs/manual: {e}"))?;
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_symlink() || !ft.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(ver) = parse_minor_dir_version(&name)
            && entry.path().join("manifest.json").is_file()
        {
            snapshots.push((ver, name));
        }
    }
    snapshots.sort_by_key(|(v, _)| *v);
    let Some((_, newest_dir)) = snapshots.last() else {
        return Ok(());
    };

    for (_, snap_dir) in &snapshots {
        let manifest_path = manual_dir.join(snap_dir).join("manifest.json");
        let raw = std::fs::read_to_string(&manifest_path).map_err(|e| format!("read {}: {e}", manifest_path.display()))?;
        let manifest: serde_json::Value = serde_json::from_str(&raw).map_err(|e| format!("parse {}: {e}", manifest_path.display()))?;

        let version =
            manifest.get("version").and_then(serde_json::Value::as_str).ok_or_else(|| format!("{}: missing `version`", manifest_path.display()))?;
        let commit =
            manifest.get("commit").and_then(serde_json::Value::as_str).ok_or_else(|| format!("{}: missing `commit`", manifest_path.display()))?;
        if commit.len() != 40 || !commit.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(format!("{}: `commit` `{commit}` must be a 40-hex SHA", manifest_path.display()));
        }

        // If a release tag for `version` already exists in git, verify the manifest commit matches the tag commit.
        for tag_candidate in [format!("v{version}"), version.to_owned()] {
            if let Some(tag_sha) = resolve_git_tag_commit(root, &tag_candidate)?
                && tag_sha != commit
            {
                return Err(format!(
                    "{}: manifest commit `{commit}` does not match release tag `{tag_candidate}` commit `{tag_sha}`",
                    manifest_path.display()
                ));
            }
        }
    }

    let latest_manifest_path = manual_dir.join("latest/manifest.json");
    let expected_refresh = if std::fs::symlink_metadata(&latest_manifest_path).is_ok_and(|m| !m.file_type().is_symlink() && m.file_type().is_file()) {
        let latest_raw = std::fs::read_to_string(&latest_manifest_path).map_err(|e| format!("read {}: {e}", latest_manifest_path.display()))?;
        let newest_manifest_path = manual_dir.join(newest_dir).join("manifest.json");
        let newest_raw = std::fs::read_to_string(&newest_manifest_path).map_err(|e| format!("read {}: {e}", newest_manifest_path.display()))?;
        if latest_raw != newest_raw {
            return Err(format!("docs/manual/latest/manifest.json does not match newest snapshot docs/manual/{newest_dir}/manifest.json"));
        }
        String::from("latest/index.html")
    } else {
        format!("{newest_dir}/index.html")
    };

    let index_html = std::fs::read_to_string(manual_dir.join("index.html")).map_err(|e| format!("read docs/manual/index.html: {e}"))?;
    if !index_html.contains("http-equiv=\"refresh\"") || !index_html.contains(&expected_refresh) {
        return Err(format!("docs/manual/index.html must redirect via meta-refresh to `{expected_refresh}`"));
    }
    Ok(())
}

/// Audit root GitHub Pages HTML files (`docs/index.html`, `docs/privacy.html`, `docs/manual/index.html`, the other
/// hand-written `seo::LANDING_PAGES`,
/// and any existing root redirect shims `docs/get-started.html`, `docs/troubleshooting.html`) for
/// strict CSP, `no-referrer`, accessibility structure, zero scripts/forms/inline styles/external
/// assets, and valid local relative targets under `docs/`.
pub fn check_root_pages(root: &Path) -> Result<(), String> {
    let docs_dir = root.join("docs");
    let required = ["index.html", "privacy.html", "manual/index.html"];
    let mut pages_to_audit: Vec<&str> = required.to_vec();
    let optional = super::seo::LANDING_PAGES
        .iter()
        .copied()
        .filter(|p| !required.contains(p))
        .chain(["404.html", "get-started.html", "troubleshooting.html"]);
    for shim in optional {
        let path = docs_dir.join(shim);
        if std::fs::symlink_metadata(&path).is_ok_and(|m| !m.file_type().is_symlink() && m.file_type().is_file()) {
            pages_to_audit.push(shim);
        }
    }

    let mut audited: BTreeMap<String, PageFacts> = BTreeMap::new();
    for rel in pages_to_audit {
        let full = docs_dir.join(rel);
        let html = std::fs::read_to_string(&full).map_err(|e| format!("read docs/{rel}: {e}"))?;
        let facts = audit_single_html(&format!("docs/{rel}"), &html)?;
        audited.insert(rel.to_owned(), facts);
    }

    check_unique_meta_descriptions(audited.iter().map(|(k, v)| (k.as_str(), v)))?;

    let mut extra_html_ids: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (rel, facts) in &audited {
        for raw_href in &facts.hrefs {
            if is_external_url(raw_href) || is_mailto_url(raw_href) {
                continue;
            }
            verify_docs_relative_target(&docs_dir, rel, raw_href, facts, &audited, &mut extra_html_ids)?;
        }
        for raw_src in &facts.asset_srcs {
            if is_external_url(raw_src) || is_forbidden_uri_scheme(raw_src) {
                return Err(format!("docs/{rel}: external or forbidden asset URL `{raw_src}`"));
            }
            verify_docs_relative_target(&docs_dir, rel, raw_src, facts, &audited, &mut extra_html_ids)?;
        }
        for raw_refresh in &facts.meta_refresh_targets {
            if is_external_url(raw_refresh) || is_forbidden_uri_scheme(raw_refresh) {
                return Err(format!("docs/{rel}: external or forbidden `<meta http-equiv=\"refresh\">` target `{raw_refresh}`"));
            }
            verify_docs_relative_target(&docs_dir, rel, raw_refresh, facts, &audited, &mut extra_html_ids)?;
        }
    }
    Ok(())
}

fn verify_docs_relative_target(
    docs_dir: &Path,
    from_rel: &str,
    raw_ref: &str,
    from_facts: &PageFacts,
    audited: &BTreeMap<String, PageFacts>,
    extra_html_ids: &mut BTreeMap<String, BTreeSet<String>>,
) -> Result<(), String> {
    let (target_part, frag_part) = match raw_ref.split_once('#') {
        Some((t, f)) => (t, Some(f)),
        None => (raw_ref, None),
    };
    if target_part.is_empty() {
        if let Some(frag) = frag_part
            && !frag.is_empty()
            && !from_facts.ids.contains(frag)
        {
            return Err(format!("docs/{from_rel}: broken same-page anchor `#{frag}`"));
        }
        return Ok(());
    }

    let target_part = if from_rel == "404.html" && target_part.starts_with('/') {
        let base_path = crate::pages_site::site_base_path();
        let Some(stripped) = target_part.strip_prefix(base_path) else {
            return Err(format!("docs/404.html: root-absolute target `{raw_ref}` must start with `{base_path}`"));
        };
        if stripped.is_empty() { "index.html" } else { stripped }
    } else {
        target_part
    };

    let resolved = resolve_relative_segments(from_rel, target_part);
    if resolved.starts_with("..") {
        return Err(format!("docs/{from_rel}: relative target `{raw_ref}` escapes `docs/` root (`{resolved}`)"));
    }
    let target_path = docs_dir.join(&resolved);
    if !std::fs::symlink_metadata(&target_path).is_ok_and(|m| !m.file_type().is_symlink() && m.file_type().is_file()) {
        return Err(format!("docs/{from_rel}: broken relative target `{raw_ref}` (missing `docs/{resolved}`)"));
    }

    if let Some(frag) = frag_part
        && !frag.is_empty()
        && resolved.ends_with(".html")
    {
        if let Some(known) = audited.get(&resolved) {
            if !known.ids.contains(frag) {
                return Err(format!("docs/{from_rel}: broken fragment `{raw_ref}` (missing `#{frag}` in `docs/{resolved}`)"));
            }
            return Ok(());
        }
        if !extra_html_ids.contains_key(&resolved) {
            let target_html = std::fs::read_to_string(&target_path).map_err(|e| format!("read docs/{resolved}: {e}"))?;
            let mut target_facts = PageFacts::default();
            scan_html_tags(&format!("docs/{resolved}"), &target_html, &mut target_facts)?;
            extra_html_ids.insert(resolved.clone(), target_facts.ids);
        }
        if let Some(ids) = extra_html_ids.get(&resolved)
            && !ids.contains(frag)
        {
            return Err(format!("docs/{from_rel}: broken fragment `{raw_ref}` (missing `#{frag}` in `docs/{resolved}`)"));
        }
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
    if !out.success {
        return Ok(None);
    }
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if sha.is_empty() { Ok(None) } else { Ok(Some(sha)) }
}

fn run_hygiene_scanners(root: &Path, out_dir: &Path) -> Result<(), String> {
    let brand_hits = crate::brands::check_docs_site_dir(out_dir)?;
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

    let catalog = crate::assets::load_repo_asset_catalog(root)?;
    let (_, asset_violations) = crate::assets::scan_dist_artefacts(&catalog, out_dir)?;
    if let Some(first) = asset_violations.first() {
        return Err(format!("assets scanner found {} violation(s) in generated docs-site (first: {first})", asset_violations.len()));
    }

    let mut secret_hits = Vec::new();
    scan_dir_secrets(out_dir, out_dir, &mut secret_hits)?;
    if let Some(first) = secret_hits.first() {
        return Err(format!("secrets scanner found {} violation(s) in generated docs-site (first: {first})", secret_hits.len()));
    }
    Ok(())
}

fn scan_dir_secrets(base: &Path, dir: &Path, hits: &mut Vec<crate::secrets::SecretHit>) -> Result<(), String> {
    let mut paths: Vec<_> =
        std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?.filter_map(Result::ok).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.file_type().is_dir() {
            scan_dir_secrets(base, &path, hits)?;
        } else if meta.file_type().is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(ext, "html" | "htm" | "css" | "json") {
                let text = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
                let rel = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                crate::secrets::scan_text(&rel, &text, hits);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::markdown::{IdentityResolver, TocHeading, render_markdown, render_math_html};
    use super::super::template::{PageSpec, SiteMeta, render_page, render_root_redirect_shim};
    use super::*;

    fn test_meta() -> SiteMeta {
        SiteMeta {
            version_label: String::from("v0.1.0-test"),
            commit_sha: Some(String::from("0123456789abcdef0123456789abcdef01234567")),
            release_date: Some(String::from("2026-11-18")),
            is_release_snapshot: false,
        }
    }

    #[test]
    fn markdown_supports_custom_heading_ids_and_preserves_snake_case() {
        let md = "# Analyses\n\n## Two-Group Comparisons {#analysis-two-group-comparison}\n\nUse `sheet_create_data` or param_alpha_beta.\n";
        let rendered = render_markdown(md, &IdentityResolver);
        assert_eq!(rendered.title.as_deref(), Some("Analyses"));
        assert!(rendered.html.contains("id=\"analysis-two-group-comparison\""));
        assert!(!rendered.html.contains("<em>alpha</em>"), "snake_case must not become <em>: {}", rendered.html);
    }

    #[test]
    fn audit_html_rejects_scripts_forms_inline_styles_and_external_assets() {
        let meta = test_meta();
        let good = render_page(&PageSpec {
            title: "Test",
            description: "Desc",
            rel_path: "index.html",
            section_title: "Get started",
            section_rel_path: None,
            h1: "Test",
            toc_headings: &[],
            body_html: "<p>Clean body.</p>",
            body_has_h1: false,
            prev: None,
            next: None,
            meta: &meta,
        });
        assert!(audit_single_html("index.html", &good).is_ok());

        for bad_snippet in [
            "<script>alert(1)</script>",
            "<form action=\"/x\"></form>",
            "<div style=\"color:red\">x</div>",
            "<p onclick=\"x()\">x</p>",
            "<link rel=\"stylesheet\" href=\"https://evil.example/a.css\">",
            "<img src=\"https://evil.example/a.png\" alt=\"\">",
        ] {
            let poisoned = good.replace("<p>Clean body.</p>", bad_snippet);
            let err = audit_single_html("bad.html", &poisoned).unwrap_err();
            assert!(!err.is_empty());
        }
    }

    #[test]
    fn check_exclusions_and_fragment_verification_catch_failures() {
        for forbidden in ["ROADMAP.html", "contribute/agents.html"] {
            let mut pages = BTreeMap::new();
            pages.insert(String::from(forbidden), String::from("<h1>bad</h1>"));
            let site = GeneratedSite { html_pages: pages, expected_coverage: Default::default(), meta: test_meta() };
            assert!(check_exclusions(&site).unwrap_err().contains("excluded"));
        }

        let meta = test_meta();
        let page_a = render_page(&PageSpec {
            title: "A",
            description: "A",
            rel_path: "a.html",
            section_title: "Concepts",
            section_rel_path: None,
            h1: "A",
            toc_headings: &[],
            body_html: "<p><a href=\"b.html#missing-frag\">broken</a></p>",
            body_has_h1: false,
            prev: None,
            next: None,
            meta: &meta,
        });
        let page_b = render_page(&PageSpec {
            title: "B",
            description: "B",
            rel_path: "b.html",
            section_title: "Concepts",
            section_rel_path: None,
            h1: "B",
            toc_headings: &[],
            body_html: "<h2 id=\"real-frag\">Real</h2>",
            body_has_h1: false,
            prev: None,
            next: None,
            meta: &meta,
        });
        let mut facts = BTreeMap::new();
        facts.insert(String::from("a.html"), audit_single_html("a.html", &page_a).unwrap());
        facts.insert(String::from("b.html"), audit_single_html("b.html", &page_b).unwrap());
        let mut html_pages = BTreeMap::new();
        html_pages.insert(String::from("a.html"), page_a);
        html_pages.insert(String::from("b.html"), page_b);
        let site2 = GeneratedSite { html_pages, expected_coverage: Default::default(), meta };
        let err = verify_target(Path::new("."), Path::new("."), &site2, &facts, "a.html", "b.html#missing-frag").unwrap_err();
        assert!(err.contains("missing `#missing-frag`"), "unexpected err: {err}");
        assert!(verify_target(Path::new("."), Path::new("."), &site2, &facts, "a.html", "b.html#real-frag").is_ok());
        assert!(verify_target(Path::new("."), Path::new("."), &site2, &facts, "a.html", "missing.html").is_err());
    }

    #[test]
    fn fa_sec23_08_unclosed_html_quote_or_comment_fails_and_all_snapshots_checked() {
        let meta = test_meta();
        let base = render_page(&PageSpec {
            title: "T",
            description: "T",
            rel_path: "t.html",
            section_title: "Concepts",
            section_rel_path: None,
            h1: "T",
            toc_headings: &[],
            body_html: "<p>Body</p>",
            body_has_h1: false,
            prev: None,
            next: None,
            meta: &meta,
        });

        let unclosed_quote = base.replace("<p>Body</p>", "<a title=\"unclosed><script>evil()</script>");
        assert!(audit_single_html("t.html", &unclosed_quote).is_err());

        let unclosed_comment = base.replace("<p>Body</p>", "<!-- unclosed <script>evil()</script>");
        assert!(audit_single_html("t.html", &unclosed_comment).is_err());

        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("cb-docs-snapshots-{}-{nanos}", std::process::id()));
        let manual = dir.join("docs/manual");
        std::fs::create_dir_all(manual.join("v0.1")).unwrap();
        std::fs::create_dir_all(manual.join("v0.2")).unwrap();
        std::fs::write(manual.join("v0.1/manifest.json"), "{\"version\":\"0.1.0\",\"commit\":\"badsha\"}\n").unwrap();
        std::fs::write(manual.join("v0.2/manifest.json"), "{\"version\":\"0.2.0\",\"commit\":\"0123456789abcdef0123456789abcdef01234567\"}\n")
            .unwrap();
        std::fs::write(manual.join("index.html"), "<meta http-equiv=\"refresh\" content=\"0; url=v0.2/index.html\">").unwrap();

        let err = check_committed_snapshots(&dir).unwrap_err();
        assert!(err.contains("v0.1"), "expected older corrupted v0.1 snapshot to fail, got {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn static_math_renderer_formats_tex_and_preserves_currency_shell_vars_and_code() {
        let md = "\
Inline $\\text{AIC}_c = N\\ln(\\text{SS}/N) + \\frac{2K_{\\text{eff}}(K_{\\text{eff}}+1)}{N - K_{\\text{eff}} - 1}$ and $\\sqrt{x}$, $\\bar{d} \\pm \\hat{\\varepsilon} + \\tilde{\\varepsilon}$.\n\n\
Greek & ops: $\\alpha \\beta \\gamma \\varepsilon \\chi \\Phi \\kappa \\pi \\tau \\theta \\Delta \\Omega \\ge 5.0\\text{ pt} \\le 10 \\times 2 \\cdot 3 \\in \\ne \\infty \\propto \\approx \\arcsin\\exp\\log\\ln \\TeX \\,\\! \\left(x\\right)$.\n\n\
Underscores in math: $K_{\\text{eff}} + x_{i} - y_{j}^{2}$.\n\n\
Currency: \\$100 and \\$50, $100, $50, between $50 and $100, $50-$100, $100/$50.\n\n\
Shell vars: $HOME, $XDG_CONFIG_HOME, $TMPDIR, $PATH, $HOME/$PATH, $row and $index, $row/$index, `$row`, `$HOME`, `$\\frac{a}{b}`.\n\n\
$$\n\\chi^2 = \\frac{(O - E)^2}{E}\n$$\n";

        let rendered = render_markdown(md, &IdentityResolver);
        let html = &rendered.html;

        assert!(html.contains("<span class=\"math\"><span class=\"math-text\">AIC</span><sub>c</sub>"), "AICc missing: {html}");
        assert!(html.contains("<span class=\"math-frac\"><span class=\"math-num\">"), "fraction missing: {html}");
        assert!(html.contains("√<span class=\"math-rad\">x</span>"), "sqrt missing: {html}");
        assert!(html.contains("d\u{0304}") && html.contains("ε\u{0302}") && html.contains("ε\u{0303}"), "accents missing: {html}");
        assert!(
            html.contains("α")
                && html.contains("χ")
                && html.contains("Φ")
                && html.contains("Ω")
                && html.contains("≥")
                && html.contains("≤")
                && html.contains("±")
                && html.contains("×")
                && html.contains("·")
                && html.contains("∈")
                && html.contains("≠")
                && html.contains("∞")
                && html.contains("∝")
                && html.contains("≈")
                && html.contains("<span class=\"math-text\">arcsin</span>")
                && html.contains("<span class=\"math-text\">TeX</span>"),
            "symbols/functions missing: {html}"
        );
        assert!(!html.contains("<em>"), "underscores inside math must never produce <em>: {html}");
        assert!(html.contains("<div class=\"math-block\"><span class=\"math\">χ<sup>2</sup>"), "block math missing: {html}");

        // Currency, shell vars, and code spans must survive verbatim without becoming math spans.
        for literal in [
            "$100",
            "$50",
            "between $50 and $100",
            "$50-$100",
            "$100/$50",
            "$HOME",
            "$XDG_CONFIG_HOME",
            "$TMPDIR",
            "$PATH",
            "$HOME/$PATH",
            "$row and $index",
            "$row/$index",
            "<code>$row</code>",
            "<code>$HOME</code>",
            "<code>$\\frac{a}{b}</code>",
        ] {
            assert!(html.contains(literal), "expected `{literal}` preserved verbatim in: {html}");
        }

        let standalone = render_math_html("\\frac{\\text{Top} - \\text{Bottom}}{1 + 10^{(\\log\\text{EC}_{50} - X) \\cdot \\text{HillSlope}}}");
        assert!(standalone.starts_with("<span class=\"math\">"));
        assert!(standalone.contains("<span class=\"math-frac\">"));
        assert!(!standalone.contains("\\frac"));
        assert!(!standalone.contains("\\text"));
    }

    #[test]
    fn markdown_callouts_figures_toc_and_audit_gates_work() {
        let md = "\
> [!NOTE]\n> First callout body.\n\n\
> [!WARNING]\n> Watch out for $\\chi^2$.\n\n\
![Workspace screenshot](images/workspace-split-fit.png)\n\n\
> Regular blockquote.\n";

        let rendered = render_markdown(md, &IdentityResolver);
        assert!(rendered.html.contains("<div class=\"callout callout-note\" role=\"note\">\n<p class=\"callout-title\">Note</p>"));
        assert!(rendered.html.contains("<div class=\"callout callout-warning\" role=\"note\">\n<p class=\"callout-title\">Warning</p>"));
        assert!(rendered.html.contains("<blockquote>\n<p>Regular blockquote.</p>\n</blockquote>"));
        assert!(rendered.html.contains(
            "<figure class=\"doc-figure\"><img src=\"images/workspace-split-fit.png\" alt=\"Workspace screenshot\" loading=\"lazy\"><figcaption>Workspace screenshot</figcaption></figure>"
        ));
        assert!(!rendered.html.contains("[!NOTE]") && !rendered.html.contains("[!WARNING]"));

        // Long TOC (> 8 headings) wraps in <details open class="toc">; short TOC uses <nav class="toc">.
        let meta = test_meta();
        let long_headings: Vec<TocHeading> = (1..=9).map(|n| TocHeading { level: 2, id: format!("h-{n}"), text: format!("Heading {n}") }).collect();
        let long_page = render_page(&PageSpec {
            title: "Long",
            description: "Long",
            rel_path: "analyses.html",
            section_title: "Analyses",
            section_rel_path: Some("section-analyses.html"),
            h1: "Long",
            toc_headings: &long_headings,
            body_html: "<p>Body.</p>",
            body_has_h1: false,
            prev: None,
            next: None,
            meta: &meta,
        });
        assert!(long_page.contains("<details open class=\"toc\">\n          <summary class=\"toc-title\">On this page</summary>"));
        assert!(long_page.contains("href=\"section-analyses.html\" aria-current=\"page\">Reference</a>"));
        assert!(audit_single_html("analyses.html", &long_page).is_ok());

        // Leaked TeX or raw [!NOTE] outside <code> must fail audit_single_html, while inside <code> is allowed.
        let leaked_tex = long_page.replace("<p>Body.</p>", "<p>Raw \\frac{1}{2}</p>");
        assert!(audit_single_html("analyses.html", &leaked_tex).unwrap_err().contains("unrendered TeX macro"));
        let leaked_callout = long_page.replace("<p>Body.</p>", "<p>[!CAUTION] oops</p>");
        assert!(audit_single_html("analyses.html", &leaked_callout).unwrap_err().contains("callout marker"));
        let code_tex = long_page.replace("<p>Body.</p>", "<p><code>\\frac{1}{2}</code> and <code>[!NOTE]</code></p>");
        assert!(audit_single_html("analyses.html", &code_tex).is_ok());

        // Root redirect shim passes audit_single_html and check_root_pages.
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let root_dir = std::env::temp_dir().join(format!("cb-docs-root-pages-{}-{nanos}", std::process::id()));
        let docs = root_dir.join("docs");
        std::fs::create_dir_all(docs.join("images")).unwrap();
        std::fs::create_dir_all(docs.join("manual/v0.1")).unwrap();
        std::fs::write(docs.join("site.css"), "body {}\n").unwrap();
        std::fs::write(docs.join("images/curvebook.png"), b"png").unwrap();

        let shim_gs = render_root_redirect_shim("Get Started", "Get started redirect", "manual/v0.1/get-started.html", "v0.1.0");
        let shim_ts = render_root_redirect_shim("Troubleshooting", "Troubleshooting redirect", "manual/v0.1/troubleshooting.html#known-issues", "v0.1.0");
        assert!(audit_single_html("docs/get-started.html", &shim_gs).is_ok());
        assert!(audit_single_html("docs/troubleshooting.html", &shim_ts).is_ok());

        let base_root_html = |title: &str, nav: &str| {
            format!(
                "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"description\" content=\"{title} page description\"><meta name=\"referrer\" content=\"no-referrer\"><meta http-equiv=\"Content-Security-Policy\" content=\"{STRICT_CSP}\"><title>{title}</title><link rel=\"icon\" href=\"images/curvebook.png\"><link rel=\"stylesheet\" href=\"site.css\"></head><body><a class=\"skip-link\" href=\"#main\">Skip</a><main id=\"main\"><h1>{title}</h1>{nav}</main></body></html>"
            )
        };
        std::fs::write(docs.join("index.html"), base_root_html("Home", "<a href=\"privacy.html\">P</a><a href=\"manual/index.html\">M</a>")).unwrap();
        std::fs::write(docs.join("privacy.html"), base_root_html("Privacy", "<a href=\"index.html\">H</a>")).unwrap();
        std::fs::write(
            docs.join("manual/index.html"),
            format!(
                "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"description\" content=\"Manual redirect description\"><meta name=\"referrer\" content=\"no-referrer\"><meta http-equiv=\"Content-Security-Policy\" content=\"{STRICT_CSP}\"><meta http-equiv=\"refresh\" content=\"0; url=v0.1/index.html\"><title>Manual</title><link rel=\"stylesheet\" href=\"../site.css\"></head><body><a class=\"skip-link\" href=\"#main\">Skip</a><main id=\"main\"><h1>Manual</h1><a href=\"v0.1/index.html\">v0.1</a></main></body></html>"
            ),
        )
        .unwrap();
        std::fs::write(docs.join("manual/v0.1/index.html"), "<main id=\"main\"><h1>Manual</h1></main>").unwrap();
        std::fs::write(docs.join("manual/v0.1/get-started.html"), "<main id=\"main\"><h1>GS</h1></main>").unwrap();
        std::fs::write(docs.join("manual/v0.1/troubleshooting.html"), "<main id=\"main\"><h1>TS</h1><h2 id=\"known-issues\">KI</h2></main>").unwrap();
        std::fs::write(docs.join("get-started.html"), &shim_gs).unwrap();
        std::fs::write(docs.join("troubleshooting.html"), &shim_ts).unwrap();

        assert!(check_root_pages(&root_dir).is_ok());
        // Duplicate meta description across root pages must fail check_root_pages.
        let dup_privacy = base_root_html("Privacy", "<a href=\"index.html\">H</a>").replace("Privacy page description", "Home page description");
        std::fs::write(docs.join("privacy.html"), dup_privacy).unwrap();
        assert!(check_root_pages(&root_dir).unwrap_err().contains("duplicate `<meta name=\"description\">`"));
        std::fs::write(docs.join("privacy.html"), base_root_html("Privacy", "<a href=\"index.html\">H</a>")).unwrap();
        // Breaking a local link inside docs/index.html must fail check_root_pages.
        std::fs::write(docs.join("index.html"), base_root_html("Home", "<a href=\"missing.html\">Broken</a>")).unwrap();
        assert!(check_root_pages(&root_dir).unwrap_err().contains("missing.html"));
        let _ = std::fs::remove_dir_all(&root_dir);
    }

    #[test]
    fn da_03_04_empty_multiline_display_math_terminates_cleanly() {
        let rendered1 = render_markdown("$$\n$$\n", &IdentityResolver);
        assert!(rendered1.html.contains("$$"));

        let rendered2 = render_markdown("$$\n   \n$$\nAfter\n", &IdentityResolver);
        assert!(rendered2.html.contains("After"), "unexpected html: {}", rendered2.html);
    }

    #[test]
    fn da_03_05_math_and_markdown_recursion_caps_complexity_and_blockquote_ids() {
        // 1. Exponential O(2^(n/2)) hang regression on repeated incomplete \frac / \binom.
        let frac_bomb = format!("${}$", "\\frac".repeat(80));
        let rendered_frac = render_markdown(&frac_bomb, &IdentityResolver);
        assert!(!rendered_frac.html.is_empty());

        let binom_bomb = "\\binom".repeat(80);
        let rendered_binom = render_math_html(&binom_bomb);
        assert!(!rendered_binom.is_empty());

        // 2. Unbounded stack recursion in brace-free TeX macros & Markdown blocks/inlines.
        let deep_bar = format!("{}x", "\\bar".repeat(500));
        assert!(!render_math_html(&deep_bar).is_empty());
        let deep_sqrt = format!("{}x", "\\sqrt".repeat(500));
        assert!(!render_math_html(&deep_sqrt).is_empty());

        let deep_quote = format!("{}hi\n", "> ".repeat(200));
        assert!(render_markdown(&deep_quote, &IdentityResolver).html.contains("hi"));

        let mut deep_list = String::new();
        for depth in 0..200 {
            deep_list.push_str(&"  ".repeat(depth));
            deep_list.push_str("- item\n");
        }
        assert!(render_markdown(&deep_list, &IdentityResolver).html.contains("item"));

        let deep_inline = format!("{}x{}", "*".repeat(200), "*".repeat(200));
        assert!(render_markdown(&deep_inline, &IdentityResolver).html.contains('x'));

        let deep_links = format!("{}x{}", "[".repeat(200), "](u)".repeat(200));
        assert!(render_markdown(&deep_links, &IdentityResolver).html.contains('x'));

        // 3. Unclosed delimiter + `$` rescan complexity stays fast.
        let unclosed_rescan = "*a ${b ".repeat(500);
        assert!(!render_markdown(&unclosed_rescan, &IdentityResolver).html.is_empty());

        // 4. Headings inside blockquotes/callouts share parent RenderCtx slug_counts so IDs stay unique.
        let dup_md = "# Intro\n\n> # Intro\n\n> [!NOTE]\n> ## Intro\n";
        let rendered_dup = render_markdown(dup_md, &IdentityResolver);
        assert!(rendered_dup.html.contains("id=\"intro\""), "{}", rendered_dup.html);
        assert!(rendered_dup.html.contains("id=\"intro-1\""), "{}", rendered_dup.html);
        assert!(rendered_dup.html.contains("id=\"intro-2\""), "{}", rendered_dup.html);
        let meta = test_meta();
        let page = render_page(&PageSpec {
            title: "Dup",
            description: "Dup",
            rel_path: "dup.html",
            section_title: "Concepts",
            section_rel_path: None,
            h1: "Dup",
            toc_headings: &[],
            body_html: &rendered_dup.html,
            body_has_h1: true,
            prev: None,
            next: None,
            meta: &meta,
        });
        assert!(audit_single_html("dup.html", &page).is_ok());
    }

    #[test]
    fn r12_8_canonical_link_allows_only_the_published_site_url() {
        let meta = test_meta();
        let page = render_page(&PageSpec {
            title: "Canon",
            description: "Canon",
            rel_path: "canon.html",
            section_title: "Concepts",
            section_rel_path: None,
            h1: "Canon",
            toc_headings: &[],
            body_html: "<p>Body.</p>",
            body_has_h1: false,
            prev: None,
            next: None,
            meta: &meta,
        });
        let with_link = |tag: &str| page.replacen("</head>", &format!("{tag}</head>"), 1);

        let ok = audit_single_html("canon.html", &with_link("<link rel=\"canonical\" href=\"https://hausergroup.github.io/curvebook/canon.html\">"))
            .unwrap();
        assert!(!ok.hrefs.iter().any(|h| h.ends_with("/curvebook/canon.html")), "canonical URL must not enter the local link checker");
        let off_site = audit_single_html("canon.html", &with_link("<link rel=\"canonical\" href=\"https://evil.example/canon.html\">"));
        assert!(off_site.unwrap_err().contains("canonical link"));
        // Any other external `<link href>` stays forbidden.
        let stylesheet =
            audit_single_html("canon.html", &with_link("<link rel=\"stylesheet\" href=\"https://hausergroup.github.io/curvebook/site.css\">"));
        assert!(stylesheet.unwrap_err().contains("external asset URL"));
    }

    #[test]
    fn da_03_09_release_snapshot_parent_docs_rejects_traversal_symlinks_and_bad_fragments() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("cb-docs-da-03-09-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(dir.join("docs/manual/v0.1")).unwrap();
        std::fs::write(dir.join("README.md"), "# root\n").unwrap();
        std::fs::write(dir.join("docs/index.html"), "<main id=\"main\"><h1>Home</h1></main>").unwrap();
        std::fs::write(dir.join("docs/privacy.html"), "<main id=\"main\"><h1>Privacy</h1><h2 id=\"policy\">P</h2></main>").unwrap();

        let mut meta = test_meta();
        meta.is_release_snapshot = true;
        let site = GeneratedSite { html_pages: BTreeMap::new(), expected_coverage: Default::default(), meta };
        let facts = BTreeMap::new();
        let out_dir = dir.join("docs/manual/v0.1");

        // Valid single-level parent-docs target and valid fragment pass.
        assert!(verify_target(&dir, &out_dir, &site, &facts, "index.html", "../../privacy.html#policy").is_ok());
        // Missing fragment in parent docs/ HTML must fail.
        let frag_err = verify_target(&dir, &out_dir, &site, &facts, "index.html", "../../privacy.html#missing").unwrap_err();
        assert!(frag_err.contains("missing `#missing`"), "{frag_err}");
        // Escaping docs/ via repeated `../` must fail (not stripped repeatedly by trim_start_matches).
        assert!(verify_target(&dir, &out_dir, &site, &facts, "index.html", "../../../README.md").is_err());
        assert!(verify_target(&dir, &out_dir, &site, &facts, "index.html", "../../../../index.html").is_err());

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("README.md"), dir.join("docs/link.html")).unwrap();
            assert!(verify_target(&dir, &out_dir, &site, &facts, "index.html", "../../link.html").is_err());
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn da_03_10_uri_obfuscation_and_meta_refresh_are_rejected_everywhere() {
        let meta = test_meta();
        let good = render_page(&PageSpec {
            title: "Sec",
            description: "Sec",
            rel_path: "index.html",
            section_title: "Get started",
            section_rel_path: None,
            h1: "Sec",
            toc_headings: &[],
            body_html: "<p>Clean body.</p>",
            body_has_h1: false,
            prev: None,
            next: None,
            meta: &meta,
        });

        for bad_snippet in [
            "<a href=\"&#x6a;avascript:alert(1)\">x</a>",
            "<a href=\"&#106;avascript:alert(1)\">x</a>",
            "<a href=\" java\tscript:alert(1)\">x</a>",
            "<a href=\"javascript&colon;alert(1)\">x</a>",
            "<a href=\"vbscript:msgbox(1)\">x</a>",
            "<a href=\" \n data:text/html,hi\">x</a>",
            "<img src=\"javascript:alert(1)\" alt=\"\">",
            "<video poster=\"&#x6a;avascript:alert(1)\"></video>",
            "<meta http-equiv=\"refresh\" content=\"0; url=https://evil.example/\">",
            "<meta http-equiv=\"refresh\" content=\"0; url=&#x6a;avascript:alert(1)\">",
            "<meta http-equiv=\"refresh\" content=\"0; url=data:text/html,hi\">",
        ] {
            let poisoned = good.replace("<p>Clean body.</p>", bad_snippet);
            assert!(audit_single_html("bad.html", &poisoned).is_err(), "expected `{bad_snippet}` to be rejected");
        }

        // Local meta-refresh to a missing page must fail check_internal_links_and_fragments.
        let local_missing_refresh = good.replace("<p>Clean body.</p>", "<meta http-equiv=\"refresh\" content=\"0; url=missing.html\">");
        let page_facts = audit_single_html("index.html", &local_missing_refresh).unwrap();
        let mut facts = BTreeMap::new();
        facts.insert(String::from("index.html"), page_facts);
        let mut html_pages = BTreeMap::new();
        html_pages.insert(String::from("index.html"), local_missing_refresh);
        let site = GeneratedSite { html_pages, expected_coverage: Default::default(), meta };
        assert!(check_internal_links_and_fragments(Path::new("."), Path::new("."), &site, &facts).is_err());

        // External meta-refresh in root docs/index.html must fail check_root_pages.
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let root_dir = std::env::temp_dir().join(format!("cb-docs-da-03-10-{}-{nanos}", std::process::id()));
        let docs = root_dir.join("docs");
        std::fs::create_dir_all(docs.join("manual")).unwrap();
        let root_html = |extra: &str| {
            format!(
                "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"referrer\" content=\"no-referrer\"><meta http-equiv=\"Content-Security-Policy\" content=\"{STRICT_CSP}\">{extra}<title>T</title></head><body><a class=\"skip-link\" href=\"#main\">Skip</a><main id=\"main\"><h1>T</h1></main></body></html>"
            )
        };
        std::fs::write(docs.join("index.html"), root_html("<meta http-equiv=\"refresh\" content=\"0; url=https://evil.example/\">")).unwrap();
        std::fs::write(docs.join("privacy.html"), root_html("")).unwrap();
        std::fs::write(docs.join("manual/index.html"), root_html("")).unwrap();
        assert!(check_root_pages(&root_dir).is_err());
        let _ = std::fs::remove_dir_all(&root_dir);
    }

    #[test]
    fn r12_10_manual_pages_emit_latest_canonical_og_and_latest_manifest_is_verified() {
        let meta = test_meta();
        let page = render_page(&PageSpec {
            title: "Analyses",
            description: "Statistical analyses reference.",
            rel_path: "analyses.html",
            section_title: "Analyses",
            section_rel_path: Some("section-analyses.html"),
            h1: "Analyses",
            toc_headings: &[],
            body_html: "<p>Body.</p>",
            body_has_h1: false,
            prev: None,
            next: None,
            meta: &meta,
        });
        assert!(audit_single_html("analyses.html", &page).is_ok());
        assert!(page.contains("<link rel=\"canonical\" href=\"https://hausergroup.github.io/curvebook/manual/latest/analyses.html\">"));
        assert!(page.contains("<meta property=\"og:type\" content=\"article\">"));
        assert!(page.contains("<meta property=\"og:site_name\" content=\"CurveBook\">"));
        assert!(page.contains("<meta property=\"og:title\" content=\"Analyses — CurveBook Documentation\">"));
        assert!(page.contains("<meta property=\"og:description\" content=\"Statistical analyses reference.\">"));
        assert!(page.contains("<meta property=\"og:url\" content=\"https://hausergroup.github.io/curvebook/manual/latest/analyses.html\">"));
        assert!(page.contains("<meta property=\"og:image\" content=\"https://hausergroup.github.io/curvebook/images/workspace-layout-dark.png\">"));
        assert!(page.contains("<meta name=\"twitter:card\" content=\"summary_large_image\">"));
        assert!(!page.contains("<script"));

        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("cb-docs-latest-{}-{nanos}", std::process::id()));
        let manual = dir.join("docs/manual");
        std::fs::create_dir_all(manual.join("v0.1")).unwrap();
        std::fs::create_dir_all(manual.join("latest")).unwrap();
        let manifest = "{\"version\":\"0.1.0\",\"commit\":\"0123456789abcdef0123456789abcdef01234567\"}\n";
        std::fs::write(manual.join("v0.1/manifest.json"), manifest).unwrap();
        std::fs::write(manual.join("latest/manifest.json"), manifest).unwrap();
        std::fs::write(manual.join("index.html"), "<meta http-equiv=\"refresh\" content=\"0; url=latest/index.html\">").unwrap();
        assert!(check_committed_snapshots(&dir).is_ok());

        // Redirect pointing to v0.1 instead of latest must fail when latest/manifest.json exists.
        std::fs::write(manual.join("index.html"), "<meta http-equiv=\"refresh\" content=\"0; url=v0.1/index.html\">").unwrap();
        assert!(check_committed_snapshots(&dir).unwrap_err().contains("latest/index.html"));

        // Mismatched latest/manifest.json must fail.
        std::fs::write(manual.join("index.html"), "<meta http-equiv=\"refresh\" content=\"0; url=latest/index.html\">").unwrap();
        std::fs::write(manual.join("latest/manifest.json"), "{\"version\":\"0.1.0\",\"commit\":\"ffffffffffffffffffffffffffffffffffffffff\"}\n")
            .unwrap();
        assert!(check_committed_snapshots(&dir).unwrap_err().contains("latest/manifest.json"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
