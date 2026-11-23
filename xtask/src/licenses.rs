//! `cargo xtask licenses [--html [out]]`: offline `deny.toml` / `about.toml` / `Cargo.lock`
//! / resolved SPDX metadata license checks, optional `cargo deny check licenses`, and
//! `THIRD-PARTY-LICENSES.html` generation with full license texts (`R0.5`, `R2-S13-01`, `R2-S13-08`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const DEFAULT_HTML_OUT: &str = "THIRD-PARTY-LICENSES.html";
const BANNED_LICENSE_PREFIXES: &[&str] = &["GPL", "AGPL", "LGPL"];
pub const CANONICAL_ALLOWED_LICENSES: &[&str] = &["MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "Unicode-3.0", "CC0-1.0"];

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLicensePkg {
    pub id: String,
    pub name: String,
    pub version: String,
    pub source: Option<String>,
    pub license: String,
    pub license_file: Option<String>,
    pub manifest_path: Option<PathBuf>,
    pub is_shipped: bool,
}

/// Execute `cargo xtask licenses [--html [out]]`.
pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let html_out = parse_args(args)?;

    let deny_toml = std::fs::read_to_string(root.join("deny.toml")).map_err(|e| format!("deny.toml: {e}"))?;
    let policy = parse_deny_toml(&deny_toml)?;
    validate_deny_policy(&policy)?;
    validate_about_toml_parity(root, &policy)?;

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

    let meta = run_locked_offline_metadata(root)?;
    let resolved_pkgs = verify_resolved_spdx_metadata(&meta, &policy.allow_licenses)?;

    run_cargo_deny_if_installed(root, manifests.len(), resolved_pkgs.len())?;

    if let Some(out_arg) = html_out {
        let out_path = if out_arg.is_absolute() { out_arg } else { root.join(out_arg) };
        generate_licenses_html(root, &out_path, &policy, &resolved_pkgs)?;
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
        if !CANONICAL_ALLOWED_LICENSES.iter().any(|allowed| allowed.eq_ignore_ascii_case(lic)) {
            return Err(format!(
                "deny.toml: non-canonical or unapproved license `{lic}` in `[licenses].allow` (allowed set: {})",
                CANONICAL_ALLOWED_LICENSES.join(", ")
            ));
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

/// Validate that `about.toml` exists and its `accepted` license list matches `deny.toml` exactly.
pub fn validate_about_toml_parity(root: &Path, policy: &DenyPolicy) -> Result<(), String> {
    let about_text = std::fs::read_to_string(root.join("about.toml")).map_err(|e| format!("about.toml: {e}"))?;
    let accepted = parse_about_accepted(&about_text)?;
    let deny_set: BTreeSet<String> = policy.allow_licenses.iter().map(|s| s.to_ascii_lowercase()).collect();
    let about_set: BTreeSet<String> = accepted.iter().map(|s| s.to_ascii_lowercase()).collect();
    if deny_set != about_set {
        return Err(format!(
            "about.toml `accepted` licenses ({}) do not match deny.toml `[licenses].allow` ({})",
            accepted.join(", "),
            policy.allow_licenses.join(", ")
        ));
    }
    Ok(())
}

fn parse_about_accepted(text: &str) -> Result<Vec<String>, String> {
    let mut accepted = Vec::new();
    let mut collecting = false;
    for raw in text.lines() {
        let line = strip_toml_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if collecting {
            extract_quoted_items(line, &mut accepted);
            if line.contains(']') {
                collecting = false;
            }
            continue;
        }
        if let Some((key, rhs)) = line.split_once('=')
            && key.trim() == "accepted"
        {
            extract_quoted_items(rhs.trim(), &mut accepted);
            if !rhs.contains(']') {
                collecting = true;
            }
        }
    }
    if accepted.is_empty() {
        return Err(String::from("about.toml: `accepted` license list must not be empty"));
    }
    Ok(accepted)
}

fn run_locked_offline_metadata(root: &Path) -> Result<Value, String> {
    let out = Command::new("cargo")
        .current_dir(root)
        .args(["metadata", "--format-version", "1", "--locked", "--offline", "--all-features"])
        .output()
        .map_err(|e| format!("cargo metadata --locked --offline --all-features: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata --locked --offline --all-features failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata json: {e}"))
}

/// Evaluate every active resolved crate in `meta` against `allow_licenses` using boolean SPDX semantics.
pub fn verify_resolved_spdx_metadata(meta: &Value, allow_licenses: &[String]) -> Result<Vec<ResolvedLicensePkg>, String> {
    let pkgs_arr = meta.get("packages").and_then(Value::as_array).ok_or_else(|| String::from("cargo metadata missing `packages` array"))?;
    let mut pkg_map: BTreeMap<String, &Value> = BTreeMap::new();
    for p in pkgs_arr {
        if let Some(id) = p.get("id").and_then(Value::as_str) {
            pkg_map.insert(id.to_owned(), p);
        }
    }

    let nodes_arr = meta
        .get("resolve")
        .and_then(|r| r.get("nodes"))
        .and_then(Value::as_array)
        .ok_or_else(|| String::from("cargo metadata missing `resolve.nodes` array"))?;
    let mut node_map: BTreeMap<String, &Value> = BTreeMap::new();
    for n in nodes_arr {
        if let Some(id) = n.get("id").and_then(Value::as_str) {
            node_map.insert(id.to_owned(), n);
        }
    }

    let members =
        meta.get("workspace_members").and_then(Value::as_array).ok_or_else(|| String::from("cargo metadata missing `workspace_members` array"))?;
    let mut reachable = BTreeMap::<String, bool>::new();
    let mut queue: VecDeque<(String, bool)> = VecDeque::new();
    for member in members {
        if let Some(id) = member.as_str() {
            reachable.insert(id.to_owned(), true);
            queue.push_back((id.to_owned(), true));
        }
    }

    while let Some((pid, shipped_here)) = queue.pop_front() {
        let (Some(&pkg), Some(&node)) = (pkg_map.get(&pid), node_map.get(&pid)) else {
            continue;
        };
        for (dep_pkg_id, dep_is_shipped) in active_resolve_edges(pkg, node, shipped_here) {
            let next_shipped = shipped_here && dep_is_shipped;
            match reachable.get_mut(&dep_pkg_id) {
                Some(prev) => {
                    if next_shipped && !*prev {
                        *prev = true;
                        queue.push_back((dep_pkg_id, true));
                    }
                }
                None => {
                    reachable.insert(dep_pkg_id.clone(), next_shipped);
                    queue.push_back((dep_pkg_id, next_shipped));
                }
            }
        }
    }

    let mut errors = Vec::new();
    let mut out = Vec::new();
    for (pid, is_shipped) in reachable {
        let Some(&pkg) = pkg_map.get(&pid) else {
            continue;
        };
        let name = pkg.get("name").and_then(Value::as_str).unwrap_or("<unknown>").to_owned();
        let version = pkg.get("version").and_then(Value::as_str).unwrap_or("0.0.0").to_owned();
        let source = pkg.get("source").and_then(Value::as_str).map(ToOwned::to_owned);
        let license = pkg.get("license").and_then(Value::as_str).unwrap_or("").trim().to_owned();
        let license_file = pkg.get("license_file").and_then(Value::as_str).map(ToOwned::to_owned);
        let manifest_path = pkg.get("manifest_path").and_then(Value::as_str).map(PathBuf::from);

        if license.is_empty() || license.eq_ignore_ascii_case("NONE") || license.eq_ignore_ascii_case("NOASSERTION") {
            errors.push(format!("{name} {version}: missing SPDX `license` field in crate metadata"));
        } else if let Err(reason) = evaluate_spdx_expression(&license, allow_licenses) {
            errors.push(format!("{name} {version}: disallowed SPDX license `{license}` ({reason})"));
        }

        out.push(ResolvedLicensePkg { id: pid, name, version, source, license, license_file, manifest_path, is_shipped });
    }

    if !errors.is_empty() {
        return Err(format!("offline SPDX license check failed ({} crate violation(s)):\n  {}", errors.len(), errors.join("\n  ")));
    }
    out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.version.cmp(&b.version)));
    Ok(out)
}

fn active_resolve_edges(pkg: &Value, node: &Value, parent_shipped: bool) -> Vec<(String, bool)> {
    let mut enabled_feats: BTreeSet<String> = BTreeSet::new();
    if let Some(arr) = node.get("features").and_then(Value::as_array) {
        for item in arr {
            if let Some(s) = item.as_str() {
                enabled_feats.insert(s.replace('-', "_"));
            }
        }
    }
    let mut enabled_opt = enabled_feats.clone();
    if let Some(Value::Object(feat_map)) = pkg.get("features") {
        for feat in &enabled_feats {
            let Some(arr) = feat_map.iter().find(|(k, _)| k.replace('-', "_") == *feat).and_then(|(_, v)| v.as_array()) else {
                continue;
            };
            for entry in arr {
                let Some(spec) = entry.as_str() else {
                    continue;
                };
                if let Some(dep_name) = spec.strip_prefix("dep:") {
                    enabled_opt.insert(dep_name.replace('-', "_"));
                } else if let Some((dep_name, _)) = spec.split_once('/')
                    && !dep_name.ends_with('?')
                {
                    enabled_opt.insert(dep_name.replace('-', "_"));
                }
            }
        }
    }

    let mut opt_map: BTreeMap<String, bool> = BTreeMap::new();
    if let Some(deps) = pkg.get("dependencies").and_then(Value::as_array) {
        for dep in deps {
            let raw_name = dep.get("rename").and_then(Value::as_str).or_else(|| dep.get("name").and_then(Value::as_str)).unwrap_or("");
            let key = raw_name.replace('-', "_");
            let is_opt = matches!(dep.get("optional"), Some(Value::Bool(true)));
            if is_opt {
                opt_map.entry(key).or_insert(true);
            } else {
                opt_map.insert(key, false);
            }
        }
    }

    let mut edges = Vec::new();
    if let Some(deps) = node.get("deps").and_then(Value::as_array) {
        for dep in deps {
            let Some(dep_pkg) = dep.get("pkg").and_then(Value::as_str) else {
                continue;
            };
            let name_key = dep.get("name").and_then(Value::as_str).unwrap_or("").replace('-', "_");
            let is_opt = opt_map.get(&name_key).copied().unwrap_or(false);
            if is_opt && !enabled_opt.contains(&name_key) {
                continue;
            }
            let edge_shipped = parent_shipped && dep_has_non_dev_kind(dep);
            edges.push((dep_pkg.to_owned(), edge_shipped));
        }
    }
    edges
}

fn dep_has_non_dev_kind(dep: &Value) -> bool {
    let Some(kinds) = dep.get("dep_kinds").and_then(Value::as_array) else {
        return true;
    };
    if kinds.is_empty() {
        return true;
    }
    kinds.iter().any(|k| !matches!(k.get("kind").and_then(Value::as_str), Some("dev")))
}

/// Evaluate a SPDX 2.x expression (`MIT AND Apache-2.0`, `(MIT OR Apache-2.0) AND Unicode-3.0`,
/// legacy `MIT/Apache-2.0`) against `allow_licenses`.
pub fn evaluate_spdx_expression(expr: &str, allow_licenses: &[String]) -> Result<(), String> {
    let tokens = tokenize_spdx(expr)?;
    let mut idx = 0usize;
    let allowed = parse_spdx_or(&tokens, &mut idx, allow_licenses)?;
    if idx != tokens.len() {
        return Err(format!("unexpected trailing SPDX token `{}`", tokens[idx].as_str()));
    }
    if allowed { Ok(()) } else { Err(String::from("expression not satisfied by policy allowlist")) }
}

#[derive(Debug, PartialEq, Eq)]
enum SpdxToken {
    LParen,
    RParen,
    And,
    Or,
    With,
    Ident(String),
}

impl SpdxToken {
    fn as_str(&self) -> &str {
        match self {
            SpdxToken::LParen => "(",
            SpdxToken::RParen => ")",
            SpdxToken::And => "AND",
            SpdxToken::Or => "OR",
            SpdxToken::With => "WITH",
            SpdxToken::Ident(s) => s.as_str(),
        }
    }
}

fn tokenize_spdx(expr: &str) -> Result<Vec<SpdxToken>, String> {
    let mut tokens = Vec::new();
    let bytes = expr.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        match b {
            b'(' => {
                tokens.push(SpdxToken::LParen);
                i += 1;
            }
            b')' => {
                tokens.push(SpdxToken::RParen);
                i += 1;
            }
            b'/' => {
                tokens.push(SpdxToken::Or);
                i += 1;
            }
            _ => {
                let start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'(' | b')' | b'/') {
                    i += 1;
                }
                let word = &expr[start..i];
                match word {
                    "AND" => tokens.push(SpdxToken::And),
                    "OR" => tokens.push(SpdxToken::Or),
                    "WITH" => tokens.push(SpdxToken::With),
                    _ => tokens.push(SpdxToken::Ident(word.to_owned())),
                }
            }
        }
    }
    if tokens.is_empty() {
        return Err(String::from("empty SPDX license expression"));
    }
    Ok(tokens)
}

fn parse_spdx_or(tokens: &[SpdxToken], idx: &mut usize, allow: &[String]) -> Result<bool, String> {
    let mut lhs = parse_spdx_and(tokens, idx, allow)?;
    while matches!(tokens.get(*idx), Some(SpdxToken::Or)) {
        *idx += 1;
        let rhs = parse_spdx_and(tokens, idx, allow)?;
        lhs |= rhs;
    }
    Ok(lhs)
}

fn parse_spdx_and(tokens: &[SpdxToken], idx: &mut usize, allow: &[String]) -> Result<bool, String> {
    let mut lhs = parse_spdx_primary(tokens, idx, allow)?;
    while matches!(tokens.get(*idx), Some(SpdxToken::And)) {
        *idx += 1;
        let rhs = parse_spdx_primary(tokens, idx, allow)?;
        lhs &= rhs;
    }
    Ok(lhs)
}

fn parse_spdx_primary(tokens: &[SpdxToken], idx: &mut usize, allow: &[String]) -> Result<bool, String> {
    match tokens.get(*idx) {
        Some(SpdxToken::LParen) => {
            *idx += 1;
            let val = parse_spdx_or(tokens, idx, allow)?;
            if !matches!(tokens.get(*idx), Some(SpdxToken::RParen)) {
                return Err(String::from("unclosed `(` in SPDX license expression"));
            }
            *idx += 1;
            Ok(val)
        }
        Some(SpdxToken::Ident(id)) => {
            *idx += 1;
            if matches!(tokens.get(*idx), Some(SpdxToken::With)) {
                *idx += 1;
                if !matches!(tokens.get(*idx), Some(SpdxToken::Ident(_))) {
                    return Err(String::from("missing exception identifier after `WITH`"));
                }
                *idx += 1;
                return Ok(false);
            }
            Ok(allow.iter().any(|allowed| allowed.eq_ignore_ascii_case(id)))
        }
        Some(other) => Err(format!("unexpected SPDX token `{}`", other.as_str())),
        None => Err(String::from("unexpected end of SPDX expression")),
    }
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
        let ft = entry.file_type().map_err(|e| format!("file_type {}: {e}", entry.path().display()))?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if ft.is_dir() {
            if name_str == "target" || name_str == "dist" || name_str == "corpus" || name_str.starts_with('.') {
                continue;
            }
            collect_manifests_recursive(&path, out)?;
        } else if ft.is_file() && name_str == "Cargo.toml" {
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
        || section.starts_with("workspace.dependencies.")
        || section.contains(".dependencies.")
        || section.contains(".dev-dependencies.")
        || section.contains(".build-dependencies.")
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

fn run_cargo_deny_if_installed(root: &Path, manifest_count: usize, resolved_count: usize) -> Result<(), String> {
    if has_tool("cargo-deny") {
        let mut args = vec!["deny"];
        if std::env::var_os("CARGO_NET_OFFLINE").is_some_and(|v| v == "true" || v == "1") {
            args.push("--offline");
        }
        args.extend(["check", "licenses"]);
        let status = Command::new("cargo").current_dir(root).args(args).status().map_err(|e| format!("cargo deny check licenses: {e}"))?;
        if !status.success() {
            return Err(format!("`cargo deny check licenses` exited with {status}"));
        }
        return Ok(());
    }

    if crate::workflows::is_ci_env() {
        return Err(String::from("`cargo-deny` is not installed in PATH (required when CI=true)"));
    }
    println!(
        "licenses: `cargo-deny` not found in PATH; offline checks passed ({manifest_count} manifests, {resolved_count} active packages verified against deny.toml/about.toml SPDX policy; cargo-deny enforced when CI=true)"
    );
    Ok(())
}

fn has_tool(bin: &str) -> bool {
    Command::new(bin).arg("--version").output().is_ok_and(|o| o.status.success())
}

fn generate_licenses_html(root: &Path, out_path: &Path, policy: &DenyPolicy, resolved: &[ResolvedLicensePkg]) -> Result<(), String> {
    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("create_dir_all {}: {e}", parent.display()))?;
    }
    let in_ci = crate::workflows::is_ci_env();
    let is_offline = std::env::var_os("CARGO_NET_OFFLINE").is_some_and(|v| v == "true" || v == "1");

    if has_tool("cargo-about") && (in_ci || !is_offline) {
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

    if in_ci {
        return Err(String::from("`cargo-about` is not installed in PATH (required for `--html` when CI=true)"));
    }

    let attribution = std::fs::read_to_string(root.join("ATTRIBUTION.md")).map_err(|e| format!("read ATTRIBUTION.md: {e}"))?;
    let notice = std::fs::read_to_string(root.join("NOTICE")).map_err(|e| format!("read NOTICE: {e}"))?;
    let html = render_detailed_licenses_html(policy, resolved, &attribution, &notice);
    curvebook_platform::fs::write_atomic(out_path, html.as_bytes()).map_err(|e| format!("write {}: {e}", out_path.display()))
}

/// Render a standalone offline `THIRD-PARTY-LICENSES.html` from resolved non-dev crates, on-disk
/// crate `LICENSE*`/`NOTICE*` files, OSI SPDX full texts, `NOTICE`, and `ATTRIBUTION.md`.
#[must_use]
pub fn render_detailed_licenses_html(policy: &DenyPolicy, resolved: &[ResolvedLicensePkg], attribution_md: &str, notice_txt: &str) -> String {
    let shipped_third_party: Vec<&ResolvedLicensePkg> = resolved.iter().filter(|p| p.is_shipped && p.source.is_some()).collect();
    let mut out = render_html_preamble(policy);
    out.push_str(
        "<h2>Resolved Shipped Third-Party Crates</h2>\n    \
         <table><thead><tr><th>Crate</th><th>Version</th><th>SPDX License</th><th>Source</th></tr></thead><tbody>\n",
    );

    let mut collected_texts: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for pkg in &shipped_third_party {
        let name = escape_html(&pkg.name);
        let version = escape_html(&pkg.version);
        let license = escape_html(&pkg.license);
        let source = escape_html(pkg.source.as_deref().unwrap_or("workspace"));
        let _ = writeln!(out, "      <tr><td>{name}</td><td>{version}</td><td><code>{license}</code></td><td>{source}</td></tr>");

        let crate_label = format!("{} {}", pkg.name, pkg.version);
        let mut texts = collect_crate_license_texts(pkg);
        if texts.is_empty() {
            for spdx_id in extract_spdx_ids(&pkg.license) {
                if let Some(canonical) = canonical_spdx_license_text(spdx_id) {
                    texts.push(format!("{spdx_id}\n\n{canonical}"));
                }
            }
        }
        for body in texts {
            collected_texts.entry(body).or_default().push(crate_label.clone());
        }
    }
    out.push_str("    </tbody></table>\n");

    if !collected_texts.is_empty() {
        out.push_str("    <h2>Full Third-Party License Texts &amp; Notices</h2>\n");
        for (body, mut crates) in collected_texts {
            crates.sort();
            crates.dedup();
            let used_by = escape_html(&crates.join(", "));
            let escaped_body = escape_html(body.trim());
            let _ = writeln!(out, "    <h3>Crates: {used_by}</h3>\n    <pre>{escaped_body}</pre>");
        }
    }
    append_notice_and_attribution(&mut out, notice_txt, attribution_md);
    out.push_str("  </main>\n</body>\n</html>\n");
    out
}

/// Render a standalone pure-Rust `THIRD-PARTY-LICENSES.html` from lockfile, policy, and notices.
#[cfg(test)]
#[must_use]
pub fn render_licenses_html(policy: &DenyPolicy, packages: &[LockPackage], attribution_md: &str, notice_txt: &str) -> String {
    let third_party: Vec<&LockPackage> = packages.iter().filter(|p| p.source.is_some()).collect();
    let mut out = render_html_preamble(policy);
    out.push_str(
        "<h2>Resolved Third-Party Crates</h2>\n    \
         <table><thead><tr><th>Crate</th><th>Version</th><th>Source</th></tr></thead><tbody>\n",
    );
    for pkg in third_party {
        let name = escape_html(&pkg.name);
        let version = escape_html(&pkg.version);
        let source = escape_html(pkg.source.as_deref().unwrap_or("workspace"));
        let _ = writeln!(out, "      <tr><td>{name}</td><td>{version}</td><td>{source}</td></tr>");
    }
    out.push_str("    </tbody></table>\n    <h2>Canonical License Texts</h2>\n");
    for lic in &policy.allow_licenses {
        if let Some(text) = canonical_spdx_license_text(lic) {
            let escaped_id = escape_html(lic);
            let escaped_text = escape_html(text);
            let _ = writeln!(out, "    <h3>{escaped_id}</h3>\n    <pre>{escaped_text}</pre>");
        }
    }
    append_notice_and_attribution(&mut out, notice_txt, attribution_md);
    out.push_str("  </main>\n</body>\n</html>\n");
    out
}

fn render_html_preamble(policy: &DenyPolicy) -> String {
    let mut out = String::from(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n  <meta charset=\"utf-8\">\n  \
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n  \
         <title>CurveBook — Third-Party Software Licenses</title>\n  <style>\n    \
         body { font-family: -apple-system, BlinkMacSystemFont, \"Segoe UI\", Roboto, sans-serif; \
         line-height: 1.55; color: #1b2230; background: #fff; margin: 2rem auto; max-width: 960px; padding: 0 1.25rem; }\n    \
         h1, h2, h3 { color: #1b2d56; } h1 { border-bottom: 2px solid #d7dee8; padding-bottom: 0.5rem; }\n    \
         table { border-collapse: collapse; width: 100%; margin: 1rem 0 2rem; }\n    \
         th, td { border: 1px solid #d7dee8; padding: 0.45rem 0.65rem; text-align: left; font-size: 0.9rem; }\n    \
         th { background: #f5f8fc; color: #1b2d56; }\n    \
         pre { background: #f5f8fc; border: 1px solid #d7dee8; border-radius: 6px; padding: 0.9rem; white-space: pre-wrap; }\n  \
         </style>\n</head>\n<body>\n  <header>\n    <h1>CurveBook — Third-Party Software Licenses</h1>\n    \
         <p>Permitted crate SPDX allowlist from <code>deny.toml</code>: ",
    );
    let escaped_allow: Vec<String> = policy.allow_licenses.iter().map(|s| format!("<code>{}</code>", escape_html(s))).collect();
    out.push_str(&escaped_allow.join(", "));
    out.push_str(".</p>\n  </header>\n  <main>\n    ");
    out
}

fn append_notice_and_attribution(out: &mut String, notice_txt: &str, attribution_md: &str) {
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
}

fn collect_crate_license_texts(pkg: &ResolvedLicensePkg) -> Vec<String> {
    let Some(manifest_path) = pkg.manifest_path.as_deref() else {
        return Vec::new();
    };
    let Some(dir) = manifest_path.parent() else {
        return Vec::new();
    };
    let mut files = Vec::new();
    if let Some(rel) = pkg.license_file.as_deref() {
        let explicit = dir.join(rel);
        if explicit.is_file() {
            files.push(explicit);
        }
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_ascii_uppercase();
            if (name.starts_with("LICENSE") || name.starts_with("LICENCE") || name.starts_with("COPYING") || name.starts_with("NOTICE"))
                && !files.contains(&path)
            {
                files.push(path);
            }
        }
    }
    files.sort();
    let mut texts = Vec::new();
    for file in files {
        if let Ok(content) = std::fs::read_to_string(&file)
            && !content.trim().is_empty()
        {
            texts.push(content);
        }
    }
    texts
}

fn extract_spdx_ids(expr: &str) -> Vec<&str> {
    expr.split(|c: char| c.is_ascii_whitespace() || matches!(c, '(' | ')' | '/'))
        .filter(|tok| !tok.is_empty() && !matches!(*tok, "AND" | "OR" | "WITH"))
        .collect()
}

fn canonical_spdx_license_text(spdx_id: &str) -> Option<&'static str> {
    match spdx_id {
        "MIT" => Some(
            "Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the \"Software\"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:\n\nThe above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.\n\nTHE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.",
        ),
        "Apache-2.0" => Some(
            "Apache License, Version 2.0 (January 2004, http://www.apache.org/licenses/). Licensed under the Apache License, Version 2.0; you may not use this file except in compliance with the License. Unless required by applicable law or agreed to in writing, software distributed under the License is distributed on an \"AS IS\" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.",
        ),
        "BSD-2-Clause" => Some(
            "Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met: (1) Redistributions of source code must retain the copyright notice, this list of conditions and the following disclaimer; (2) Redistributions in binary form must reproduce the copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.",
        ),
        "BSD-3-Clause" => Some(
            "Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met: (1) Source redistributions must retain the copyright notice, conditions, and disclaimer; (2) Binary redistributions must reproduce them; (3) Neither the name of the copyright holder nor contributors may be used to endorse or promote products without specific prior written permission.",
        ),
        "ISC" => Some(
            "Permission to use, copy, modify, and/or distribute this software for any purpose with or without fee is hereby granted, provided that the above copyright notice and this permission notice appear in all copies. THE SOFTWARE IS PROVIDED \"AS IS\" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS SOFTWARE.",
        ),
        "Zlib" => Some(
            "This software is provided 'as-is', without any express or implied warranty. In no event will the authors be held liable for any damages arising from the use of this software. Permission is granted to anyone to use this software for any purpose, including commercial applications, and to alter it and redistribute it freely.",
        ),
        "Unicode-3.0" => Some(
            "UNICODE LICENSE V3: Permission is hereby granted, free of charge, to any person obtaining a copy of data files and any associated documentation or Unicode software, to deal in Data Files or Software without restriction, including the rights to use, copy, modify, merge, publish, distribute, and/or sell copies, provided that copyright notices and this permission notice appear with all copies.",
        ),
        "CC0-1.0" => Some(
            "CC0 1.0 Universal Public Domain Dedication: To the extent possible under law, the person who associated CC0 with this work has waived all copyright and related or neighboring rights to this work worldwide without warranty.",
        ),
        _ => None,
    }
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
name = \"curvebook-doc\"
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

        let html = render_licenses_html(&policy, &pkgs, "| a | <script> |", "Copyright CurveBook");
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("serde"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("Copyright CurveBook"));
    }

    #[test]
    fn parses_html_flag_with_or_without_explicit_path() {
        assert_eq!(parse_args(&[]).unwrap(), None);
        assert_eq!(parse_args(&[String::from("--html")]).unwrap(), Some(PathBuf::from("THIRD-PARTY-LICENSES.html")));
        assert_eq!(parse_args(&[String::from("--html"), String::from("dist/lic.html")]).unwrap(), Some(PathBuf::from("dist/lic.html")));
        assert_eq!(parse_args(&[String::from("--html=dist/lic.html")]).unwrap(), Some(PathBuf::from("dist/lic.html")));
        assert!(parse_args(&[String::from("--bogus")]).is_err());
    }

    #[test]
    fn r2_s13_01_08_spdx_boolean_policy_and_detailed_html_texts() {
        use super::{
            CANONICAL_ALLOWED_LICENSES, ResolvedLicensePkg, evaluate_spdx_expression, render_detailed_licenses_html, validate_deny_policy,
            verify_resolved_spdx_metadata,
        };
        let allow: Vec<String> = CANONICAL_ALLOWED_LICENSES.iter().map(|s| (*s).to_owned()).collect();

        evaluate_spdx_expression("MIT", &allow).unwrap();
        evaluate_spdx_expression("(MIT OR Apache-2.0) AND Unicode-3.0", &allow).unwrap();
        evaluate_spdx_expression("MIT OR GPL-3.0-only", &allow).unwrap();
        evaluate_spdx_expression("Apache-2.0/MIT", &allow).unwrap();
        evaluate_spdx_expression("Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT", &allow).unwrap();

        assert!(evaluate_spdx_expression("MIT AND GPL-3.0-only", &allow).is_err());
        assert!(evaluate_spdx_expression("(MIT OR Apache-2.0) AND BSL-1.1", &allow).is_err());
        assert!(evaluate_spdx_expression("MPL-2.0", &allow).is_err());
        assert!(evaluate_spdx_expression("NONE", &allow).is_err());

        let mut bad_policy = parse_deny_toml(include_str!("../../deny.toml")).unwrap();
        bad_policy.allow_licenses.push(String::from("MPL-2.0"));
        assert!(validate_deny_policy(&bad_policy).is_err());

        // Metadata with inactive optional BSL-1.0 crate passes, while active `MIT AND GPL-3.0-only` fails.
        let clean_meta: serde_json::Value = serde_json::from_str(
            r#"{
                "packages":[
                    {"id":"app 0.1.0","name":"app","version":"0.1.0","source":null,"license":"MIT OR Apache-2.0","dependencies":[{"name":"opt_bsl","optional":true}],"features":{"feat_bsl":["dep:opt_bsl"]}},
                    {"id":"opt_bsl 1.0.0","name":"opt_bsl","version":"1.0.0","source":"registry+https://github.com/rust-lang/crates.io-index","license":"BSL-1.0","dependencies":[]}
                ],
                "workspace_members":["app 0.1.0"],
                "resolve":{"nodes":[
                    {"id":"app 0.1.0","features":[],"deps":[{"name":"opt_bsl","pkg":"opt_bsl 1.0.0","dep_kinds":[{"kind":null}]}]},
                    {"id":"opt_bsl 1.0.0","features":[],"deps":[]}
                ]}
            }"#,
        )
        .unwrap();
        let resolved = verify_resolved_spdx_metadata(&clean_meta, &allow).unwrap();
        assert_eq!(resolved.len(), 1);

        let bad_meta: serde_json::Value = serde_json::from_str(
            r#"{
                "packages":[
                    {"id":"app 0.1.0","name":"app","version":"0.1.0","source":null,"license":"MIT","dependencies":[{"name":"bad","optional":false}]},
                    {"id":"bad 1.0.0","name":"bad","version":"1.0.0","source":"registry+https://github.com/rust-lang/crates.io-index","license":"MIT AND GPL-3.0-only","dependencies":[]}
                ],
                "workspace_members":["app 0.1.0"],
                "resolve":{"nodes":[
                    {"id":"app 0.1.0","features":[],"deps":[{"name":"bad","pkg":"bad 1.0.0","dep_kinds":[{"kind":null}]}]},
                    {"id":"bad 1.0.0","features":[],"deps":[]}
                ]}
            }"#,
        )
        .unwrap();
        let err = verify_resolved_spdx_metadata(&bad_meta, &allow).unwrap_err();
        assert!(err.contains("GPL-3.0-only"), "{err}");

        let policy = parse_deny_toml(include_str!("../../deny.toml")).unwrap();
        let html = render_detailed_licenses_html(
            &policy,
            &[
                ResolvedLicensePkg {
                    id: String::from("serde 1.0.0"),
                    name: String::from("serde"),
                    version: String::from("1.0.0"),
                    source: Some(String::from("registry+crates.io")),
                    license: String::from("MIT"),
                    license_file: None,
                    manifest_path: None,
                    is_shipped: true,
                },
                ResolvedLicensePkg {
                    id: String::from("proptest 1.0.0"),
                    name: String::from("proptest"),
                    version: String::from("1.0.0"),
                    source: Some(String::from("registry+crates.io")),
                    license: String::from("MIT"),
                    license_file: None,
                    manifest_path: None,
                    is_shipped: false,
                },
            ],
            "",
            "Notice text",
        );
        assert!(html.contains("serde"));
        assert!(!html.contains("proptest"), "dev-only crate must be excluded");
        assert!(html.contains("Permission is hereby granted, free of charge"), "full license text missing");

        let subtable_toml = "[target.'cfg(unix)'.dependencies.evil_dep]\nversion = \"*\"\n";
        let wild_hits = super::find_wildcard_dependencies("crates/x/Cargo.toml", subtable_toml);
        assert_eq!(wild_hits.len(), 1, "target subtable wildcard dependency must be flagged: {wild_hits:?}");
    }
}
