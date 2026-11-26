//! `cargo xtask telemetry`: enforce CurveBook's zero-telemetry, zero-cloud, and local-only
//! networking policy across manifests, Rust sources, workflows, and web security headers (`R0.6`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::rust_lex::{
    RustLexer, StrippedRust, find_fn_body_braces, is_cfg_test_only_submodule, is_ident_byte, strip_non_prod_rust_both as strip_non_prod_rust,
};
use crate::scan_util::{
    TomlEntry, TomlVal, format_error_list, has_ext_ci, is_real_dir, lower_ext, next_ident_pattern_match, parse_toml_entries, rel_slash_lossy,
    strip_hash_comment, unquote_scalar, walk_file_paths, walk_rel_files,
};

const ALLOWED_LOOPBACK_NET_FILES: &[&str] = &["crates/ui-egui/src/control_io.rs", "crates/mcp/src/tools.rs", "apps/curvebook/src/main.rs"];
const ALWAYS_BANNED_NET_PATTERNS: &str = "UdpSocket ToSocketAddrs core::net UnixStream UnixListener UnixDatagram web_sys::WebSocket web_sys::EventSource web_sys::XmlHttpRequest web_sys::Request web_sys::RequestInit web_sys::RequestMode web_sys::Response web_sys::Headers web_sys::Navigator web_sys::Worker web_sys::SharedWorker web_sys::ServiceWorkerContainer fetch_with_str fetch_with_request fetch_with_request_and_init send_beacon send_with_str send_with_u8_array";
const LOOPBACK_SCOPED_NET_PATTERNS: &[&str] = &["TcpStream", "TcpListener", "std::net"];
const BANNED_WEB_SYS_NET_FEATURES: &str =
    "websocket eventsource xmlhttprequest request requestinit requestmode response headers navigator worker sharedworker serviceworker beacon";
const BANNED_WEB_AND_WORKFLOW_TERMS: &str = "sonarcloud|sonarsource|sonar_token|codecov|coveralls|googletagmanager|google-analytics|gtag(|plausible|posthog|mixpanel|amplitude|sentry|bugsnag|datadog|clarity.ms|hotjar|navigator.sendbeacon|sendbeacon|xmlhttprequest|new websocket|eventsource|new worker|new sharedworker|serviceworker|new image";
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

fn words_has(list: &str, needle: &str) -> bool {
    list.split_ascii_whitespace().any(|w| w == needle)
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

pub fn run_checks(root: &Path) -> Result<(TelemetryStats, Vec<TelemetryViolation>), String> {
    let (mut stats, mut violations) = (TelemetryStats::default(), Vec::new());
    stats.manifests = check_banned_crates(root, &mut violations, &mut stats.lock_packages)?;
    stats.rust_sources = check_rust_network_primitives(root, &mut violations)?;
    stats.web_and_workflow_files = check_web_and_workflow_telemetry(root, &mut violations)?;
    stats.workflows = check_workflow_security(root, &mut violations)?;
    stats.header_configs = check_web_security_headers(root, &mut violations)?;
    violations.sort_by(|a, b| a.check.cmp(&b.check).then_with(|| a.path.cmp(&b.path)).then_with(|| a.line.cmp(&b.line)));
    Ok((stats, violations))
}

// ---------- Check 1: Banned crates & web-sys network features ----------

fn check_banned_crates(root: &Path, violations: &mut Vec<TelemetryViolation>, lock_count: &mut usize) -> Result<usize, String> {
    let mut manifests = Vec::new();
    walk_file_paths(root, &skip_workspace_dir, &|p| p.file_name().is_some_and(|n| n == "Cargo.toml"), &mut manifests)?;
    manifests.sort();
    if manifests.is_empty() {
        return Err(String::from("no Cargo.toml manifests found in workspace"));
    }
    let ws_aliases: BTreeSet<String> = if root.join("Cargo.toml").is_file() {
        let txt = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| format!("Cargo.toml: {e}"))?;
        parse_toml_entries(&txt)
            .into_iter()
            .filter_map(|e| match e {
                TomlEntry::Leaf { path, value, .. }
                    if path.len() >= 4
                        && path[0] == "workspace"
                        && path[1] == "dependencies"
                        && path[3] == "package"
                        && value.as_str().is_some_and(|p| norm_crate_ident(p) == "web-sys") =>
                {
                    Some(norm_crate_ident(&path[2]))
                }
                _ => None,
            })
            .collect()
    } else {
        BTreeSet::new()
    };
    for manifest in &manifests {
        let rel = rel_slash_lossy(root, manifest);
        let text = std::fs::read_to_string(manifest).map_err(|e| format!("{rel}: {e}"))?;
        scan_manifest_for_banned_crates(&rel, &text, &ws_aliases, violations);
    }
    let lock_text = std::fs::read_to_string(root.join("Cargo.lock")).map_err(|e| format!("Cargo.lock: {e}"))?;
    *lock_count = scan_lock_for_banned_crates(&lock_text, violations);
    Ok(manifests.len())
}

fn norm_crate_ident(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace('_', "-")
}

fn dep_path_parts(path: &[String]) -> Option<(&str, Option<&str>)> {
    let is_dep = |s: &str| matches!(s, "dependencies" | "dev-dependencies" | "build-dependencies");
    match path {
        [d, dep] if is_dep(d) => Some((dep, None)),
        [d, dep, sub, ..] if is_dep(d) => Some((dep, Some(sub))),
        [w, d, dep] if w == "workspace" && d == "dependencies" => Some((dep, None)),
        [w, d, dep, sub, ..] if w == "workspace" && d == "dependencies" => Some((dep, Some(sub))),
        [t, _, d, dep] if t == "target" && is_dep(d) => Some((dep, None)),
        [t, _, d, dep, sub, ..] if t == "target" && is_dep(d) => Some((dep, Some(sub))),
        _ => None,
    }
}

fn scan_manifest_for_banned_crates(rel: &str, toml: &str, ws_aliases: &BTreeSet<String>, violations: &mut Vec<TelemetryViolation>) {
    let entries = parse_toml_entries(toml);
    let (mut dep_pkg, mut local_ws, mut local_inh, mut local_other) = (BTreeMap::new(), BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for entry in &entries {
        if let TomlEntry::Leaf { path, value, .. } = entry
            && let Some((dep, Some(sub))) = dep_path_parts(path)
        {
            let dn = norm_crate_ident(dep);
            if sub == "package"
                && let Some(pkg) = value.as_str()
            {
                let pn = norm_crate_ident(pkg);
                dep_pkg.insert(path[..path.len() - 1].to_vec(), pn.clone());
                (if pn == "web-sys" { &mut local_ws } else { &mut local_other }).insert(dn);
            } else if sub == "workspace" && matches!(value, TomlVal::Other(s) if s == "true") {
                local_inh.insert(dn);
            }
        }
    }
    let mut web_sys_aliases: BTreeSet<String> = ws_aliases.union(&local_ws).cloned().collect();
    local_other.iter().filter(|s| !local_ws.contains(*s) && !local_inh.contains(*s)).for_each(|s| {
        web_sys_aliases.remove(s);
    });
    let is_ws = |name: &str| {
        let n = norm_crate_ident(name);
        n == "web-sys" || web_sys_aliases.contains(&n)
    };
    let mut seen_hits = BTreeSet::new();
    for entry in &entries {
        match entry {
            TomlEntry::Section { line, path } => {
                if let Some((dep, None)) = dep_path_parts(path)
                    && is_banned_crate_name(dep)
                    && seen_hits.insert((*line, norm_crate_ident(dep)))
                {
                    push_v(violations, 1, rel, *line, format!("banned telemetry/HTTP crate `{dep}` in section `[{}]`", path.join(".")));
                }
            }
            TomlEntry::Leaf { line, path, value } if path.first().map(String::as_str) == Some("features") => {
                let mut items = Vec::new();
                value.collect_strings(&mut items);
                for item in items.into_iter().map(str::trim) {
                    let (raw_dep, feat) = item.split_once('/').map_or((item, None), |(d, f)| (d, Some(f.trim())));
                    if let Some(dep) = raw_dep.strip_prefix("dep:").or_else(|| feat.map(|_| raw_dep)).map(|d| d.trim_end_matches('?').trim()) {
                        if is_banned_crate_name(dep) {
                            push_v(violations, 1, rel, *line, format!("banned telemetry/HTTP crate `{dep}` activated in `[features]`"));
                        }
                        if let Some(f) = feat
                            && is_ws(dep)
                            && words_has(BANNED_WEB_SYS_NET_FEATURES, &f.to_ascii_lowercase())
                        {
                            push_v(violations, 1, rel, *line, format!("banned `web-sys` network feature `{f}` activated via `[features]`"));
                        }
                    }
                }
            }
            TomlEntry::Leaf { line, path, value } => {
                let Some((dep, sub)) = dep_path_parts(path) else { continue };
                if dep != "package" && is_banned_crate_name(dep) && seen_hits.insert((*line, norm_crate_ident(dep))) {
                    push_v(violations, 1, rel, *line, format!("banned telemetry/HTTP crate `{dep}` in `Cargo.toml`"));
                }
                if sub == Some("package")
                    && let Some(pkg) = value.as_str().filter(|p| is_banned_crate_name(p))
                {
                    push_v(violations, 1, rel, *line, format!("banned telemetry/HTTP crate `{pkg}` (aliased via `package =`)"));
                }
                if sub == Some("features") && dep_pkg.get(&path[..path.len() - 1]).map_or_else(|| is_ws(dep), |p| p == "web-sys") {
                    let mut feats = Vec::new();
                    value.collect_strings(&mut feats);
                    for val in feats.into_iter().map(str::trim).filter(|f| words_has(BANNED_WEB_SYS_NET_FEATURES, &f.to_ascii_lowercase())) {
                        push_v(violations, 1, rel, *line, format!("banned `web-sys` network feature `{val}` in `Cargo.toml`"));
                    }
                }
            }
        }
    }
}

fn scan_lock_for_banned_crates(lock_text: &str, violations: &mut Vec<TelemetryViolation>) -> usize {
    let (mut count, mut in_pkg, mut name, mut version, mut name_line) = (0usize, false, String::new(), String::new(), 1usize);
    let mut flush = |name: &mut String, version: &mut String, line: usize| {
        if !name.is_empty() {
            count += 1;
            if is_banned_crate_name(name) {
                push_v(violations, 1, "Cargo.lock", line, format!("banned telemetry/HTTP crate `{name}` ({version}) in Cargo.lock"));
            }
            name.clear();
            version.clear();
        }
    };
    for (idx, raw) in lock_text.lines().enumerate() {
        let line = raw.trim();
        if line == "[[package]]" || line.starts_with('[') {
            flush(&mut name, &mut version, name_line);
            in_pkg = line == "[[package]]";
        } else if in_pkg && let Some((k, v)) = line.split_once('=') {
            match k.trim() {
                "name" => (name, name_line) = (unquote_scalar(v).to_owned(), idx + 1),
                "version" => version = unquote_scalar(v).to_owned(),
                _ => {}
            }
        }
    }
    flush(&mut name, &mut version, name_line);
    count
}

#[must_use]
pub fn is_banned_crate_name(raw: &str) -> bool {
    const EXACT: &str = "reqwest ureq hyper surf isahc curl attohttpc minreq awc rama";
    const DASH: &str = "sentry opentelemetry tracing-opentelemetry";
    const PREFIX: &str = "posthog amplitude mixpanel segment bugsnag rollbar datadog newrelic honeycomb aptabase";
    let norm = norm_crate_ident(raw);
    !norm.is_empty()
        && (words_has(EXACT, &norm)
            || DASH.split_ascii_whitespace().any(|p| norm == p || norm.starts_with(&format!("{p}-")))
            || PREFIX.split_ascii_whitespace().any(|p| norm.starts_with(p)))
}

// ---------- Check 2: Rust non-test network primitives & loopback guards ----------

fn check_rust_network_primitives(root: &Path, violations: &mut Vec<TelemetryViolation>) -> Result<usize, String> {
    let mut files = Vec::new();
    for top in ["crates", "apps"] {
        let base = root.join(top);
        if base.is_dir() {
            let mut paths = Vec::new();
            walk_file_paths(&base, &skip_workspace_dir, &|p| has_ext_ci(p, &["rs"]), &mut paths)?;
            for path in paths {
                let rel = rel_slash_lossy(root, &path);
                if (rel.contains("/src/") && !is_cfg_test_only_submodule(root, &rel)) || rel == "build.rs" || rel.ends_with("/build.rs") {
                    files.push(rel);
                }
            }
        }
    }
    files.sort();
    files.dedup();
    for rel in &files {
        let src = std::fs::read_to_string(root.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
        scan_rust_file_network(rel, &strip_non_prod_rust(&src), violations);
    }
    Ok(files.len())
}

fn for_each_use_item(prod_code: &str, mut f: impl FnMut(usize, &str, bool)) {
    for (line, stmt) in crate::layers::collect_use_statements(prod_code) {
        let mut items = Vec::new();
        crate::layers::expand_use_tree_items("", &stmt, &mut items);
        for item in items {
            f(line, item.path.strip_prefix("::").unwrap_or(&item.path), item.alias.is_some());
        }
    }
}

fn scan_rust_file_network(rel: &str, stripped: &StrippedRust, violations: &mut Vec<TelemetryViolation>) {
    const ALWAYS_BANNED_IMPORTS: &str = "std::net::UdpSocket net::UdpSocket std::net::ToSocketAddrs net::ToSocketAddrs core::net core::net::* std::os::unix::net std::os::unix::net::* os::unix::net unix::net web_sys::*";
    const BANNED_LEAVES: &str = "UdpSocket ToSocketAddrs UnixStream UnixListener UnixDatagram WebSocket EventSource XmlHttpRequest Request RequestInit RequestMode Response Headers Navigator Worker SharedWorker ServiceWorkerContainer";
    const JS_NEEDLES: &str =
        "fetch_with_request_and_init fetch_with_request fetch_with_str fetch sendbeacon send_beacon websocket eventsource xmlhttprequest";
    let banned_pats: Vec<&str> = ALWAYS_BANNED_NET_PATTERNS.split_ascii_whitespace().collect();
    let (compact_prod, prod_lines) = crate::layers::collapse_colon_whitespace(&stripped.prod_code);
    scan_code_patterns_with_lines(rel, &compact_prod, &prod_lines, &banned_pats, "banned network socket primitive", violations);

    let mut scoped_hits = Vec::new();
    scan_code_patterns_with_lines(rel, &compact_prod, &prod_lines, LOOPBACK_SCOPED_NET_PATTERNS, "unapproved std::net / TCP usage", &mut scoped_hits);
    let allowlisted = ALLOWED_LOOPBACK_NET_FILES.contains(&rel);
    for_each_use_item(&stripped.prod_code, |line, norm, aliased| {
        if let Some(pat) =
            ALWAYS_BANNED_IMPORTS.split_ascii_whitespace().find(|&p| norm == p).or_else(|| (norm == "web_sys" && aliased).then_some("web_sys"))
        {
            push_v(violations, 2, rel, line, format!("banned network socket primitive import `{pat}`"));
        } else if ["std::net::", "core::net::", "std::os::unix::net::", "web_sys::"].iter().any(|p| norm.starts_with(p))
            && norm.rsplit("::").next().is_some_and(|leaf| words_has(BANNED_LEAVES, leaf))
        {
            push_v(violations, 2, rel, line, format!("banned network socket primitive import `{norm}`"));
        }
        if norm == "std::net" || norm == "std::net::*" || norm.starts_with("std::net::") {
            push_v(&mut scoped_hits, 2, rel, line, format!("unapproved std::net / TCP usage `{norm}`"));
            if allowlisted && (norm == "std::net::*" || aliased) {
                push_v(violations, 2, rel, line, "aliasing or wildcard-importing `std::net` is forbidden in allowlisted files");
            }
        }
    });
    if ["js_sys", "web_sys", "wasm_bindgen"].iter().any(|k| stripped.prod_code.contains(k)) {
        let js_needles: Vec<&str> = JS_NEEDLES.split_ascii_whitespace().collect();
        for (line_no, raw_lit) in extract_rust_string_literals(&stripped.no_comments_code) {
            if let Some((_, matched)) = next_ident_pattern_match(&raw_lit.trim().to_ascii_lowercase(), 0, &js_needles) {
                push_v(violations, 2, rel, line_no, format!("forbidden JS network reflection string `{matched}` in WASM source"));
            }
        }
    }
    if !allowlisted {
        violations.extend(scoped_hits);
    } else {
        check_allowlisted_net_file(rel, stripped, &scoped_hits, violations);
    }
}

fn extract_rust_string_literals(src: &str) -> Vec<(usize, String)> {
    let (mut lex, mut lits) = (RustLexer::new(src), Vec::new());
    while !lex.is_eof() {
        if lex.try_read_comment().is_some() || lex.try_skip_char_literal() {
        } else if let Some(lit) = lex.try_read_raw_string().or_else(|| lex.try_read_quoted_string()) {
            lits.push((lit.start_line, lit.text));
        } else {
            lex.advance_one();
        }
    }
    lits
}

fn check_allowlisted_net_file(rel: &str, stripped: &StrippedRust, scoped_hits: &[TelemetryViolation], violations: &mut Vec<TelemetryViolation>) {
    let fn_spans = collect_rust_fn_spans(&stripped.prod_code);
    let mut checked = 0usize;
    for &(start, end, line_no) in &fn_spans {
        if let Some(fn_prod) = stripped.prod_code.get(start..end)
            && fn_opens_socket(fn_prod)
        {
            checked += 1;
            check_loopback_slice(
                rel,
                line_no,
                fn_prod,
                stripped.no_comments_code.get(start..end).unwrap_or(fn_prod),
                "allowlisted network function",
                violations,
            );
        }
    }
    let (outside_prod, outside_lits) = mask_spans_preserving_lines(&stripped.prod_code, &stripped.no_comments_code, &fn_spans);
    if fn_opens_socket(&outside_prod) {
        let ln = outside_prod
            .lines()
            .enumerate()
            .find(|(_, l)| ["TcpStream", "TcpListener", "bind(", "connect"].iter().any(|k| l.contains(k)))
            .map_or(1, |(i, _)| i + 1);
        check_loopback_slice(rel, ln, &outside_prod, &outside_lits, "allowlisted module-level network initializer", violations);
    } else if checked == 0 && !scoped_hits.is_empty() && !slice_has_loopback_guard(&stripped.prod_code, &stripped.no_comments_code) {
        push_v(
            violations,
            2,
            rel,
            scoped_hits.first().map_or(1, |v| v.line),
            "allowlisted network module lacks a loopback guard (`is_loopback`, `[127, 0, 0, 1]`, `127.0.0.1`, `::1`, or `LOCALHOST`)",
        );
    }
}

fn check_loopback_slice(rel: &str, base_line: usize, prod: &str, lits: &str, ctx: &str, violations: &mut Vec<TelemetryViolation>) {
    if !slice_has_loopback_guard(prod, lits) {
        push_v(
            violations,
            2,
            rel,
            base_line,
            format!("{ctx} lacks a loopback guard (`is_loopback`, `[127, 0, 0, 1]`, `127.0.0.1`, `::1`, or `LOCALHOST`)"),
        );
    }
    for (rel_line, lit) in extract_rust_string_literals(lits) {
        if let Some(bad) = check_non_loopback_host_or_ip(&lit) {
            push_v(violations, 2, rel, base_line + rel_line.saturating_sub(1), format!("non-loopback socket literal `{bad}` in {ctx}"));
        }
    }
    let (compact, lines) = crate::layers::collapse_colon_whitespace(prod);
    for pat in ["Ipv4Addr::UNSPECIFIED", "Ipv6Addr::UNSPECIFIED", "Ipv4Addr::BROADCAST"] {
        if let Some((pos, _)) = next_ident_pattern_match(&compact, 0, &[pat]) {
            push_v(
                violations,
                2,
                rel,
                base_line + lines.get(pos).copied().unwrap_or(1).saturating_sub(1),
                format!("non-loopback IP constant `{pat}` in {ctx}"),
            );
        }
    }
}

fn mask_spans_preserving_lines(prod_code: &str, lits_code: &str, spans: &[(usize, usize, usize)]) -> (String, String) {
    let (mut prod, mut lits) = (prod_code.as_bytes().to_vec(), lits_code.as_bytes().to_vec());
    for &(start, end, _) in spans {
        for buf in [&mut prod, &mut lits] {
            buf.iter_mut().take(end).skip(start).filter(|b| **b != b'\n').for_each(|b| *b = b' ');
        }
    }
    (
        String::from_utf8(prod).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
        String::from_utf8(lits).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
    )
}

fn collect_rust_fn_spans(prod_code: &str) -> Vec<(usize, usize, usize)> {
    let (bytes, mut spans, mut i, mut line) = (prod_code.as_bytes(), Vec::new(), 0usize, 1usize);
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            line += 1;
        } else if bytes[i..].starts_with(b"fn") && (i == 0 || !is_ident_byte(bytes[i - 1])) && bytes.get(i + 2).is_some_and(u8::is_ascii_whitespace) {
            let mut p = i + 2;
            while p < bytes.len() && bytes[p].is_ascii_whitespace() {
                p += 1;
            }
            if bytes.get(p).is_some_and(|b| is_ident_byte(*b)) {
                while p < bytes.len() && is_ident_byte(bytes[p]) {
                    p += 1;
                }
                if let Some((_, close)) = find_fn_body_braces(bytes, p) {
                    spans.push((i, close + 1, line));
                    i += 2;
                    continue;
                }
            }
        }
        i += 1;
    }
    spans
}

fn fn_opens_socket(fn_prod: &str) -> bool {
    let (c, _) = crate::layers::collapse_colon_whitespace(fn_prod);
    c.contains("TcpListener::bind")
        || c.contains("TcpStream::connect")
        || ((c.contains("TcpListener") || c.contains("TcpStream") || c.contains("std::net"))
            && (c.contains("::bind(") || c.contains("::connect(") || c.contains("connect_timeout(")))
}

fn slice_has_loopback_guard(prod_code: &str, no_comments_code: &str) -> bool {
    const GUARDS: &str = ".is_loopback() validate_loopback_addr( bind_control_tcp_listener( SocketAddr::from(([127,0,0,1], SocketAddrV4::new(Ipv4Addr::LOCALHOST, SocketAddr::from((Ipv4Addr::LOCALHOST, SocketAddr::from((IpAddr::V4(Ipv4Addr::LOCALHOST), SocketAddr::from((Ipv6Addr::LOCALHOST, ::bind((Ipv4Addr::LOCALHOST, ::connect((Ipv4Addr::LOCALHOST, ::bind(([127,0,0,1], ::connect(([127,0,0,1],";
    let compact: String = prod_code.chars().filter(|c| !c.is_whitespace()).collect();
    GUARDS.split_ascii_whitespace().any(|k| compact.contains(k))
        || no_comments_code.lines().any(|line| {
            ["TcpListener", "TcpStream", "SocketAddr", "bind(", "connect", ".parse"].iter().any(|k| line.contains(k))
                && (line.contains("\"127.0.0.1") || line.contains("\"::1") || line.contains("\"[::1]"))
        })
}

fn check_non_loopback_host_or_ip(lit: &str) -> Option<String> {
    fn unbracket(s: &str) -> &str {
        s.strip_prefix('[').and_then(|x| x.strip_suffix(']')).unwrap_or(s)
    }
    let t = lit.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(sock) = t.parse::<std::net::SocketAddr>() {
        return (!sock.ip().is_loopback()).then(|| t.to_owned());
    }
    if let Ok(ip) = unbracket(t).parse::<std::net::IpAddr>() {
        return (!ip.is_loopback()).then(|| t.to_owned());
    }
    let (host, port) = t.rsplit_once(':')?;
    if port.parse::<u16>().is_err() {
        return None;
    }
    let clean = unbracket(host);
    if let Ok(ip) = clean.parse::<std::net::IpAddr>() {
        return (!ip.is_loopback()).then(|| t.to_owned());
    }
    (!clean.is_empty()
        && !clean.eq_ignore_ascii_case("localhost")
        && clean.contains('.')
        && clean.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
        && clean.bytes().any(|b| b.is_ascii_alphabetic()))
    .then(|| t.to_owned())
}

fn scan_code_patterns_with_lines(
    rel: &str,
    code: &str,
    line_of_byte: &[usize],
    patterns: &[&str],
    reason: &str,
    violations: &mut Vec<TelemetryViolation>,
) {
    let mut pos = 0usize;
    while let Some((hit_pos, pat)) = next_ident_pattern_match(code, pos, patterns) {
        push_v(violations, 2, rel, line_of_byte.get(hit_pos).copied().unwrap_or(1), format!("{reason} `{pat}`"));
        pos = hit_pos + pat.len();
    }
}

fn skip_workspace_dir(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|s| matches!(s, "target" | "dist" | "corpus") || s.starts_with('.'))
}

// ---------- Check 3: Web & workflow telemetry / external URLs / browser network APIs ----------

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
    let (mut scanned, mut web_assets) = (0usize, Vec::new());
    for rel in &targets {
        let bytes = std::fs::read(root.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
        let text = match crate::text_decode::decode_utf8_or_utf16(&bytes) {
            Ok(Some(d)) => d,
            _ if is_binary_web_asset_blob(rel, &bytes) => continue,
            _ => String::from_utf8_lossy(&bytes).into_owned(),
        };
        scanned += 1;
        scan_text_for_web_telemetry(rel, &text, violations);
        if is_web_asset_file(rel) {
            web_assets.push((rel.clone(), text));
        }
    }
    if !root.join("apps/curvebook-web/index.html").is_file() {
        push_v(violations, 3, "apps/curvebook-web/index.html", 1, "missing apps/curvebook-web/index.html");
    }
    web_assets.into_iter().for_each(|(rel, text)| scan_web_asset_external_urls_and_apis(&rel, &text, violations));
    Ok(scanned)
}

fn is_binary_web_asset_blob(rel: &str, bytes: &[u8]) -> bool {
    const EXTS: &str = "png jpg jpeg gif webp ico icns ttf otf woff woff2 wasm zip pdf";
    words_has(EXTS, &lower_ext(rel))
        || (!is_web_asset_file(rel)
            && [&b"\x89PNG\r\n\x1a\n"[..], b"PK\x03\x04", b"\x00asm", b"wOF2", b"OTTO", b"\x00\x01\x00\x00"].iter().any(|s| bytes.starts_with(s)))
}

fn is_web_asset_file(rel: &str) -> bool {
    (rel.starts_with("apps/curvebook-web/") || rel.starts_with("packaging/web/"))
        && words_has("html htm js mjs cjs ts jsx tsx css svg json", &lower_ext(rel))
}

fn scan_text_for_web_telemetry(rel: &str, text: &str, violations: &mut Vec<TelemetryViolation>) {
    let check_fetch = words_has("html htm js mjs cjs ts jsx tsx svg", &lower_ext(rel));
    for (idx, line) in text.lines().enumerate() {
        let lower = crate::text_decode::decode_xml_html_entities(line).to_ascii_lowercase();
        for term in BANNED_WEB_AND_WORKFLOW_TERMS.split('|') {
            if contains_telemetry_term(&lower, term) {
                push_v(violations, 3, rel, idx + 1, format!("forbidden telemetry/cloud-service reference `{term}`"));
            }
        }
        if check_fetch && contains_browser_fetch_call(&lower) {
            push_v(violations, 3, rel, idx + 1, "forbidden network `fetch(` call");
        }
    }
}

fn contains_telemetry_term(lower_line: &str, term: &str) -> bool {
    let (bytes, mut start) = (lower_line.as_bytes(), 0usize);
    while let Some(rel) = lower_line.get(start..).and_then(|s| s.find(term)) {
        let (pos, end) = (start + rel, start + rel + term.len());
        if (pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric())
            && (!term.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric) || end == bytes.len() || !bytes[end].is_ascii_alphanumeric())
        {
            return true;
        }
        start = pos + lower_line.get(pos..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
    }
    false
}

fn contains_browser_fetch_call(lower_line: &str) -> bool {
    let (bytes, mut start) = (lower_line.as_bytes(), 0usize);
    while let Some(rel) = lower_line.get(start..).and_then(|s| s.find("fetch")) {
        let pos = start + rel;
        let left_ok = pos == 0
            || (bytes[pos - 1] != b'.' && !is_ident_byte(bytes[pos - 1]))
            || (bytes[pos - 1] == b'.'
                && lower_line.get(..pos).is_some_and(|b| {
                    ["window.", "globalthis.", "self."]
                        .iter()
                        .any(|r| b.trim_end().strip_suffix(r).is_some_and(|p| p.is_empty() || !is_ident_byte(p.as_bytes()[p.len() - 1])))
                }));
        if left_ok && lower_line.get(pos + 5..).unwrap_or("").trim_start().starts_with('(') {
            return true;
        }
        start = pos + 1;
    }
    false
}

fn scan_web_asset_external_urls_and_apis(rel: &str, content: &str, violations: &mut Vec<TelemetryViolation>) {
    let is_css = rel.ends_with(".css");
    let url_end = |s: &str| s.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | ';' | '`' | ',')).unwrap_or(s.len());
    for (idx, line) in content.lines().enumerate() {
        let decoded = crate::text_decode::decode_xml_html_entities(line);
        let lower = decoded.to_ascii_lowercase();
        for scheme in ["https://", "http://", "wss://", "ws://"] {
            let mut start = 0usize;
            while let Some(off) = lower.get(start..).and_then(|s| s.find(scheme)) {
                let rest = decoded.get(start + off..).unwrap_or("");
                let url = rest.get(..url_end(rest)).unwrap_or(rest);
                if !ALLOWED_HTML_EXTERNAL_URLS.iter().any(|a| url.eq_ignore_ascii_case(a)) {
                    push_v(violations, 3, rel, idx + 1, format!("external URL `{url}` is forbidden in web assets"));
                }
                start += off + scheme.len();
            }
        }
        let (bytes, mut pos) = (decoded.as_bytes(), 0usize);
        while let Some(off) = decoded.get(pos..).and_then(|s| s.find("//")) {
            let hit = pos + off;
            let after = decoded.get(hit + 2..).unwrap_or("");
            let target = after.get(..url_end(after)).unwrap_or(after);
            let host = target.split('/').next().unwrap_or(target);
            if hit > 0
                && matches!(bytes[hit - 1], b'=' | b'"' | b'\'' | b'`' | b'(')
                && !host.is_empty()
                && host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'))
                && (host.contains('.') || host.eq_ignore_ascii_case("localhost"))
            {
                push_v(violations, 3, rel, idx + 1, format!("protocol-relative external URL `//{target}` is forbidden in web assets"));
            }
            pos = hit + 2;
        }
        if !is_css {
            for api in ["websocket", "eventsource", "sharedworker", "serviceworker"] {
                if next_ident_pattern_match(&lower, 0, &[api]).is_some() {
                    push_v(violations, 3, rel, idx + 1, format!("forbidden browser network/worker API `{api}` in web asset"));
                }
            }
            let mut wpos = 0usize;
            while let Some(off) = lower.get(wpos..).and_then(|s| s.find("worker")) {
                let hit = wpos + off;
                if (hit == 0 || !is_ident_byte(lower.as_bytes()[hit - 1])) && lower.get(hit + 6..).unwrap_or("").trim_start().starts_with('(') {
                    push_v(violations, 3, rel, idx + 1, "forbidden browser network/worker API `worker(` in web asset");
                    break;
                }
                wpos = hit + 1;
            }
        }
    }
}

// ---------- Check 4: GitHub Actions workflows & composite actions ----------

fn check_workflow_security(root: &Path, violations: &mut Vec<TelemetryViolation>) -> Result<usize, String> {
    let workflows = collect_workflow_and_action_files(root)?;
    if workflows.is_empty() {
        push_v(violations, 4, ".github/workflows", 1, "no GitHub Actions workflow files found");
        return Ok(0);
    }
    for rel in &workflows {
        let text = std::fs::read_to_string(root.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
        audit_workflow_yaml(rel, &text, !rel.starts_with(".github/actions/"), violations);
    }
    Ok(workflows.len())
}

#[derive(Default)]
struct WorkflowState {
    in_jobs: bool,
    job_indent: Option<usize>,
    current_job: Option<String>,
    steps_indent: Option<usize>,
    top_perm_indent: Option<usize>,
    job_perm_indent: Option<usize>,
    saw_top_permissions: bool,
    top_has_contents_read: bool,
    run_block: Option<(usize, Vec<(usize, String)>)>,
}

fn split_yaml_key(trimmed: &str) -> Option<(&str, &str)> {
    let (bytes, mut quote, mut i) = (trimmed.as_bytes(), 0u8, 0usize);
    while i < bytes.len() {
        let b = bytes[i];
        if quote != 0 {
            if b == b'\\' && quote == b'"' {
                i += 2;
                continue;
            }
            if b == quote {
                quote = 0;
            }
        } else if matches!(b, b'"' | b'\'') {
            quote = b;
        } else if b == b':' && bytes.get(i + 1).is_none_or(u8::is_ascii_whitespace) {
            let (raw_k, rest) = trimmed.split_at_checked(i)?;
            return Some((unquote_scalar(raw_k.trim()), rest.strip_prefix(':').unwrap_or(rest).trim()));
        }
        i += 1;
    }
    None
}

fn audit_workflow_yaml(rel: &str, yaml: &str, require_permissions: bool, violations: &mut Vec<TelemetryViolation>) {
    let mut st = WorkflowState::default();
    for (idx, raw) in yaml.lines().enumerate() {
        let line_no = idx + 1;
        if let Some((key_col, ref mut lines)) = st.run_block {
            if raw.trim().is_empty() || leading_spaces(raw) > key_col {
                lines.push((line_no, raw.to_owned()));
                continue;
            }
            flush_run_block(rel, &mut st, violations);
        }
        let code = strip_hash_comment(raw, true);
        let trimmed = code.trim();
        if trimmed.is_empty() {
            continue;
        }
        let indent = leading_spaces(code);
        if st.top_perm_indent.is_some_and(|p| indent <= p) {
            st.top_perm_indent = None;
        }
        if st.job_perm_indent.is_some_and(|p| indent <= p) {
            st.job_perm_indent = None;
        }
        if st.steps_indent.is_some_and(|s| indent < s || (indent == s && !trimmed.starts_with('-'))) {
            st.steps_indent = None;
        }
        if indent == 0 {
            st.in_jobs = split_yaml_key(trimmed).is_some_and(|(k, rhs)| k == "jobs" && rhs.is_empty());
            if !st.in_jobs {
                (st.job_indent, st.current_job) = (None, None);
            }
        } else if st.in_jobs && !trimmed.starts_with('-') {
            let expected = *st.job_indent.get_or_insert(indent);
            if indent == expected
                && let Some((k, rhs)) = split_yaml_key(trimmed)
                && rhs.is_empty()
                && !k.contains(char::is_whitespace)
            {
                (st.current_job, st.steps_indent) = (Some(k.to_owned()), None);
            }
        }
        if !trimmed.starts_with('-') && split_yaml_key(trimmed).is_some_and(|(k, _)| k == "steps") {
            st.steps_indent = Some(indent);
        }
        if handle_permissions_line(rel, line_no, indent, trimmed, &mut st, violations) {
            continue;
        }
        let t = code.trim_start();
        if let Some(("uses", rhs)) = split_yaml_key(t.strip_prefix('-').map_or(t, str::trim_start)) {
            let spec = unquote_scalar(rhs.trim_end_matches([',', '}']).trim());
            if !spec.starts_with("./")
                && !spec.starts_with("../")
                && !spec.split_once('@').is_some_and(|(_, sha)| sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                push_v(violations, 4, rel, line_no, format!("GitHub Action `{spec}` must be pinned to a full 40-character commit SHA"));
            }
        }
        check_run_directive(rel, line_no, indent, code, &mut st, violations);
    }
    flush_run_block(rel, &mut st, violations);
    if require_permissions && (!st.saw_top_permissions || !st.top_has_contents_read) {
        push_v(violations, 4, rel, 1, "workflow must declare top-level `permissions:` containing `contents: read`");
    }
}

fn handle_permissions_line(
    rel: &str,
    line_no: usize,
    indent: usize,
    trimmed: &str,
    st: &mut WorkflowState,
    violations: &mut Vec<TelemetryViolation>,
) -> bool {
    let check_entry = |scope: &str, level: &str, is_top: bool, st: &mut WorkflowState, out: &mut Vec<TelemetryViolation>| {
        let (scope, level) = (unquote_scalar(scope), unquote_scalar(level));
        if is_top && scope == "contents" && level == "read" {
            st.top_has_contents_read = true;
        }
        if level == "write" {
            let job = st.current_job.as_deref();
            let ok = !is_top
                && ((rel == ".github/workflows/release.yml" && job == Some("publish") && matches!(scope, "contents" | "id-token" | "attestations"))
                    || (rel == ".github/workflows/ci.yml" && job == Some("prune-caches") && scope == "actions")
                    || (rel == ".github/workflows/pages.yml" && job == Some("deploy") && matches!(scope, "pages" | "id-token")));
            if !ok {
                push_v(out, 4, rel, line_no, format!("unexpected write permission `{scope}: write`"));
            }
        }
    };
    if let Some(("permissions", rhs)) = split_yaml_key(trimmed) {
        let is_top = indent == 0;
        if is_top {
            (st.saw_top_permissions, st.top_perm_indent) = (true, Some(indent));
        } else {
            st.job_perm_indent = Some(indent);
        }
        if unquote_scalar(rhs) == "write-all" {
            push_v(violations, 4, rel, line_no, "`permissions: write-all` is forbidden");
        } else if let Some(inner) = rhs.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            for item in inner.split(',') {
                if let Some((scope, level)) = split_yaml_key(item.trim()) {
                    check_entry(scope, level, is_top, st, violations);
                }
            }
            if is_top {
                st.top_perm_indent = None;
            } else {
                st.job_perm_indent = None;
            }
        }
        return true;
    }
    let scope_is_top = st.top_perm_indent.filter(|&p| indent > p).map(|_| true).or_else(|| st.job_perm_indent.filter(|&p| indent > p).map(|_| false));
    if let Some(is_top) = scope_is_top
        && let Some((scope, level)) = split_yaml_key(trimmed)
    {
        check_entry(scope, level.trim_end_matches([',', '}']).trim(), is_top, st, violations);
        return true;
    }
    false
}

fn check_run_directive(rel: &str, line_no: usize, indent: usize, code: &str, st: &mut WorkflowState, violations: &mut Vec<TelemetryViolation>) {
    if !st.steps_indent.is_some_and(|s| indent >= s) {
        return;
    }
    let after_indent = code.trim_start_matches(' ');
    let (key_col, after_dash) = if let Some(rest) = after_indent.strip_prefix('-') {
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            return;
        }
        (indent + 1 + rest.len() - rest.trim_start_matches(' ').len(), rest.trim_start_matches(' '))
    } else {
        (indent, after_indent)
    };
    let Some((key, rhs)) = split_yaml_key(after_dash) else { return };
    if !matches!(key, "run" | "script" | "prepare") {
        return;
    }
    let trimmed_rhs = rhs.trim();
    if trimmed_rhs.is_empty() || trimmed_rhs.strip_prefix(['|', '>']).is_some_and(|r| r.chars().all(|c| matches!(c, '-' | '+' | '0'..='9'))) {
        st.run_block = Some((key_col, Vec::new()));
    } else {
        let val = unquote_scalar(trimmed_rhs);
        flag_untrusted_event_expr_spans(rel, val, &vec![line_no; val.len()], violations);
        flag_remote_script_execution(rel, line_no, val, violations);
    }
}

fn flush_run_block(rel: &str, st: &mut WorkflowState, violations: &mut Vec<TelemetryViolation>) {
    let Some((_, lines)) = st.run_block.take().filter(|(_, l)| !l.is_empty()) else { return };
    let (mut combined, mut offset_lines) = (String::new(), Vec::new());
    for (i, (ln, txt)) in lines.iter().enumerate() {
        if i > 0 {
            combined.push('\n');
            offset_lines.push(*ln);
        }
        combined.push_str(txt);
        offset_lines.extend(std::iter::repeat_n(*ln, txt.len()));
    }
    flag_untrusted_event_expr_spans(rel, &combined, &offset_lines, violations);
    let (mut logical_ln, mut logical_cmd) = (lines[0].0, String::new());
    for (ln, raw) in &lines {
        if logical_cmd.is_empty() {
            logical_ln = *ln;
        }
        let code = strip_hash_comment(raw, true).trim_end();
        if let Some(cont) = code.strip_suffix('\\') {
            logical_cmd.push_str(cont.trim_end());
            logical_cmd.push(' ');
        } else {
            logical_cmd.push_str(code);
            flag_remote_script_execution(rel, logical_ln, &logical_cmd, violations);
            logical_cmd.clear();
        }
    }
    if !logical_cmd.is_empty() {
        flag_remote_script_execution(rel, logical_ln, &logical_cmd, violations);
    }
}

fn flag_untrusted_event_expr_spans(rel: &str, text: &str, offset_lines: &[usize], violations: &mut Vec<TelemetryViolation>) {
    let mut pos = 0usize;
    while let Some(rel_off) = text.get(pos..).and_then(|s| s.find("${{")) {
        let (expr_line, expr_start) = (offset_lines.get(pos + rel_off).copied().unwrap_or(1), pos + rel_off + 3);
        let rest = text.get(expr_start..).unwrap_or("");
        let (expr, next_pos) = rest.find("}}").map_or((rest, text.len()), |e| (rest.get(..e).unwrap_or(rest), expr_start + e + 2));
        let compact: String = expr.chars().filter(|c| !c.is_whitespace()).map(|c| c.to_ascii_lowercase()).collect();
        for (needle, label, forbid_dot_left) in [
            ("github.event", "github.event.*", false),
            ("inputs", "inputs.*", true),
            ("github.head_ref", "github.head_ref", false),
            ("github.ref_name", "github.ref_name", false),
        ] {
            let (bytes, mut p) = (compact.as_bytes(), 0usize);
            while let Some(r) = compact.get(p..).and_then(|s| s.find(needle)) {
                let (h, e) = (p + r, p + r + needle.len());
                if (h == 0 || (!is_ident_byte(bytes[h - 1]) && (!forbid_dot_left || bytes[h - 1] != b'.')))
                    && (e == bytes.len() || !is_ident_byte(bytes[e]))
                {
                    push_v(
                        violations,
                        4,
                        rel,
                        expr_line,
                        format!("untrusted `${{{{ {label} }}}}` interpolated directly in workflow script block (use `env:` instead)"),
                    );
                    break;
                }
                p = h + 1;
            }
        }
        pos = next_pos;
    }
}

fn flag_remote_script_execution(rel: &str, line_no: usize, raw: &str, violations: &mut Vec<TelemetryViolation>) {
    const SINKS: &str = "sh bash zsh dash ksh ash fish python python3 perl ruby node tar bsdtar gtar";
    let code = strip_hash_comment(raw, true).trim();
    if code.is_empty() || code.starts_with('#') {
        return;
    }
    let lower = code.to_ascii_lowercase();
    if lower.contains("raw.githubusercontent.com/") && (lower.contains("/main/") || lower.contains("/master/") || lower.contains("latest")) {
        push_v(violations, 4, rel, line_no, "unpinned remote script URL off `main`/`master` branch is forbidden in workflows");
    }
    for segment in lower.split([';', '\n']) {
        let mut saw_fetcher = false;
        for stage in segment.split('|').filter(|s| !s.is_empty()) {
            if saw_fetcher
                && stage.split_whitespace().any(|tok| {
                    let base = tok.trim_matches(['"', '\'', '(', ')', '{', '}']).rsplit('/').next().unwrap_or("");
                    words_has(SINKS, base) || base.starts_with("python3.")
                })
            {
                push_v(
                    violations,
                    4,
                    rel,
                    line_no,
                    "piping remote download directly to shell or archive extractor without sha256sum verification is forbidden",
                );
                return;
            }
            saw_fetcher |= contains_telemetry_term(stage, "curl") || contains_telemetry_term(stage, "wget");
        }
    }
}

// ---------- Check 5: Static web security headers (_headers & .htaccess) ----------

type HeaderMap = BTreeMap<String, (usize, String)>;
type HeaderEntry = (String, usize, String);

fn check_web_security_headers(root: &Path, violations: &mut Vec<TelemetryViolation>) -> Result<usize, String> {
    type Parser = fn(&str) -> (HeaderMap, Vec<HeaderEntry>);
    let configs: &[(&str, Parser)] = &[("packaging/web/_headers", parse_netlify_header_blocks), ("packaging/web/.htaccess", parse_htaccess_headers)];
    let mut count = 0usize;
    for &(rel, parser) in configs {
        let path = root.join(rel);
        if !path.is_file() {
            push_v(violations, 5, rel, 1, format!("missing {rel}"));
        } else {
            count += 1;
            let (global_map, all_entries) = parser(&std::fs::read_to_string(&path).map_err(|e| format!("{rel}: {e}"))?);
            validate_required_web_headers(rel, &global_map, &all_entries, violations);
        }
    }
    Ok(count)
}

fn parse_netlify_header_blocks(text: &str) -> (HeaderMap, Vec<HeaderEntry>) {
    let (mut global, mut all, mut in_global) = (HeaderMap::new(), Vec::new(), false);
    for (idx, raw) in text.lines().enumerate() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !raw.starts_with(char::is_whitespace) {
            in_global = trimmed == "/*";
        } else if let Some((name, val)) = trimmed.split_once(':') {
            let (k, v) = (name.trim().to_ascii_lowercase(), val.trim().to_owned());
            if in_global {
                global.insert(k.clone(), (idx + 1, v.clone()));
            }
            all.push((k, idx + 1, v));
        }
    }
    (global, all)
}

fn parse_htaccess_headers(text: &str) -> (HeaderMap, Vec<HeaderEntry>) {
    const SCOPED: &str = "files filesmatch location locationmatch directory directorymatch if elseif else ifdefine ifversion ifdirective ifsection iffile limit limitexcept";
    let (mut map, mut all, mut stack): (HeaderMap, Vec<HeaderEntry>, Vec<(String, bool)>) = (HeaderMap::new(), Vec::new(), Vec::new());
    for (idx, raw) in text.lines().enumerate() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(after) = trimmed.strip_prefix('<') {
            let body = after.trim_start_matches('/').trim_end_matches('>').trim();
            let (name_raw, args) = body.split_once(char::is_whitespace).unwrap_or((body, ""));
            let name = name_raw.to_ascii_lowercase();
            if after.starts_with('/') {
                if let Some(pos) = stack.iter().rposition(|(t, _)| t == &name) {
                    stack.truncate(pos);
                }
            } else {
                let scoped = words_has(SCOPED, &name) || (name == "ifmodule" && args.trim_start().trim_start_matches(['"', '\'']).starts_with('!'));
                stack.push((name, scoped));
            }
            continue;
        }
        let in_scoped = stack.iter().any(|(_, s)| *s);
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
    let (first, rest) = next_apache_token(trimmed)?;
    if !unquote_scalar(first).eq_ignore_ascii_case("header") {
        return None;
    }
    let mut tok = next_apache_token(rest)?;
    if matches!(unquote_scalar(tok.0).to_ascii_lowercase().as_str(), "always" | "onsuccess") {
        tok = next_apache_token(tok.1)?;
    }
    let action = unquote_scalar(tok.0).to_ascii_lowercase();
    if !matches!(action.as_str(), "set" | "setifempty" | "add" | "append" | "unset" | "edit" | "edit*" | "merge") {
        return None;
    }
    let (raw_name, val_part) = next_apache_token(tok.1)?;
    let name = unquote_scalar(raw_name).trim_end_matches(':').trim().to_owned();
    (!name.is_empty()).then(|| (action, name, unquote_scalar(val_part.trim()).to_owned()))
}

fn next_apache_token(s: &str) -> Option<(&str, &str)> {
    let trimmed = s.trim_start();
    let &first = trimmed.as_bytes().first()?;
    if matches!(first, b'"' | b'\'') {
        let (bytes, mut i) = (trimmed.as_bytes(), 1usize);
        while i < bytes.len() {
            if bytes[i] == b'\\' && first == b'"' && i + 1 < bytes.len() {
                i += 2;
            } else if bytes[i] == first {
                return trimmed.split_at_checked(i + 1);
            } else {
                i += 1;
            }
        }
        return Some((trimmed, ""));
    }
    Some(trimmed.find(char::is_whitespace).and_then(|idx| trimmed.split_at_checked(idx)).unwrap_or((trimmed, "")))
}

fn validate_required_web_headers(rel: &str, headers: &HeaderMap, all_entries: &[HeaderEntry], violations: &mut Vec<TelemetryViolation>) {
    const REQUIRED: &[(&str, &str, Option<&str>)] = &[
        ("content-security-policy", "Content-Security-Policy", None),
        ("referrer-policy", "Referrer-Policy", Some("no-referrer")),
        ("permissions-policy", "Permissions-Policy", None),
        ("x-content-type-options", "X-Content-Type-Options", Some("nosniff")),
        ("cross-origin-opener-policy", "Cross-Origin-Opener-Policy", Some("same-origin")),
        ("cross-origin-embedder-policy", "Cross-Origin-Embedder-Policy", Some("require-corp")),
    ];
    for &(key, display, expected) in REQUIRED {
        if !headers.contains_key(key) {
            let suffix = expected.map_or_else(String::new, |v| format!(": {v}"));
            push_v(violations, 5, rel, 1, format!("missing `{display}{suffix}` header"));
        }
    }
    for (key, line, val) in all_entries {
        if let Some(hdr) = key.strip_prefix("__unset__:") {
            if REQUIRED.iter().any(|&(r, _, _)| r == hdr) {
                push_v(violations, 5, rel, *line, format!("forbids `Header ... unset {hdr}` on required security header"));
            }
        } else if let Some((act, hdr)) = key.strip_prefix("__mutate__:").and_then(|s| s.split_once(':')) {
            if REQUIRED.iter().any(|&(r, _, _)| r == hdr) {
                push_v(violations, 5, rel, *line, format!("forbids `Header ... {act} {hdr}` mutation on required security header"));
            }
        } else if key == "content-security-policy" {
            validate_csp_directives(rel, *line, val, violations);
        } else if key == "permissions-policy" {
            let compact: String = val.chars().filter(|c| !c.is_whitespace()).map(|c| c.to_ascii_lowercase()).collect();
            for feat in ["camera=()", "microphone=()", "geolocation=()"] {
                if !compact.contains(feat) {
                    push_v(violations, 5, rel, *line, format!("`Permissions-Policy` must disable `{feat}`"));
                }
            }
        } else if let Some(&(_, display, Some(expected))) = REQUIRED.iter().find(|&&(k, _, _)| k == key)
            && !val.trim().eq_ignore_ascii_case(expected)
        {
            push_v(violations, 5, rel, *line, format!("`{display}` must be `{expected}`, found `{val}`"));
        }
    }
}

fn validate_csp_directives(rel: &str, line: usize, csp: &str, violations: &mut Vec<TelemetryViolation>) {
    let directives: Vec<String> =
        csp.split(';').map(|d| d.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase()).filter(|d| !d.is_empty()).collect();
    let mut seen = BTreeSet::new();
    for dir in &directives {
        let name = dir.split_whitespace().next().unwrap_or("");
        if !seen.insert(name) {
            push_v(violations, 5, rel, line, format!("`Content-Security-Policy` contains duplicate directive `{name}`"));
        }
    }
    for req in REQUIRED_CSP_DIRECTIVES {
        if !directives.iter().any(|d| d == req) {
            push_v(violations, 5, rel, line, format!("`Content-Security-Policy` missing exact directive `{req}`"));
        }
    }
    for dir in &directives {
        let name = dir.split_whitespace().next().unwrap_or("");
        for tok in dir.split_whitespace().skip(1) {
            let is_hash = tok
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
                .and_then(|i| i.strip_prefix("sha256-").or_else(|| i.strip_prefix("sha384-")).or_else(|| i.strip_prefix("sha512-")))
                .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'-' | b'_')));
            let ok = matches!(tok, "'self'" | "'none'" | "'unsafe-inline'" | "'wasm-unsafe-eval'")
                || is_hash
                || (tok == "data:" && matches!(name, "img-src" | "font-src"))
                || (tok == "blob:" && name == "img-src");
            if !ok {
                push_v(violations, 5, rel, line, format!("`Content-Security-Policy` forbids external/unsafe source `{tok}` in `{dir}`"));
            }
        }
    }
}

fn collect_workflow_and_action_files(root: &Path) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for (dir, skip_sub) in [(".github/workflows", true), (".github/actions", false)] {
        let full = root.join(dir);
        if is_real_dir(&full) {
            walk_rel_files(root, &full, &|_| skip_sub, &|p| has_ext_ci(p, &["yml", "yaml"]), &mut out)?;
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn leading_spaces(s: &str) -> usize {
    s.bytes().take_while(|&b| b == b' ').count()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{is_banned_crate_name, run, run_checks};
    use crate::scan_util::test_support::TestDir;

    type Case = (&'static str, &'static str, &'static str, u8, &'static [&'static str]);

    fn clean_fixture(label: &str) -> TestDir {
        let dir = TestDir::new(&format!("tel-{label}"));
        for (rel, content) in [
            ("Cargo.toml", "[workspace]\nmembers = [\"crates/core\", \"crates/mcp\"]\n"),
            ("Cargo.lock", "[[package]]\nname = \"serde\"\nversion = \"1.0.229\"\n"),
            ("crates/core/Cargo.toml", "[package]\nname = \"curvebook-core\"\n[dependencies]\nserde = \"1\"\n"),
            ("crates/core/src/lib.rs", "// TcpStream\n#[cfg(test)] mod t { use std::net::UdpSocket; }\n"),
            (
                "crates/mcp/src/tools.rs",
                "use std::net::{SocketAddr, TcpStream};\npub fn a(p: u16) -> SocketAddr { SocketAddr::from(([127, 0, 0, 1], p)) }\n",
            ),
            ("apps/curvebook-web/index.html", "<!DOCTYPE html><html><body><svg xmlns=\"http://www.w3.org/2000/svg\"></svg></body></html>\n"),
            (
                ".github/workflows/ci.yml",
                "name: CI\non: [push]\npermissions:\n  contents: read\njobs:\n  g:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo test\n",
            ),
            (
                "packaging/web/_headers",
                "/*\n  Content-Security-Policy: default-src 'self'; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'none'\n  Referrer-Policy: no-referrer\n  Permissions-Policy: camera=(), microphone=(), geolocation=()\n  X-Content-Type-Options: nosniff\n  Cross-Origin-Opener-Policy: same-origin\n  Cross-Origin-Embedder-Policy: require-corp\n",
            ),
            (
                "packaging/web/.htaccess",
                "<IfModule mod_headers.c>\n  Header always set Content-Security-Policy \"default-src 'self'; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'none'\"\n  Header always set Referrer-Policy \"no-referrer\"\n  Header always set Permissions-Policy \"camera=(), microphone=(), geolocation=()\"\n  Header always set X-Content-Type-Options \"nosniff\"\n  Header always set Cross-Origin-Opener-Policy \"same-origin\"\n  Header always set Cross-Origin-Embedder-Policy \"require-corp\"\n</IfModule>\n",
            ),
        ] {
            dir.write(rel, content);
        }
        dir
    }

    #[test]
    fn real_workspace_and_checks_1_to_5_matrix() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let (stats, violations) = run_checks(root).unwrap();
        assert!(violations.is_empty(), "{violations:?}");
        assert!(stats.manifests >= 20 && stats.lock_packages >= 20 && stats.workflows >= 4 && stats.header_configs == 2);
        run(root, &[]).unwrap();
        assert!(run(root, &[String::from("--bad")]).is_err());
        assert!(is_banned_crate_name("reqwest") && is_banned_crate_name("sentry-core") && !is_banned_crate_name("hyperbolic"));

        let cases: &[Case] = &[
            (
                "c1-dep",
                "crates/core/Cargo.toml",
                "[package]\nname = \"x\"\n[dependencies]\nweb-sys = { version = \"0.3\", features = [\"WebSocket\"] }\nc = { package = \"reqwest\", version = \"0.12\" }\n",
                1,
                &["WebSocket", "reqwest"],
            ),
            ("c1-lock", "Cargo.lock", "[[package]]\nname = \"sentry-core\"\nversion = \"0.34.0\"\n", 1, &["sentry-core"]),
            (
                "c2-net",
                "crates/core/src/bad.rs",
                "pub fn f() { let _ = std::net::TcpStream::connect(\"1.2.3.4:80\"); let _ = js_sys::eval(\"fetch('x')\"); }\n",
                2,
                &["TcpStream", "fetch"],
            ),
            (
                "c2-loop",
                "crates/mcp/src/tools.rs",
                "use std::net::TcpStream;\npub fn f() { let _ = TcpStream::connect(\"198.51.100.1:443\"); }\n",
                2,
                &["loopback", "198.51.100.1:443"],
            ),
            (
                "c3-web",
                "apps/curvebook-web/index.html",
                "<script>window.fetch('https://bad.test/p'); navigator.sendBeacon('/t'); new WebSocket('//bad.test/s');</script>\n",
                3,
                &["fetch(", "sendbeacon", "https://bad.test/p", "//bad.test/s"],
            ),
            (
                "c4-wf",
                ".github/workflows/ci.yml",
                "name: CI\non: [issues]\njobs:\n  g:\n    permissions:\n      contents: write\n    steps:\n      - uses: actions/checkout@v7\n      - run: |\n          echo \"${{ github.event.issue.title }} ${{ inputs.v }}\"\n          curl -fsSL https://raw.githubusercontent.com/x/y/main/a.sh | bash\n",
                4,
                &["contents: read", "write permission", "40-character", "github.event.", "inputs.*", "sha256sum"],
            ),
            (
                "c5-hdr",
                "packaging/web/_headers",
                "/*\n  Content-Security-Policy: default-src 'self' https:; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'none'\n  Referrer-Policy: no-referrer\n  Permissions-Policy: camera=()\n  X-Content-Type-Options: nosniff\n",
                5,
                &["https:", "geolocation=()", "Cross-Origin-Opener-Policy"],
            ),
        ];
        for &(label, rel, body, check, needles) in cases {
            let dir = clean_fixture(label);
            dir.write(rel, body);
            let (_, hits) = run_checks(dir.path()).unwrap();
            for &n in needles {
                assert!(hits.iter().any(|v| v.check == check && v.detail.contains(n)), "{label}: missing `{n}` in {hits:?}");
            }
        }
        let ok_dir = clean_fixture("narrow-write");
        ok_dir.write(".github/workflows/release.yml", "name: R\npermissions:\n  contents: read\njobs:\n  publish:\n    permissions:\n      contents: write\n      id-token: write\n      attestations: write\n    steps:\n      - run: echo ok\n");
        ok_dir.write(".github/workflows/pages.yml", "name: P\npermissions:\n  contents: read\njobs:\n  deploy:\n    permissions: { pages: write, id-token: write }\n    steps:\n      - run: echo ok\n");
        assert!(run_checks(ok_dir.path()).unwrap().1.is_empty());
    }
}
