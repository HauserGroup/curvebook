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

    check_internal_links_and_fragments(root, out_dir, site, &facts)?;
    check_registry_coverage(&facts, site)?;
    check_committed_snapshots(root)?;
    run_hygiene_scanners(root, out_dir)?;
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

    while let Some((_, after_lt)) = rest.split_once('<') {
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
            if key_lower == "href" {
                let lower_v = v.to_ascii_lowercase();
                if lower_v.starts_with("javascript:") || lower_v.starts_with("data:") {
                    return Err(format!("{rel_path}: forbidden `href` URI `{v}`"));
                }
                if matches!(tag_lower.as_str(), "link" | "base") && is_external_url(&v) {
                    return Err(format!("{rel_path}: external asset URL forbidden in `<{tag_lower} href=\"{v}\">`"));
                }
                facts.hrefs.push(v);
            } else if matches!(key_lower.as_str(), "src" | "poster" | "action") {
                if is_external_url(&v) || v.to_ascii_lowercase().starts_with("data:") {
                    return Err(format!("{rel_path}: external or inline `{key_lower}` asset forbidden (`{v}`)"));
                }
                facts.asset_srcs.push(v);
            }
        }
        let _ = inside_code_or_pre;
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
    raw.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">")
}

fn is_external_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("//")
}

fn check_internal_links_and_fragments(root: &Path, out_dir: &Path, site: &GeneratedSite, facts: &BTreeMap<String, PageFacts>) -> Result<(), String> {
    for (page_rel, page_facts) in facts {
        for raw_href in &page_facts.hrefs {
            if is_external_url(raw_href) || raw_href.starts_with("mailto:") {
                continue;
            }
            verify_target(root, out_dir, site, facts, page_rel, raw_href)?;
        }
        for raw_src in &page_facts.asset_srcs {
            verify_target(root, out_dir, site, facts, page_rel, raw_src)?;
        }
    }
    Ok(())
}

fn verify_target(
    root: &Path,
    out_dir: &Path,
    site: &GeneratedSite,
    facts: &BTreeMap<String, PageFacts>,
    from_rel: &str,
    raw_ref: &str,
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
    if resolved.starts_with("../../") && site.meta.is_release_snapshot {
        let docs_rel = resolved.trim_start_matches("../../");
        let docs_target = root.join("docs").join(docs_rel);
        if !docs_target.is_file() {
            return Err(format!("{from_rel}: broken parent-docs asset/page `{raw_ref}` -> `{}`", docs_target.display()));
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

    if out_dir.join(&resolved).is_file() {
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

    let index_html = std::fs::read_to_string(manual_dir.join("index.html")).map_err(|e| format!("read docs/manual/index.html: {e}"))?;
    let expected_refresh = format!("{newest_dir}/index.html");
    if !index_html.contains("http-equiv=\"refresh\"") || !index_html.contains(&expected_refresh) {
        return Err(format!("docs/manual/index.html must redirect via meta-refresh to `{expected_refresh}`"));
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
    use super::super::markdown::{IdentityResolver, render_markdown};
    use super::super::template::{PageSpec, SiteMeta, render_page};
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
}
