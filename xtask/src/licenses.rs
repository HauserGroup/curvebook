//! `cargo xtask licenses [--html [out]]`: offline `deny.toml` / `Cargo.toml` / `Cargo.lock`
//! supply-chain checks, optional `cargo deny check licenses`, and `THIRD-PARTY-LICENSES.html`
//! generation.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

const DEFAULT_HTML_OUT: &str = "THIRD-PARTY-LICENSES.html";
const BANNED_LICENSE_PREFIXES: &[&str] = &["GPL", "AGPL", "LGPL"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenyPolicy {
    pub allow_licenses: Vec<String>,
    pub wildcards: String,
    pub unknown_registry: String,
    pub unknown_git: String,
    pub allow_registry: Vec<String>,
    pub allow_git: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockPackage {
    pub name: String,
    pub version: String,
    pub source: Option<String>,
}

/// Execute `cargo xtask licenses [--html [out]]`.
pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let html_out = parse_args(args)?;

    let deny_toml = std::fs::read_to_string(root.join("deny.toml")).map_err(|e| format!("deny.toml: {e}"))?;
    let policy = parse_deny_toml(&deny_toml)?;
    validate_deny_policy(&policy)?;

    let manifests = find_workspace_manifests(root)?;
    let mut wildcard_hits = Vec::new();
    for manifest in &manifests {
        let rel = manifest.strip_prefix(root).unwrap_or(manifest).to_string_lossy().replace('\\', "/");
        let text = std::fs::read_to_string(manifest).map_err(|e| format!("{rel}: {e}"))?;
        wildcard_hits.extend(find_wildcard_dependencies(&rel, &text));
    }
    if !wildcard_hits.is_empty() {
        return Err(format!("wildcard dependency version(s) forbidden by deny.toml:\n  {}", wildcard_hits.join("\n  ")));
    }

    let lock_text = std::fs::read_to_string(root.join("Cargo.lock")).map_err(|e| format!("Cargo.lock: {e}"))?;
    let packages = parse_cargo_lock(&lock_text);
    let source_errors = verify_lock_sources(&packages, &policy);
    if !source_errors.is_empty() {
        return Err(format!("disallowed Cargo.lock package source(s):\n  {}", source_errors.join("\n  ")));
    }

    run_cargo_deny_if_installed(root, manifests.len(), packages.len())?;

    if let Some(out_arg) = html_out {
        let out_path = if out_arg.is_absolute() { out_arg } else { root.join(out_arg) };
        generate_licenses_html(root, &out_path, &policy, &packages)?;
        println!("licenses: wrote {}", out_path.display());
    }
    Ok(())
}

pub fn parse_args(args: &[String]) -> Result<Option<PathBuf>, String> {
    match args {
        [] => Ok(None),
        [flag] if flag == "--html" => Ok(Some(PathBuf::from(DEFAULT_HTML_OUT))),
        [flag, out] if flag == "--html" => {
            if out.trim().is_empty() {
                return Err(String::from("--html requires a non-empty output path"));
            }
            Ok(Some(PathBuf::from(out)))
        }
        [single] if single.starts_with("--html=") => {
            let out = single.trim_start_matches("--html=").trim();
            if out.is_empty() {
                return Err(String::from("--html= requires a non-empty output path"));
            }
            Ok(Some(PathBuf::from(out)))
        }
        other => Err(format!("usage: cargo xtask licenses [--html [out]] (unexpected args: `{}`)", other.join(" "))),
    }
}

/// Parse `deny.toml` allowlist, ban settings, and source restrictions without external TOML crates.
pub fn parse_deny_toml(text: &str) -> Result<DenyPolicy, String> {
    let mut section = String::new();
    let mut allow_licenses = Vec::new();
    let mut wildcards = String::new();
    let mut unknown_registry = String::new();
    let mut unknown_git = String::new();
    let mut allow_registry = Vec::new();
    let mut allow_git = Vec::new();
    let mut collecting_array: Option<&'static str> = None;

    for raw in text.lines() {
        let line = strip_toml_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(target) = collecting_array {
            let target_vec = match target {
                "allow" => &mut allow_licenses,
                "allow-registry" => &mut allow_registry,
                _ => &mut allow_git,
            };
            extract_quoted_items(line, target_vec);
            if line.contains(']') {
                collecting_array = None;
            }
            continue;
        }
        if let Some(sec) = parse_section_header(line) {
            section = sec.to_owned();
            continue;
        }
        let Some((key, rhs)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let rhs = rhs.trim();
        match (section.as_str(), key) {
            ("licenses", "allow") => {
                extract_quoted_items(rhs, &mut allow_licenses);
                if !rhs.contains(']') {
                    collecting_array = Some("allow");
                }
            }
            ("bans", "wildcards") => wildcards = unquote_toml_str(rhs),
            ("sources", "unknown-registry") => unknown_registry = unquote_toml_str(rhs),
            ("sources", "unknown-git") => unknown_git = unquote_toml_str(rhs),
            ("sources", "allow-registry") => {
                extract_quoted_items(rhs, &mut allow_registry);
                if !rhs.contains(']') {
                    collecting_array = Some("allow-registry");
                }
            }
            ("sources", "allow-git") => {
                extract_quoted_items(rhs, &mut allow_git);
                if !rhs.contains(']') {
                    collecting_array = Some("allow-git");
                }
            }
            _ => {}
        }
    }

    if allow_licenses.is_empty() {
        return Err(String::from("deny.toml: `[licenses].allow` must list at least one SPDX license"));
    }
    Ok(DenyPolicy { allow_licenses, wildcards, unknown_registry, unknown_git, allow_registry, allow_git })
}

fn validate_deny_policy(policy: &DenyPolicy) -> Result<(), String> {
    for lic in &policy.allow_licenses {
        let upper = lic.to_ascii_uppercase();
        if BANNED_LICENSE_PREFIXES.iter().any(|p| upper.starts_with(p)) {
            return Err(format!("deny.toml: prohibited copyleft license `{lic}` in `[licenses].allow`"));
        }
    }
    if policy.wildcards != "deny" {
        return Err(format!("deny.toml: expected `[bans].wildcards = \"deny\"`, found `{}`", policy.wildcards));
    }
    if policy.unknown_registry != "deny" || policy.unknown_git != "deny" {
        return Err(String::from("deny.toml: `[sources]` must set `unknown-registry = \"deny\"` and `unknown-git = \"deny\"`"));
    }
    Ok(())
}

fn find_workspace_manifests(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut manifests = Vec::new();
    collect_manifests_recursive(root, &mut manifests)?;
    manifests.sort();
    Ok(manifests)
}

fn collect_manifests_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry {}: {e}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if path.is_dir() {
            if name_str == "target" || name_str == "dist" || name_str == "corpus" || name_str.starts_with('.') {
                continue;
            }
            collect_manifests_recursive(&path, out)?;
        } else if name_str == "Cargo.toml" {
            out.push(path);
        }
    }
    Ok(())
}

/// Detect wildcard `"*"` dependency versions inside dependency tables of a `Cargo.toml` file.
#[must_use]
pub fn find_wildcard_dependencies(rel_path: &str, toml: &str) -> Vec<String> {
    let mut hits = Vec::new();
    let mut section = String::new();
    for (idx, raw) in toml.lines().enumerate() {
        let line = strip_toml_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(sec) = parse_section_header(line) {
            section = sec.to_owned();
            continue;
        }
        if !is_dependency_section(&section) {
            continue;
        }
        let Some((lhs, rhs)) = line.split_once('=') else {
            continue;
        };
        let key = lhs.trim();
        let rhs = rhs.trim();
        if is_wildcard_dep_spec(key, rhs) {
            hits.push(format!("{rel_path}:{}: `{line}`", idx + 1));
        }
    }
    hits
}

fn is_dependency_section(section: &str) -> bool {
    section == "dependencies"
        || section == "dev-dependencies"
        || section == "build-dependencies"
        || section == "workspace.dependencies"
        || section.ends_with(".dependencies")
        || section.ends_with(".dev-dependencies")
        || section.ends_with(".build-dependencies")
        || section.starts_with("dependencies.")
        || section.starts_with("dev-dependencies.")
        || section.starts_with("build-dependencies.")
}

fn is_wildcard_dep_spec(key: &str, rhs: &str) -> bool {
    if rhs.starts_with('"') || rhs.starts_with('\'') {
        let v = unquote_toml_str(rhs);
        return (key == "version" || !key.is_empty()) && is_wildcard_version_str(&v);
    }
    if rhs.starts_with('{')
        && let Some(ver_pos) = rhs.find("version")
    {
        let after = &rhs[ver_pos + "version".len()..];
        if let Some((_, val_part)) = after.split_once('=') {
            let val = val_part.trim();
            if val.starts_with('"') || val.starts_with('\'') {
                let first_quoted = extract_first_quoted(val);
                return is_wildcard_version_str(&first_quoted);
            }
        }
    }
    false
}

fn is_wildcard_version_str(v: &str) -> bool {
    v == "*" || v.contains(".*")
}

/// Parse `[[package]]` entries from `Cargo.lock`.
#[must_use]
pub fn parse_cargo_lock(lock_text: &str) -> Vec<LockPackage> {
    let mut pkgs = Vec::new();
    let mut current: Option<LockPackage> = None;

    for raw in lock_text.lines() {
        let line = raw.trim();
        if line == "[[package]]" {
            if let Some(pkg) = current.take()
                && !pkg.name.is_empty()
            {
                pkgs.push(pkg);
            }
            current = Some(LockPackage { name: String::new(), version: String::new(), source: None });
            continue;
        }
        if line.starts_with('[') {
            if let Some(pkg) = current.take()
                && !pkg.name.is_empty()
            {
                pkgs.push(pkg);
            }
            continue;
        }
        let Some(pkg) = current.as_mut() else {
            continue;
        };
        if let Some((k, v)) = line.split_once('=') {
            match k.trim() {
                "name" => pkg.name = unquote_toml_str(v.trim()),
                "version" => pkg.version = unquote_toml_str(v.trim()),
                "source" => pkg.source = Some(unquote_toml_str(v.trim())),
                _ => {}
            }
        }
    }
    if let Some(pkg) = current
        && !pkg.name.is_empty()
    {
        pkgs.push(pkg);
    }
    pkgs
}

/// Verify every external `Cargo.lock` source against `deny.toml`'s `[sources]` policy.
#[must_use]
pub fn verify_lock_sources(packages: &[LockPackage], policy: &DenyPolicy) -> Vec<String> {
    let mut errors = Vec::new();
    for pkg in packages {
        let Some(src) = pkg.source.as_deref() else {
            continue;
        };
        if let Some(git_url) = src.strip_prefix("git+") {
            let allowed = policy.allow_git.iter().any(|g| git_url.starts_with(g.as_str()));
            if !allowed {
                errors.push(format!("{} {}: disallowed git source `{src}`", pkg.name, pkg.version));
            }
        } else if let Some(reg_url) = src.strip_prefix("registry+") {
            let allowed = policy.allow_registry.iter().any(|r| r == reg_url);
            if !allowed {
                errors.push(format!("{} {}: unknown registry `{reg_url}`", pkg.name, pkg.version));
            }
        } else {
            errors.push(format!("{} {}: unknown source `{src}`", pkg.name, pkg.version));
        }
    }
    errors
}

fn run_cargo_deny_if_installed(root: &Path, manifest_count: usize, lock_count: usize) -> Result<(), String> {
    if has_tool("cargo-deny") {
        let status = Command::new("cargo")
            .current_dir(root)
            .args(["deny", "check", "licenses"])
            .status()
            .map_err(|e| format!("cargo deny check licenses: {e}"))?;
        if !status.success() {
            return Err(format!("`cargo deny check licenses` exited with {status}"));
        }
        return Ok(());
    }

    let in_ci = std::env::var_os("CI").is_some_and(|v| !v.is_empty());
    if in_ci {
        return Err(String::from("`cargo-deny` is not installed in PATH (required when CI=true)"));
    }
    println!(
        "licenses: `cargo-deny` not found in PATH; offline checks passed ({manifest_count} manifests, {lock_count} locked packages; enforced when CI=true)"
    );
    Ok(())
}

fn has_tool(bin: &str) -> bool {
    Command::new(bin).arg("--version").output().is_ok_and(|o| o.status.success())
}

fn generate_licenses_html(root: &Path, out_path: &Path, policy: &DenyPolicy, packages: &[LockPackage]) -> Result<(), String> {
    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("create_dir_all {}: {e}", parent.display()))?;
    }
    if has_tool("cargo-about") {
        let out_str = out_path.to_string_lossy().into_owned();
        let status = Command::new("cargo")
            .current_dir(root)
            .args(["about", "generate", "about.hbs", "-o", &out_str])
            .status()
            .map_err(|e| format!("cargo about generate: {e}"))?;
        if !status.success() {
            return Err(format!("`cargo about generate` exited with {status}"));
        }
        return Ok(());
    }

    let attribution = std::fs::read_to_string(root.join("ATTRIBUTION.md")).unwrap_or_default();
    let notice = std::fs::read_to_string(root.join("NOTICE")).unwrap_or_default();
    let html = render_licenses_html(policy, packages, &attribution, &notice);
    std::fs::write(out_path, html).map_err(|e| format!("write {}: {e}", out_path.display()))
}

/// Render a standalone pure-Rust `THIRD-PARTY-LICENSES.html` from lockfile, policy, and notices.
#[must_use]
pub fn render_licenses_html(policy: &DenyPolicy, packages: &[LockPackage], attribution_md: &str, notice_txt: &str) -> String {
    let third_party: Vec<&LockPackage> = packages.iter().filter(|p| p.source.is_some()).collect();
    let mut out = String::from(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n  <meta charset=\"utf-8\">\n  \
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n  \
         <title>LumenCraft — Third-Party Software Licenses</title>\n  <style>\n    \
         body { font-family: -apple-system, BlinkMacSystemFont, \"Segoe UI\", Roboto, sans-serif; \
         line-height: 1.55; color: #1b2230; background: #fff; margin: 2rem auto; max-width: 960px; padding: 0 1.25rem; }\n    \
         h1, h2 { color: #1b2d56; } h1 { border-bottom: 2px solid #d7dee8; padding-bottom: 0.5rem; }\n    \
         table { border-collapse: collapse; width: 100%; margin: 1rem 0 2rem; }\n    \
         th, td { border: 1px solid #d7dee8; padding: 0.45rem 0.65rem; text-align: left; font-size: 0.9rem; }\n    \
         th { background: #f5f8fc; color: #1b2d56; }\n    \
         pre { background: #f5f8fc; border: 1px solid #d7dee8; border-radius: 6px; padding: 0.9rem; white-space: pre-wrap; }\n  \
         </style>\n</head>\n<body>\n  <header>\n    <h1>LumenCraft — Third-Party Software Licenses</h1>\n    \
         <p>Permitted crate SPDX allowlist from <code>deny.toml</code>: ",
    );
    let escaped_allow: Vec<String> = policy.allow_licenses.iter().map(|s| format!("<code>{}</code>", escape_html(s))).collect();
    out.push_str(&escaped_allow.join(", "));
    out.push_str(
        ".</p>\n  </header>\n  <main>\n    <h2>Resolved Third-Party Crates</h2>\n    \
         <table><thead><tr><th>Crate</th><th>Version</th><th>Source</th></tr></thead><tbody>\n",
    );
    for pkg in third_party {
        let name = escape_html(&pkg.name);
        let version = escape_html(&pkg.version);
        let source = escape_html(pkg.source.as_deref().unwrap_or("workspace"));
        let _ = writeln!(out, "      <tr><td>{name}</td><td>{version}</td><td>{source}</td></tr>");
    }
    out.push_str("    </tbody></table>\n");
    if !notice_txt.trim().is_empty() {
        out.push_str("    <h2>Notice</h2>\n    <pre>");
        out.push_str(&escape_html(notice_txt.trim()));
        out.push_str("</pre>\n");
    }
    if !attribution_md.trim().is_empty() {
        out.push_str("    <h2>Asset Attribution (ATTRIBUTION.md)</h2>\n    <pre>");
        out.push_str(&escape_html(attribution_md.trim()));
        out.push_str("</pre>\n");
    }
    out.push_str("  </main>\n</body>\n</html>\n");
    out
}

fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn strip_toml_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_dq = false;
    let mut in_sq = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if in_dq => {
                i += 2;
                continue;
            }
            b'"' if !in_sq => in_dq = !in_dq,
            b'\'' if !in_dq => in_sq = !in_sq,
            b'#' if !in_dq && !in_sq => return &line[..i],
            _ => {}
        }
        i += 1;
    }
    line
}

fn parse_section_header(line: &str) -> Option<&str> {
    if line.starts_with('[') && !line.starts_with("[[") && line.ends_with(']') { Some(line[1..line.len() - 1].trim()) } else { None }
}

fn unquote_toml_str(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2 && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\''))) {
        t[1..t.len() - 1].to_owned()
    } else {
        t.to_owned()
    }
}

fn extract_first_quoted(s: &str) -> String {
    let Some(quote) = s.chars().next().filter(|&c| c == '"' || c == '\'') else {
        return String::new();
    };
    s[1..].split(quote).next().unwrap_or("").to_owned()
}

fn extract_quoted_items(s: &str, out: &mut Vec<String>) {
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch != '"' && ch != '\'' {
            continue;
        }
        let mut item = String::new();
        for c in chars.by_ref() {
            if c == ch {
                break;
            }
            item.push(c);
        }
        if !item.is_empty() {
            out.push(item);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        find_wildcard_dependencies, parse_args, parse_cargo_lock, parse_deny_toml, render_licenses_html, validate_deny_policy, verify_lock_sources,
    };

    #[test]
    fn parses_deny_policy_and_flags_wildcards_and_bad_sources() {
        let deny = include_str!("../../deny.toml");
        let policy = parse_deny_toml(deny).unwrap();
        validate_deny_policy(&policy).unwrap();
        assert!(policy.allow_licenses.contains(&String::from("MIT")));
        assert!(policy.allow_licenses.contains(&String::from("Apache-2.0")));

        let manifest = "\
[profile.dev.package.\"*\"]
opt-level = 2

[dependencies]
serde = \"1\"
bad_star = \"*\"
bad_inline = { version = \"1.*\", features = [\"x\"] }
";
        let hits = find_wildcard_dependencies("crates/x/Cargo.toml", manifest);
        assert_eq!(hits.len(), 2, "{hits:?}");

        let lock = "\
[[package]]
name = \"lumencraft-doc\"
version = \"0.1.0\"

[[package]]
name = \"serde\"
version = \"1.0.229\"
source = \"registry+https://github.com/rust-lang/crates.io-index\"

[[package]]
name = \"evil-git\"
version = \"0.1.0\"
source = \"git+https://example.com/evil.git#abc\"

[[package]]
name = \"evil-reg\"
version = \"0.1.0\"
source = \"registry+https://example.com/index\"
";
        let pkgs = parse_cargo_lock(lock);
        let errs = verify_lock_sources(&pkgs, &policy);
        assert_eq!(errs.len(), 2, "{errs:?}");

        let html = render_licenses_html(&policy, &pkgs, "| a | <script> |", "Copyright LumenCraft");
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("serde"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("Copyright LumenCraft"));
    }

    #[test]
    fn parses_html_flag_with_or_without_explicit_path() {
        assert_eq!(parse_args(&[]).unwrap(), None);
        assert_eq!(parse_args(&[String::from("--html")]).unwrap(), Some(PathBuf::from("THIRD-PARTY-LICENSES.html")));
        assert_eq!(parse_args(&[String::from("--html"), String::from("dist/lic.html")]).unwrap(), Some(PathBuf::from("dist/lic.html")));
        assert_eq!(parse_args(&[String::from("--html=dist/lic.html")]).unwrap(), Some(PathBuf::from("dist/lic.html")));
        assert!(parse_args(&[String::from("--bogus")]).is_err());
    }
}
