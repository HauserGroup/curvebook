//! `cargo xtask telemetry`: enforce CurveBook's zero-telemetry, zero-cloud, and local-only
//! networking policy across manifests, Rust sources, workflows, and web security headers (`R0.6`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::licenses::parse_cargo_lock;

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
    "web_sys::Navigator",
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
    for (idx, raw) in toml.lines().enumerate() {
        let line = strip_toml_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(sec) = parse_section_header(line) {
            section = sec.to_owned();
            if let Some(dep_name) = section_dependency_name(&section)
                && is_banned_crate_name(dep_name)
            {
                violations.push(TelemetryViolation {
                    check: 1,
                    path: rel.to_owned(),
                    line: idx + 1,
                    detail: format!("banned telemetry/HTTP crate `{dep_name}` in section `[{section}]`"),
                });
            }
            continue;
        }
        if !is_dependency_section(&section) {
            continue;
        }
        let Some((lhs, rhs)) = line.split_once('=') else {
            continue;
        };
        let key = unquote_toml_str(lhs.trim());
        let rhs = rhs.trim();
        if key != "package" && is_banned_crate_name(&key) {
            violations.push(TelemetryViolation {
                check: 1,
                path: rel.to_owned(),
                line: idx + 1,
                detail: format!("banned telemetry/HTTP crate `{key}` in `Cargo.toml`"),
            });
        }
        if let Some(pkg) = extract_inline_package_name(rhs)
            && is_banned_crate_name(&pkg)
        {
            violations.push(TelemetryViolation {
                check: 1,
                path: rel.to_owned(),
                line: idx + 1,
                detail: format!("banned telemetry/HTTP crate `{pkg}` (aliased via `package =`)"),
            });
        }
        let is_web_sys_entry = key == "web-sys" || section_dependency_name(&section) == Some("web-sys") && key == "features";
        if is_web_sys_entry {
            for feat in extract_banned_web_sys_features(rhs) {
                violations.push(TelemetryViolation {
                    check: 1,
                    path: rel.to_owned(),
                    line: idx + 1,
                    detail: format!("banned `web-sys` network feature `{feat}` in `Cargo.toml`"),
                });
            }
        }
    }
}

fn extract_banned_web_sys_features(rhs: &str) -> Vec<String> {
    let mut hits = Vec::new();
    let mut pos = 0usize;
    let bytes = rhs.as_bytes();
    while pos < bytes.len() {
        let q = bytes[pos];
        if (q == b'"' || q == b'\'')
            && let Some(end) = rhs[pos + 1..].find(char::from(q))
        {
            let val = &rhs[pos + 1..pos + 1 + end];
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

fn extract_inline_package_name(rhs: &str) -> Option<String> {
    if !rhs.starts_with('{') {
        return None;
    }
    let mut pos = 0;
    while let Some(rel) = rhs[pos..].find("package") {
        let hit = pos + rel;
        let left_ok = hit == 0 || !is_ident_byte(rhs.as_bytes()[hit - 1]);
        let after = &rhs[hit + "package".len()..];
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
    scan_code_patterns(rel, &stripped.prod_code, 2, ALWAYS_BANNED_NET_PATTERNS, "banned network socket primitive", violations);
    check_js_sys_network_strings(rel, stripped, violations);

    let allowed = ALLOWED_LOOPBACK_NET_FILES.contains(&rel);
    let mut scoped_hits = Vec::new();
    scan_code_patterns(rel, &stripped.prod_code, 2, LOOPBACK_SCOPED_NET_PATTERNS, "unapproved std::net / TCP usage", &mut scoped_hits);

    if !allowed {
        violations.extend(scoped_hits);
        return;
    }
    check_allowlisted_net_file(rel, stripped, &scoped_hits, violations);
}

fn check_js_sys_network_strings(rel: &str, stripped: &StrippedRust, violations: &mut Vec<TelemetryViolation>) {
    let uses_wasm_bridge =
        stripped.prod_code.contains("js_sys") || stripped.prod_code.contains("web_sys") || stripped.prod_code.contains("wasm_bindgen");
    if !uses_wasm_bridge {
        return;
    }
    const BANNED_JS_LITERALS: &[&str] = &["fetch", "sendbeacon", "websocket", "eventsource", "xmlhttprequest"];
    for (idx, line) in stripped.no_comments_code.lines().enumerate() {
        let mut pos = 0usize;
        let bytes = line.as_bytes();
        while pos < bytes.len() {
            if bytes[pos] == b'"'
                && let Some(end_rel) = line[pos + 1..].find('"')
            {
                let lit = line[pos + 1..pos + 1 + end_rel].trim().to_ascii_lowercase();
                if BANNED_JS_LITERALS.contains(&lit.as_str()) {
                    violations.push(TelemetryViolation {
                        check: 2,
                        path: rel.to_owned(),
                        line: idx + 1,
                        detail: format!("forbidden JS network reflection string `\"{lit}\"` in WASM source"),
                    });
                }
                pos = pos + 1 + end_rel + 1;
                continue;
            }
            pos += 1;
        }
    }
}

fn check_allowlisted_net_file(rel: &str, stripped: &StrippedRust, scoped_hits: &[TelemetryViolation], violations: &mut Vec<TelemetryViolation>) {
    check_net_alias_imports(rel, &stripped.prod_code, violations);

    let fn_spans = collect_rust_fn_spans(&stripped.prod_code);
    let mut checked_fn_sockets = 0usize;
    for (start, end, line_no) in fn_spans {
        let fn_prod = &stripped.prod_code[start..end];
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
    for (idx, line) in prod_code.lines().enumerate() {
        let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        if compact.contains("std::net::*") || compact.contains("std::netas") || (compact.contains("std::net") && line.contains(" as ")) {
            violations.push(TelemetryViolation {
                check: 2,
                path: rel.to_owned(),
                line: idx + 1,
                detail: String::from("aliasing or wildcard-importing `std::net` is forbidden in allowlisted files"),
            });
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
        let mut p = i + 2;
        while p < bytes.len() && bytes[p] != b'{' && bytes[p] != b';' {
            p += 1;
        }
        if bytes.get(p) != Some(&b'{') {
            i += 2;
            continue;
        }
        let mut depth = 1usize;
        p += 1;
        while p < bytes.len() && depth > 0 {
            match bytes[p] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            p += 1;
        }
        spans.push((fn_start, p, fn_line));
        i += 2;
    }
    spans
}

fn fn_opens_socket(fn_prod: &str) -> bool {
    fn_prod.contains("TcpListener::bind")
        || fn_prod.contains("TcpStream::connect")
        || ((fn_prod.contains("TcpListener") || fn_prod.contains("TcpStream") || fn_prod.contains("std::net"))
            && (fn_prod.contains("::bind(") || fn_prod.contains("::connect(") || fn_prod.contains("connect_timeout(")))
}

fn slice_has_loopback_guard(prod_code: &str, no_comments_code: &str) -> bool {
    let compact: String = prod_code.chars().filter(|c| !c.is_whitespace()).collect();
    if prod_code.contains("is_loopback")
        || prod_code.contains("validate_loopback_addr")
        || prod_code.contains("bind_control_tcp_listener")
        || compact.contains("[127,0,0,1]")
        || prod_code.contains("LOCALHOST")
    {
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
    for (offset, line) in fn_lits.lines().enumerate() {
        if line.contains("\"[::]\"") || line.contains("\"0.0.0.0") {
            return Some((base_line + offset, String::from(if line.contains("0.0.0.0") { "0.0.0.0" } else { "[::]" })));
        }
        let mut pos = 0usize;
        while let Some(q_rel) = line[pos..].find('"') {
            let start = pos + q_rel + 1;
            let Some(end_rel) = line[start..].find('"') else {
                break;
            };
            let lit = &line[start..start + end_rel];
            let host = lit.split(':').next().unwrap_or(lit);
            if is_ipv4_dotted_quad(host) && !host.starts_with("127.") {
                return Some((base_line + offset, lit.to_owned()));
            }
            pos = start + end_rel + 1;
        }
    }
    None
}

fn is_ipv4_dotted_quad(s: &str) -> bool {
    let mut parts = s.split('.');
    let mut count = 0usize;
    for part in &mut parts {
        if part.is_empty() || part.parse::<u8>().is_err() {
            return false;
        }
        count += 1;
    }
    count == 4
}

fn scan_code_patterns(rel: &str, code: &str, check: u8, patterns: &[&'static str], reason: &str, violations: &mut Vec<TelemetryViolation>) {
    for (idx, line) in code.lines().enumerate() {
        let mut pos = 0usize;
        while pos < line.len() {
            if let Some((hit_pos, pat)) = next_ident_pattern_match(line, pos, patterns) {
                violations.push(TelemetryViolation { check, path: rel.to_owned(), line: idx + 1, detail: format!("{reason} `{pat}`") });
                pos = hit_pos + pat.len();
            } else {
                break;
            }
        }
    }
}

fn collect_non_test_src_rs_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
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
            collect_non_test_src_rs_files(root, &path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
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

/// Check 3: scan `.github/workflows/*.yml`, `apps/curvebook-web/**`, and `packaging/**` for
/// telemetry/analytics/cloud-service markers and external URLs / browser network APIs across web assets.
fn check_web_and_workflow_telemetry(root: &Path, violations: &mut Vec<TelemetryViolation>) -> Result<usize, String> {
    let mut targets = collect_workflow_files(root)?;
    for dir_name in ["apps/curvebook-web", "packaging"] {
        let dir = root.join(dir_name);
        if dir.is_dir() {
            collect_all_files_under(root, &dir, &mut targets)?;
        }
    }
    targets.sort();
    targets.dedup();

    let mut scanned = 0usize;
    let mut web_asset_files = Vec::new();
    for rel in &targets {
        let bytes = std::fs::read(root.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
        if bytes.contains(&0) {
            continue;
        }
        scanned += 1;
        let text = String::from_utf8_lossy(&bytes);
        scan_text_for_web_telemetry(rel, &text, violations);
        if is_web_asset_file(rel) {
            web_asset_files.push((rel.clone(), text.into_owned()));
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

fn is_web_asset_file(rel: &str) -> bool {
    let in_web = rel.starts_with("apps/curvebook-web/") || rel.starts_with("packaging/web/");
    if !in_web {
        return false;
    }
    matches!(
        Path::new(rel).extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()).as_deref(),
        Some("html" | "htm" | "js" | "mjs" | "ts" | "css")
    )
}

fn is_browser_markup_or_script(rel: &str) -> bool {
    matches!(
        Path::new(rel).extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()).as_deref(),
        Some("html" | "htm" | "js" | "mjs" | "ts" | "jsx" | "tsx")
    )
}

fn scan_text_for_web_telemetry(rel: &str, text: &str, violations: &mut Vec<TelemetryViolation>) {
    let check_fetch = is_browser_markup_or_script(rel);
    for (idx, line) in text.lines().enumerate() {
        let lower = line.to_ascii_lowercase();
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
    while let Some(rel) = lower_line[start..].find(term) {
        let pos = start + rel;
        let end = pos + term.len();
        let left_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
        let right_needs_boundary = term.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric);
        let right_ok = !right_needs_boundary || end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if left_ok && right_ok {
            return true;
        }
        start = pos + 1;
    }
    false
}

fn contains_browser_fetch_call(lower_line: &str) -> bool {
    let bytes = lower_line.as_bytes();
    let needle = "fetch";
    let mut start = 0usize;
    while let Some(rel) = lower_line[start..].find(needle) {
        let pos = start + rel;
        let after_ws = lower_line[pos + needle.len()..].trim_start();
        let followed_by_paren = after_ws.starts_with('(');
        let left_is_ok = pos == 0
            || (bytes[pos - 1] != b'.' && !is_ident_byte(bytes[pos - 1]))
            || (bytes[pos - 1] == b'.' && is_global_fetch_receiver(&lower_line[..pos]));
        if followed_by_paren && left_is_ok {
            return true;
        }
        start = pos + 1;
    }
    false
}

fn is_global_fetch_receiver(before_fetch: &str) -> bool {
    ["window.", "globalthis.", "self."].iter().any(|recv| {
        if let Some(prefix) = before_fetch.strip_suffix(recv) {
            prefix.is_empty() || !is_ident_byte(prefix.as_bytes()[prefix.len() - 1])
        } else {
            false
        }
    })
}

fn scan_web_asset_external_urls_and_apis(rel: &str, content: &str, violations: &mut Vec<TelemetryViolation>) {
    let is_css = rel.ends_with(".css");
    for (idx, line) in content.lines().enumerate() {
        let lower = line.to_ascii_lowercase();
        for scheme in ["https://", "http://", "wss://", "ws://"] {
            let mut start = 0usize;
            while let Some(offset) = lower[start..].find(scheme) {
                let pos = start + offset;
                let rest = &line[pos..];
                let end = rest.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | ';' | '`' | ',')).unwrap_or(rest.len());
                let url = &rest[..end];
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
        scan_protocol_relative_urls(rel, idx + 1, line, violations);
        if !is_css {
            scan_web_network_api_tokens(rel, idx + 1, &lower, violations);
        }
    }
}

fn scan_protocol_relative_urls(rel: &str, line_no: usize, line: &str, violations: &mut Vec<TelemetryViolation>) {
    let bytes = line.as_bytes();
    let mut pos = 0usize;
    while let Some(offset) = line[pos..].find("//") {
        let hit = pos + offset;
        let prev_ok = hit > 0 && matches!(bytes[hit - 1], b'=' | b'"' | b'\'' | b'`' | b'(');
        let after = &line[hit + 2..];
        let end = after.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | ';' | '`' | ',')).unwrap_or(after.len());
        let target = &after[..end];
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
    for api in ["websocket", "eventsource", "sharedworker", "serviceworker", "worker("] {
        let mut start = 0usize;
        while let Some(offset) = lower[start..].find(api) {
            let pos = start + offset;
            let bytes = lower.as_bytes();
            let left_ok = pos == 0 || !is_ident_byte(bytes[pos - 1]);
            let right_ok = api.ends_with('(') || pos + api.len() == bytes.len() || !is_ident_byte(bytes[pos + api.len()]);
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
}

/// Check 4: GitHub Actions workflow permissions and `run:` script-injection hygiene.
fn check_workflow_security(root: &Path, violations: &mut Vec<TelemetryViolation>) -> Result<usize, String> {
    let workflows = collect_workflow_files(root)?;
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
        audit_workflow_yaml(rel, &text, violations);
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
    run_block_col: Option<usize>,
}

fn audit_workflow_yaml(rel: &str, yaml: &str, violations: &mut Vec<TelemetryViolation>) {
    let mut state = WorkflowParseState::default();
    for (idx, raw) in yaml.lines().enumerate() {
        let line_no = idx + 1;
        if let Some(run_col) = state.run_block_col {
            if raw.trim().is_empty() || leading_spaces(raw) > run_col {
                flag_untrusted_event_expr(rel, line_no, raw, violations);
                flag_remote_script_execution(rel, line_no, raw, violations);
                continue;
            }
            state.run_block_col = None;
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
        check_run_directive(rel, line_no, code, &mut state, violations);
    }

    if !state.saw_top_permissions || !state.top_has_contents_read {
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
        state.in_jobs = trimmed == "jobs:";
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
            && let Some(job_key) = trimmed.strip_suffix(':')
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
    if let Some(rest) = trimmed.strip_prefix("permissions:") {
        let rhs = rest.trim();
        let is_top = indent == 0;
        if is_top {
            state.saw_top_permissions = true;
            state.top_perm_indent = Some(indent);
        } else {
            state.job_perm_indent = Some(indent);
        }
        if rhs == "write-all" {
            violations.push(TelemetryViolation {
                check: 4,
                path: rel.to_owned(),
                line: line_no,
                detail: String::from("`permissions: write-all` is forbidden"),
            });
            return true;
        }
        if rhs.starts_with('{') && rhs.ends_with('}') {
            let inner = &rhs[1..rhs.len() - 1];
            for entry in inner.split(',') {
                if let Some((scope, level)) = entry.split_once(':') {
                    validate_permission_entry(rel, line_no, scope.trim(), level.trim(), is_top, state, violations);
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
        && let Some((scope, level)) = trimmed.split_once(':')
    {
        validate_permission_entry(rel, line_no, scope.trim(), level.trim(), true, state, violations);
        return true;
    }

    if state.job_perm_indent.is_some_and(|p| indent > p)
        && let Some((scope, level)) = trimmed.split_once(':')
    {
        validate_permission_entry(rel, line_no, scope.trim(), level.trim(), false, state, violations);
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
    let clean_level = unquote_yaml_scalar(level);
    if is_top && scope == "contents" && clean_level == "read" {
        state.top_has_contents_read = true;
    }
    if clean_level == "write" {
        let publish_ok = !is_top && rel == ".github/workflows/release.yml" && state.current_job.as_deref() == Some("publish") && scope == "contents";
        if !publish_ok {
            violations.push(TelemetryViolation {
                check: 4,
                path: rel.to_owned(),
                line: line_no,
                detail: format!("unexpected write permission `{scope}: write` (only release.yml `publish` job may declare `contents: write`)"),
            });
        }
    }
}

fn check_workflow_uses_sha(rel: &str, line_no: usize, code: &str, violations: &mut Vec<TelemetryViolation>) {
    let trimmed = code.trim_start();
    let after_dash = trimmed.strip_prefix('-').map_or(trimmed, str::trim_start);
    let Some(rhs) = after_dash.strip_prefix("uses:") else {
        return;
    };
    let spec = unquote_yaml_scalar(rhs.trim());
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

fn check_run_directive(rel: &str, line_no: usize, code: &str, state: &mut WorkflowParseState, violations: &mut Vec<TelemetryViolation>) {
    let Some((key_col, key_name, rhs)) = split_yaml_script_key(code) else {
        return;
    };
    let trimmed_rhs = rhs.trim();
    if is_yaml_block_scalar(trimmed_rhs) {
        state.run_block_col = Some(key_col);
    } else if let Some(inner) = trimmed_rhs.strip_prefix("run:") {
        let val = inner.trim();
        if is_yaml_block_scalar(val) {
            state.run_block_col = Some(key_col);
        } else {
            flag_untrusted_event_expr(rel, line_no, val, violations);
            flag_remote_script_execution(rel, line_no, val, violations);
        }
    } else {
        let _ = key_name;
        flag_untrusted_event_expr(rel, line_no, trimmed_rhs, violations);
        flag_remote_script_execution(rel, line_no, trimmed_rhs, violations);
    }
}

fn split_yaml_script_key(code: &str) -> Option<(usize, &'static str, &str)> {
    let indent = leading_spaces(code);
    let after_indent = &code[indent..];
    let (key_col, after_dash) = if let Some(rest) = after_indent.strip_prefix('-') {
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            return None;
        }
        let extra = leading_spaces(rest);
        (indent + 1 + extra, &rest[extra..])
    } else {
        (indent, after_indent)
    };
    for key in ["run:", "script:", "prepare:", "with:"] {
        if let Some(rhs) = after_dash.strip_prefix(key) {
            if key == "with:" && !rhs.trim_start().starts_with("run:") {
                return None;
            }
            return Some((key_col, key, rhs));
        }
    }
    None
}

fn is_yaml_block_scalar(rhs: &str) -> bool {
    let Some(first) = rhs.chars().next() else {
        return false;
    };
    (first == '|' || first == '>') && rhs[1..].chars().all(|c| matches!(c, '-' | '+' | '0'..='9'))
}

fn flag_untrusted_event_expr(rel: &str, line_no: usize, text: &str, violations: &mut Vec<TelemetryViolation>) {
    let mut pos = 0usize;
    while let Some(start_rel) = text[pos..].find("${{") {
        let expr_start = pos + start_rel + 3;
        let expr_end = text[expr_start..].find("}}").map_or(text.len(), |end_rel| expr_start + end_rel);
        let expr = &text[expr_start..expr_end];
        let lower_compact: String = expr.chars().filter(|c| !c.is_whitespace()).map(|c| c.to_ascii_lowercase()).collect();
        if let Some(ref_kind) = untrusted_expr_kind(&lower_compact) {
            violations.push(TelemetryViolation {
                check: 4,
                path: rel.to_owned(),
                line: line_no,
                detail: format!("untrusted `${{{{ {ref_kind} }}}}` interpolated directly in workflow script block (use `env:` instead)"),
            });
        }
        pos = (expr_end + 2).min(text.len());
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
    while let Some(rel) = lower_compact[pos..].find(needle) {
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
        let map = parse_netlify_global_headers(&text);
        validate_required_web_headers(headers_rel, &map, violations);
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
        let map = parse_htaccess_headers(&text);
        validate_required_web_headers(htaccess_rel, &map, violations);
    }

    Ok(count)
}

fn parse_netlify_global_headers(text: &str) -> BTreeMap<String, (usize, String)> {
    let mut map = BTreeMap::new();
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
        if in_global && let Some((name, value)) = trimmed.split_once(':') {
            map.insert(name.trim().to_ascii_lowercase(), (idx + 1, value.trim().to_owned()));
        }
    }
    map
}

fn parse_htaccess_headers(text: &str) -> BTreeMap<String, (usize, String)> {
    let mut map = BTreeMap::new();
    for (idx, raw) in text.lines().enumerate() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((name, value)) = parse_apache_header_set(trimmed) {
            map.insert(name.to_ascii_lowercase(), (idx + 1, value));
        }
    }
    map
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
    if !tok.0.eq_ignore_ascii_case("set") && !tok.0.eq_ignore_ascii_case("setifempty") {
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
    match trimmed.find(char::is_whitespace) {
        Some(idx) => Some((&trimmed[..idx], &trimmed[idx..])),
        None => Some((trimmed, "")),
    }
}

fn unquote_apache_value(val: &str) -> String {
    if val.len() >= 2 && ((val.starts_with('"') && val.ends_with('"')) || (val.starts_with('\'') && val.ends_with('\''))) {
        val[1..val.len() - 1].to_owned()
    } else {
        val.to_owned()
    }
}

fn validate_required_web_headers(rel: &str, headers: &BTreeMap<String, (usize, String)>, violations: &mut Vec<TelemetryViolation>) {
    match headers.get("content-security-policy") {
        None => violations.push(TelemetryViolation {
            check: 5,
            path: rel.to_owned(),
            line: 1,
            detail: String::from("missing `Content-Security-Policy` header"),
        }),
        Some((line, csp)) => validate_csp_directives(rel, *line, csp, violations),
    }

    match headers.get("referrer-policy") {
        Some((line, val)) if val.trim().eq_ignore_ascii_case("no-referrer") => {
            let _ = line;
        }
        Some((line, val)) => violations.push(TelemetryViolation {
            check: 5,
            path: rel.to_owned(),
            line: *line,
            detail: format!("`Referrer-Policy` must be `no-referrer`, found `{val}`"),
        }),
        None => violations.push(TelemetryViolation {
            check: 5,
            path: rel.to_owned(),
            line: 1,
            detail: String::from("missing `Referrer-Policy: no-referrer` header"),
        }),
    }

    match headers.get("permissions-policy") {
        None => violations.push(TelemetryViolation {
            check: 5,
            path: rel.to_owned(),
            line: 1,
            detail: String::from("missing `Permissions-Policy` header"),
        }),
        Some((line, val)) => {
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
    }

    match headers.get("x-content-type-options") {
        Some((line, val)) if val.trim().eq_ignore_ascii_case("nosniff") => {
            let _ = line;
        }
        Some((line, val)) => violations.push(TelemetryViolation {
            check: 5,
            path: rel.to_owned(),
            line: *line,
            detail: format!("`X-Content-Type-Options` must be `nosniff`, found `{val}`"),
        }),
        None => violations.push(TelemetryViolation {
            check: 5,
            path: rel.to_owned(),
            line: 1,
            detail: String::from("missing `X-Content-Type-Options: nosniff` header"),
        }),
    }

    validate_exact_header(rel, headers, "cross-origin-opener-policy", "Cross-Origin-Opener-Policy", "same-origin", violations);
    validate_exact_header(rel, headers, "cross-origin-embedder-policy", "Cross-Origin-Embedder-Policy", "require-corp", violations);
}

fn validate_exact_header(
    rel: &str,
    headers: &BTreeMap<String, (usize, String)>,
    key: &str,
    display_name: &str,
    expected: &str,
    violations: &mut Vec<TelemetryViolation>,
) {
    match headers.get(key) {
        Some((line, val)) if val.trim().eq_ignore_ascii_case(expected) => {
            let _ = line;
        }
        Some((line, val)) => violations.push(TelemetryViolation {
            check: 5,
            path: rel.to_owned(),
            line: *line,
            detail: format!("`{display_name}` must be `{expected}`, found `{val}`"),
        }),
        None => violations.push(TelemetryViolation {
            check: 5,
            path: rel.to_owned(),
            line: 1,
            detail: format!("missing `{display_name}: {expected}` header"),
        }),
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
        for tok in dir.split_whitespace().skip(1) {
            if tok.contains("://") || tok.starts_with("//") || tok == "*" {
                violations.push(TelemetryViolation {
                    check: 5,
                    path: rel.to_owned(),
                    line,
                    detail: format!("`Content-Security-Policy` forbids external origin `{tok}` in `{dir}`"),
                });
            }
        }
    }
}

fn collect_workflow_files(root: &Path) -> Result<Vec<String>, String> {
    let wf_dir = root.join(".github").join("workflows");
    if !wf_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&wf_dir).map_err(|e| format!("read_dir .github/workflows: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry .github/workflows: {e}"))?;
        let path = entry.path();
        if path.is_file()
            && path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("yml") || e.eq_ignore_ascii_case("yaml"))
        {
            out.push(rel_slash(root, &path));
        }
    }
    out.sort();
    Ok(out)
}

fn collect_files_named(dir: &Path, target_name: &str, out: &mut Vec<PathBuf>) -> Result<(), String> {
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
            collect_files_named(&path, target_name, out)?;
        } else if name_str == target_name {
            out.push(path);
        }
    }
    Ok(())
}

fn collect_all_files_under(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry {}: {e}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if path.is_dir() {
            if name_str == "target" || name_str == "dist" || name_str == ".git" {
                continue;
            }
            collect_all_files_under(root, &path, out)?;
        } else if path.is_file() {
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
        while let Some(rel) = line[search..].find(pat) {
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
            search = pos + 1;
        }
    }
    best
}

const fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn leading_spaces(s: &str) -> usize {
    s.bytes().take_while(|&b| b == b' ').count()
}

fn strip_yaml_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_dq = false;
    let mut in_sq = false;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if in_dq => {
                i += 2;
                continue;
            }
            b'"' if !in_sq => in_dq = !in_dq,
            b'\'' if !in_dq => in_sq = !in_sq,
            b'#' if !in_dq && !in_sq && (i == 0 || bytes[i - 1].is_ascii_whitespace()) => return &line[..i],
            _ => {}
        }
        i += 1;
    }
    line
}

fn unquote_yaml_scalar(s: &str) -> &str {
    let t = s.trim();
    if t.len() >= 2 && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\''))) { &t[1..t.len() - 1] } else { t }
}

fn strip_toml_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_dq = false;
    let mut in_sq = false;
    let mut i = 0usize;
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

struct StrippedRust {
    prod_code: String,
    no_comments_code: String,
}

fn strip_non_prod_rust(src: &str) -> StrippedRust {
    let (blanked_all, blanked_comments_only) = blank_comments_and_literals(src);
    let mut prod_bytes = blanked_all.into_bytes();
    let mut comment_only_bytes = blanked_comments_only.into_bytes();
    mask_cfg_test_items(&prod_bytes.clone(), &mut [&mut prod_bytes, &mut comment_only_bytes]);
    StrippedRust {
        prod_code: String::from_utf8(prod_bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
        no_comments_code: String::from_utf8(comment_only_bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
    }
}

fn blank_comments_and_literals(src: &str) -> (String, String) {
    let bytes = src.as_bytes();
    let mut prod = Vec::with_capacity(bytes.len());
    let mut keep_lits = Vec::with_capacity(bytes.len());
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/') {
            skip_line_comment(bytes, &mut i, &mut prod, &mut keep_lits);
        } else if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            skip_block_comment(bytes, &mut i, &mut prod, &mut keep_lits);
        } else if try_skip_raw_string(bytes, &mut i, &mut prod, &mut keep_lits)
            || try_skip_char_literal(src, bytes, &mut i, &mut prod, &mut keep_lits)
        {
            continue;
        } else if (bytes[i] == b'b' || bytes[i] == b'c') && bytes.get(i + 1) == Some(&b'"') && !prev_is_ident(bytes, i) {
            prod.push(bytes[i]);
            keep_lits.push(bytes[i]);
            i += 1;
            skip_quoted_string(bytes, &mut i, &mut prod, &mut keep_lits);
        } else if bytes[i] == b'"' {
            skip_quoted_string(bytes, &mut i, &mut prod, &mut keep_lits);
        } else {
            prod.push(bytes[i]);
            keep_lits.push(bytes[i]);
            i += 1;
        }
    }
    let prod_str = String::from_utf8(prod).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
    let lits_str = String::from_utf8(keep_lits).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
    (prod_str, lits_str)
}

fn prev_is_ident(bytes: &[u8], i: usize) -> bool {
    i > 0 && is_ident_byte(bytes[i - 1])
}

fn skip_line_comment(bytes: &[u8], i: &mut usize, prod: &mut Vec<u8>, keep_lits: &mut Vec<u8>) {
    while *i < bytes.len() {
        let b = bytes[*i];
        *i += 1;
        let out_b = if b == b'\n' { b'\n' } else { b' ' };
        prod.push(out_b);
        keep_lits.push(out_b);
        if b == b'\n' {
            break;
        }
    }
}

fn skip_block_comment(bytes: &[u8], i: &mut usize, prod: &mut Vec<u8>, keep_lits: &mut Vec<u8>) {
    let mut depth = 1usize;
    *i += 2;
    prod.extend_from_slice(b"  ");
    keep_lits.extend_from_slice(b"  ");
    while *i < bytes.len() && depth > 0 {
        if bytes[*i] == b'/' && bytes.get(*i + 1) == Some(&b'*') {
            depth += 1;
            *i += 2;
            prod.extend_from_slice(b"  ");
            keep_lits.extend_from_slice(b"  ");
        } else if bytes[*i] == b'*' && bytes.get(*i + 1) == Some(&b'/') {
            depth -= 1;
            *i += 2;
            prod.extend_from_slice(b"  ");
            keep_lits.extend_from_slice(b"  ");
        } else {
            let out_b = if bytes[*i] == b'\n' { b'\n' } else { b' ' };
            prod.push(out_b);
            keep_lits.push(out_b);
            *i += 1;
        }
    }
}

fn try_skip_raw_string(bytes: &[u8], i: &mut usize, prod: &mut Vec<u8>, keep_lits: &mut Vec<u8>) -> bool {
    if prev_is_ident(bytes, *i) {
        return false;
    }
    let r_pos = if bytes[*i] == b'r' {
        *i
    } else if (bytes[*i] == b'b' || bytes[*i] == b'c') && bytes.get(*i + 1) == Some(&b'r') {
        *i + 1
    } else {
        return false;
    };
    let mut hashes = 0usize;
    while bytes.get(r_pos + 1 + hashes) == Some(&b'#') {
        hashes += 1;
    }
    let open_quote = r_pos + 1 + hashes;
    if bytes.get(open_quote) != Some(&b'"') {
        return false;
    }
    let prefix_end = open_quote + 1;
    prod.extend_from_slice(&bytes[*i..prefix_end]);
    keep_lits.extend_from_slice(&bytes[*i..prefix_end]);
    *i = prefix_end;
    while *i < bytes.len() {
        if bytes[*i] == b'"' && bytes.get(*i + 1..*i + 1 + hashes).is_some_and(|s| s.iter().all(|&b| b == b'#')) {
            prod.extend_from_slice(&bytes[*i..*i + 1 + hashes]);
            keep_lits.extend_from_slice(&bytes[*i..*i + 1 + hashes]);
            *i += 1 + hashes;
            break;
        }
        prod.push(if bytes[*i] == b'\n' { b'\n' } else { b' ' });
        keep_lits.push(bytes[*i]);
        *i += 1;
    }
    true
}

fn skip_quoted_string(bytes: &[u8], i: &mut usize, prod: &mut Vec<u8>, keep_lits: &mut Vec<u8>) {
    prod.push(b'"');
    keep_lits.push(b'"');
    *i += 1;
    while *i < bytes.len() {
        let b = bytes[*i];
        if b == b'\\' {
            prod.push(b' ');
            keep_lits.push(b);
            *i += 1;
            if *i < bytes.len() {
                prod.push(if bytes[*i] == b'\n' { b'\n' } else { b' ' });
                keep_lits.push(bytes[*i]);
                *i += 1;
            }
            continue;
        }
        if b == b'"' {
            prod.push(b'"');
            keep_lits.push(b'"');
            *i += 1;
            break;
        }
        prod.push(if b == b'\n' { b'\n' } else { b' ' });
        keep_lits.push(b);
        *i += 1;
    }
}

fn try_skip_char_literal(src: &str, bytes: &[u8], i: &mut usize, prod: &mut Vec<u8>, keep_lits: &mut Vec<u8>) -> bool {
    let (prefix_len, q_pos) = if bytes[*i] == b'b' && bytes.get(*i + 1) == Some(&b'\'') && !prev_is_ident(bytes, *i) {
        (2usize, *i + 1)
    } else if bytes[*i] == b'\'' {
        (1usize, *i)
    } else {
        return false;
    };
    let close = if bytes.get(q_pos + 1) == Some(&b'\\') {
        find_escaped_char_end(bytes, q_pos + 2)
    } else if let Some(rest) = src.get(q_pos + 1..)
        && let Some(ch) = rest.chars().next().filter(|&c| c != '\'' && c != '\n' && c != '\r')
        && bytes.get(q_pos + 1 + ch.len_utf8()) == Some(&b'\'')
    {
        Some(q_pos + 1 + ch.len_utf8())
    } else {
        None
    };

    let Some(end_q) = close else {
        return false;
    };
    for &b in &bytes[*i..*i + prefix_len] {
        prod.push(b'\'');
        keep_lits.push(b);
    }
    for &b in &bytes[(q_pos + 1)..end_q] {
        prod.push(b' ');
        keep_lits.push(b);
    }
    prod.push(b'\'');
    keep_lits.push(b'\'');
    *i = end_q + 1;
    true
}

fn find_escaped_char_end(bytes: &[u8], mut p: usize) -> Option<usize> {
    if p >= bytes.len() || bytes[p] == b'\n' {
        return None;
    }
    p += 1;
    let limit = (p + 14).min(bytes.len());
    while p < limit {
        if bytes[p] == b'\n' {
            return None;
        }
        if bytes[p] == b'\'' {
            return Some(p);
        }
        p += 1;
    }
    None
}

fn mask_cfg_test_items(code_bytes: &[u8], targets: &mut [&mut Vec<u8>]) {
    let mut i = 0usize;
    while i < code_bytes.len() {
        if code_bytes[i] == b'#'
            && let Some(after_attr) = crate::layers::match_cfg_test_attr(code_bytes, i)
        {
            let body_start = skip_following_attrs(code_bytes, after_attr);
            let item_end = find_item_end(code_bytes, body_start);
            for target in targets.iter_mut() {
                for b in target.iter_mut().take(item_end).skip(i) {
                    if *b != b'\n' {
                        *b = b' ';
                    }
                }
            }
            i = item_end;
            continue;
        }
        i += 1;
    }
}

fn skip_following_attrs(bytes: &[u8], mut pos: usize) -> usize {
    loop {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if bytes.get(pos) == Some(&b'#') && bytes.get(pos + 1) == Some(&b'[') {
            pos += 2;
            let mut depth = 1usize;
            while pos < bytes.len() && depth > 0 {
                match bytes[pos] {
                    b'[' => depth += 1,
                    b']' => depth -= 1,
                    _ => {}
                }
                pos += 1;
            }
        } else {
            return pos;
        }
    }
}

fn find_item_end(bytes: &[u8], mut pos: usize) -> usize {
    let mut brace_depth = 0usize;
    let mut saw_brace = false;
    while pos < bytes.len() {
        match bytes[pos] {
            b'{' => {
                brace_depth += 1;
                saw_brace = true;
            }
            b'}' if saw_brace => {
                brace_depth = brace_depth.saturating_sub(1);
                if brace_depth == 0 {
                    return pos + 1;
                }
            }
            b';' if !saw_brace => return pos + 1,
            _ => {}
        }
        pos += 1;
    }
    bytes.len()
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
}
