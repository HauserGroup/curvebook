//! Dependency layering rules (`docs/architecture.md` › Layers).
//!
//! Enforces that workspace crates depend strictly downward in the layer hierarchy, that external
//! suite crates (`craft-*`) never appear anywhere in the workspace, that UI toolkit crates (`egui`,
//! `eframe`, `winit`, `rfd`, `bevy*`) never appear below L6, and that `lumencraft-testkit` is used
//! solely as a `[dev-dependencies]` entry.

use std::path::Path;

use crate::json::Json;

/// Where a workspace crate sits in the dependency graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Regular layered crate (`0..=6`).
    Layer(u8),
    /// Layer 0, and additionally may depend on no workspace crate at all.
    Standalone,
    /// Test tooling: may depend on anything up to L5; other crates may use it only in `dev-dependencies`.
    Testkit,
    /// Application binaries and workspace build tooling: exempt from upward checks.
    Exempt,
}

impl Class {
    const fn layer(self) -> Option<u8> {
        match self {
            Class::Layer(l) => Some(l),
            Class::Standalone => Some(0),
            Class::Testkit => Some(6),
            Class::Exempt => None,
        }
    }
}

/// Registered workspace crates (package names without the `lumencraft-` prefix).
pub const TABLE: &[(&str, Class)] = &[
    // L0: numerical & colour/style foundations
    ("math", Class::Layer(0)),
    ("palette", Class::Layer(0)),
    ("platform", Class::Layer(0)),
    // L1: pure data model (typed data tables, analysis specs, graph/layout specs)
    ("doc", Class::Layer(1)),
    // L2: scientific domain engines (transforms, biostatistics, curve fitting, text, chart scene compiler)
    ("transform", Class::Layer(2)),
    ("stats", Class::Layer(2)),
    ("fit", Class::Layer(2)),
    ("text", Class::Layer(2)),
    ("chart", Class::Layer(2)),
    // L3: rendering and file formats
    ("render", Class::Layer(3)),
    ("format", Class::Layer(3)),
    ("svg", Class::Layer(3)),
    ("pdf", Class::Layer(3)),
    // L4: interactive spreadsheet, graph and layout tools
    ("tools", Class::Layer(4)),
    // L5: session, history, reactive recalculation DAG and command registry
    ("engine", Class::Layer(5)),
    // L6: thin front-ends (egui shell, stdio MCP server)
    ("ui-egui", Class::Layer(6)),
    ("mcp", Class::Layer(6)),
    // Test helpers (dev-dependency only)
    ("testkit", Class::Testkit),
    // L7: apps and workspace tooling
    ("lumencraft", Class::Exempt),
    ("cli", Class::Exempt),
    ("web", Class::Exempt),
    ("xtask", Class::Exempt),
];

/// Explicit directional edges permitted *within* the same layer (`earlier -> later`).
pub const INTRA_LAYER_ORDER: &[&[&str]] =
    &[&["math", "palette"], &["transform", "chart"], &["stats", "chart"], &["fit", "chart"], &["text", "chart"]];

fn intra_layer_allowed(from: &str, to: &str) -> bool {
    let (from, to) = (short_name(from), short_name(to));
    INTRA_LAYER_ORDER.iter().any(|chain| match (chain.iter().position(|n| *n == from), chain.iter().position(|n| *n == to)) {
        (Some(f), Some(t)) => t < f,
        _ => false,
    })
}

/// External crates that constitute a UI toolkit or windowing dependency.
pub const UI_CRATES: &[&str] = &["egui", "eframe", "winit", "egui_kittest", "rfd", "bevy*"];

/// First layer permitted to depend on [`UI_CRATES`].
pub const UI_MIN_LAYER: u8 = 6;

#[must_use]
pub fn short_name(pkg: &str) -> &str {
    pkg.strip_prefix("lumencraft-").unwrap_or(pkg)
}

#[must_use]
pub fn classify(pkg: &str) -> Option<Class> {
    let s = short_name(pkg);
    TABLE.iter().find(|(n, _)| *n == s).map(|(_, c)| *c)
}

fn is_ui_crate(name: &str) -> bool {
    UI_CRATES.iter().any(|p| match p.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == *p,
    })
}

fn is_suite_crate(name: &str) -> bool {
    name.starts_with("craft-") || name.starts_with("craft_")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepKind {
    Normal,
    Dev,
    Build,
}

#[derive(Debug, Clone)]
pub struct Dep {
    pub name: String,
    pub kind: DepKind,
    pub workspace: bool,
}

#[derive(Debug, Clone)]
pub struct Crate {
    pub name: String,
    pub deps: Vec<Dep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    Unregistered { krate: String },
    Upward { krate: String, dep: String, from: u8, to: u8, kind: DepKind },
    StandaloneHasWorkspaceDep { krate: String, dep: String },
    TestkitAsNormalDep { krate: String },
    UiBelowL6 { krate: String, dep: String, layer: u8 },
    SuiteCrate { krate: String, dep: String },
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Violation::Unregistered { krate } => {
                write!(f, "{krate}: unknown workspace crate; register it in xtask/src/layers.rs TABLE (see docs/architecture.md)")
            }
            Violation::Upward { krate, dep, from, to, kind } => {
                write!(f, "{krate} (L{from}) -> {dep} (L{to}) [{kind:?}]: may only depend on strictly lower layers")
            }
            Violation::StandaloneHasWorkspaceDep { krate, dep } => {
                write!(f, "{krate}: standalone crate must not depend on workspace crate {dep}")
            }
            Violation::TestkitAsNormalDep { krate } => {
                write!(f, "{krate}: lumencraft-testkit may only be a dev-dependency")
            }
            Violation::UiBelowL6 { krate, dep, layer } => {
                write!(f, "{krate} (L{layer}) depends on UI crate `{dep}`; UI toolkits are only allowed in L6+")
            }
            Violation::SuiteCrate { krate, dep } => {
                write!(f, "{krate} depends on external suite crate `{dep}`; LumenCraft is standalone and may not depend on `craft-*` crates")
            }
        }
    }
}

/// Evaluate all layering rules and return sorted, deduplicated violations.
#[must_use]
pub fn check(crates: &[Crate]) -> Vec<Violation> {
    let mut out = Vec::new();
    for c in crates {
        let Some(class) = classify(&c.name) else {
            out.push(Violation::Unregistered { krate: c.name.clone() });
            continue;
        };
        for d in &c.deps {
            if d.name == c.name {
                continue;
            }
            if is_suite_crate(&d.name) {
                out.push(Violation::SuiteCrate { krate: c.name.clone(), dep: d.name.clone() });
                continue;
            }
            if class != Class::Exempt {
                let layer = class.layer().unwrap_or(0);
                check_dep(c, class, layer, d, &mut out);
            }
        }
    }
    out.sort_by_key(ToString::to_string);
    out.dedup();
    out
}

fn check_dep(c: &Crate, class: Class, layer: u8, d: &Dep, out: &mut Vec<Violation>) {
    if d.workspace {
        if class == Class::Standalone {
            out.push(Violation::StandaloneHasWorkspaceDep { krate: c.name.clone(), dep: d.name.clone() });
            return;
        }
        match classify(&d.name) {
            None => {}
            Some(Class::Testkit) if d.kind != DepKind::Dev => {
                out.push(Violation::TestkitAsNormalDep { krate: c.name.clone() });
            }
            Some(Class::Testkit) => {}
            Some(dc) => {
                let to = dc.layer().unwrap_or(u8::MAX);
                if to >= layer && !(to == layer && intra_layer_allowed(&c.name, &d.name)) {
                    out.push(Violation::Upward {
                        krate: c.name.clone(),
                        dep: d.name.clone(),
                        from: layer,
                        to: if to == u8::MAX { 7 } else { to },
                        kind: d.kind,
                    });
                }
            }
        }
    } else if layer < UI_MIN_LAYER && is_ui_crate(&d.name) {
        out.push(Violation::UiBelowL6 { krate: c.name.clone(), dep: d.name.clone(), layer });
    }
}

/// Build the crate graph from `cargo metadata --format-version 1 --no-deps`.
pub fn from_metadata(meta: &Json) -> Result<Vec<Crate>, String> {
    let pkgs = meta.get("packages").and_then(Json::as_array).ok_or_else(|| String::from("metadata: no packages array"))?;
    let members: Vec<&str> = pkgs.iter().filter_map(|p| p.get("name").and_then(Json::as_str)).collect();
    let mut out = Vec::new();
    for p in pkgs {
        let name = p.get("name").and_then(Json::as_str).ok_or_else(|| String::from("package without name"))?.to_owned();
        let mut deps = Vec::new();
        for d in p.get("dependencies").and_then(Json::as_array).into_iter().flatten() {
            let dname = d.get("name").and_then(Json::as_str).unwrap_or_default().to_owned();
            let kind = match d.get("kind").and_then(Json::as_str) {
                Some("dev") => DepKind::Dev,
                Some("build") => DepKind::Build,
                _ => DepKind::Normal,
            };
            let has_path = d.get("path").is_some_and(Json::is_string);
            let workspace = members.contains(&dname.as_str()) || has_path;
            deps.push(Dep { name: dname, kind, workspace });
        }
        out.push(Crate { name, deps });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

#[must_use]
pub fn describe(class: Option<Class>) -> String {
    match class {
        Some(Class::Layer(l)) => format!("L{l}"),
        Some(Class::Standalone) => String::from("L0 standalone"),
        Some(Class::Testkit) => String::from("testkit"),
        Some(Class::Exempt) => String::from("exempt"),
        None => String::from("UNREGISTERED"),
    }
}

/// Shrink-only baseline of workspace files temporarily exempted from the raw `std::process::Command`
/// ban (`R0.10`). Keep empty once all production callers migrate to `lumencraft-platform`.
pub const ALLOWED_RAW_PROCESS_BASELINE: &[&str] = &[];

/// Shrink-only baseline of `crates/ui-egui/src` files temporarily exempted from raw filesystem or
/// process bans (`R0.10`). Keep empty once `os_integration.rs` and `control_io.rs` migrate.
pub const ALLOWED_UI_IO_BASELINE: &[&str] = &[];

const RAW_PROCESS_PATTERNS: &[&str] = &["std::process::Command::new", "std::process::Command", "process::Command::new"];

const UI_IO_PATTERNS: &[&str] = &[
    "std::fs::write",
    "fs::write",
    "std::fs::File::create",
    "File::create",
    "OpenOptions",
    "std::process::Command",
    "process::Command",
    "Command::new",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HygieneViolation {
    pub path: String,
    pub line: usize,
    pub rule: &'static str,
    pub pattern: &'static str,
}

impl std::fmt::Display for HygieneViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: [{}] forbidden `{}`", self.path, self.line, self.rule, self.pattern)
    }
}

/// Check workspace Rust sources for banned raw process spawning and unguarded `ui-egui` file I/O.
#[must_use]
pub fn check_source_hygiene(root: &Path) -> Vec<HygieneViolation> {
    let mut files = Vec::new();
    collect_rs_files(root, root, &mut files);
    files.sort();

    let mut violations = Vec::new();
    for rel in files {
        let exempt_process = is_exempt_process_path(&rel);
        let in_ui_src = is_ui_egui_prod_path(&rel);
        if exempt_process && !in_ui_src {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(root.join(&rel)) else {
            continue;
        };
        let prod = strip_non_prod_rust(&src);
        if !exempt_process && !ALLOWED_RAW_PROCESS_BASELINE.contains(&rel.as_str()) {
            scan_patterns(&rel, &prod, "raw-process", RAW_PROCESS_PATTERNS, &mut violations);
        }
        if in_ui_src && !ALLOWED_UI_IO_BASELINE.contains(&rel.as_str()) {
            scan_patterns(&rel, &prod, "ui-raw-io", UI_IO_PATTERNS, &mut violations);
        }
    }
    violations.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.line.cmp(&b.line)).then_with(|| a.rule.cmp(b.rule)));
    violations
}

fn collect_rs_files(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if path.is_dir() {
            if name_str == "target" || name_str == "dist" || name_str == "corpus" || name_str.starts_with('.') {
                continue;
            }
            collect_rs_files(root, &path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs")
            && let Ok(rel) = path.strip_prefix(root)
        {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

fn is_exempt_process_path(rel: &str) -> bool {
    rel.starts_with("crates/platform/src/")
        || rel.starts_with("xtask/")
        || rel.starts_with("tests/")
        || rel.contains("/tests/")
        || rel.ends_with("_test.rs")
}

fn is_ui_egui_prod_path(rel: &str) -> bool {
    rel.starts_with("crates/ui-egui/src/") && !rel.contains("/tests/") && !rel.ends_with("_test.rs")
}

fn scan_patterns(rel: &str, prod_src: &str, rule: &'static str, patterns: &[&'static str], out: &mut Vec<HygieneViolation>) {
    for (idx, line) in prod_src.lines().enumerate() {
        let mut pos = 0usize;
        while pos < line.len() {
            if let Some((hit_pos, pat)) = next_pattern_match(line, pos, patterns) {
                out.push(HygieneViolation { path: rel.to_owned(), line: idx + 1, rule, pattern: pat });
                pos = hit_pos + pat.len();
            } else {
                break;
            }
        }
    }
}

fn next_pattern_match(line: &str, from: usize, patterns: &[&'static str]) -> Option<(usize, &'static str)> {
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

fn strip_non_prod_rust(src: &str) -> String {
    let no_comments_or_lits = blank_comments_and_literals(src);
    strip_cfg_test_items(&no_comments_or_lits)
}

fn blank_comments_and_literals(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/') {
            skip_line_comment(bytes, &mut i, &mut out);
        } else if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            skip_block_comment(bytes, &mut i, &mut out);
        } else if try_skip_raw_string(bytes, &mut i, &mut out) || try_skip_char_literal(src, bytes, &mut i, &mut out) {
            continue;
        } else if (bytes[i] == b'b' || bytes[i] == b'c') && bytes.get(i + 1) == Some(&b'"') && !prev_is_ident(bytes, i) {
            out.push(bytes[i]);
            i += 1;
            skip_quoted_string(bytes, &mut i, &mut out);
        } else if bytes[i] == b'"' {
            skip_quoted_string(bytes, &mut i, &mut out);
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

fn prev_is_ident(bytes: &[u8], i: usize) -> bool {
    i > 0 && is_ident_byte(bytes[i - 1])
}

fn skip_line_comment(bytes: &[u8], i: &mut usize, out: &mut Vec<u8>) {
    while *i < bytes.len() {
        let b = bytes[*i];
        *i += 1;
        if b == b'\n' {
            out.push(b'\n');
            break;
        }
        out.push(b' ');
    }
}

fn skip_block_comment(bytes: &[u8], i: &mut usize, out: &mut Vec<u8>) {
    let mut depth = 1usize;
    *i += 2;
    out.extend_from_slice(b"  ");
    while *i < bytes.len() && depth > 0 {
        if bytes[*i] == b'/' && bytes.get(*i + 1) == Some(&b'*') {
            depth += 1;
            *i += 2;
            out.extend_from_slice(b"  ");
        } else if bytes[*i] == b'*' && bytes.get(*i + 1) == Some(&b'/') {
            depth -= 1;
            *i += 2;
            out.extend_from_slice(b"  ");
        } else {
            out.push(if bytes[*i] == b'\n' { b'\n' } else { b' ' });
            *i += 1;
        }
    }
}

fn try_skip_raw_string(bytes: &[u8], i: &mut usize, out: &mut Vec<u8>) -> bool {
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
    out.extend_from_slice(&bytes[*i..prefix_end]);
    *i = prefix_end;
    while *i < bytes.len() {
        if bytes[*i] == b'"' && bytes.get(*i + 1..*i + 1 + hashes).is_some_and(|s| s.iter().all(|&b| b == b'#')) {
            out.extend_from_slice(&bytes[*i..*i + 1 + hashes]);
            *i += 1 + hashes;
            break;
        }
        out.push(if bytes[*i] == b'\n' { b'\n' } else { b' ' });
        *i += 1;
    }
    true
}

fn skip_quoted_string(bytes: &[u8], i: &mut usize, out: &mut Vec<u8>) {
    out.push(b'"');
    *i += 1;
    while *i < bytes.len() {
        let b = bytes[*i];
        if b == b'\\' {
            out.push(b' ');
            *i += 1;
            if *i < bytes.len() {
                out.push(if bytes[*i] == b'\n' { b'\n' } else { b' ' });
                *i += 1;
            }
            continue;
        }
        if b == b'"' {
            out.push(b'"');
            *i += 1;
            break;
        }
        out.push(if b == b'\n' { b'\n' } else { b' ' });
        *i += 1;
    }
}

fn try_skip_char_literal(src: &str, bytes: &[u8], i: &mut usize, out: &mut Vec<u8>) -> bool {
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
    for _ in 0..prefix_len {
        out.push(b'\'');
    }
    for _ in (q_pos + 1)..end_q {
        out.push(b' ');
    }
    out.push(b'\'');
    *i = end_q + 1;
    true
}

fn find_escaped_char_end(bytes: &[u8], mut p: usize) -> Option<usize> {
    if p >= bytes.len() || bytes[p] == b'\n' {
        return None;
    }
    p += 1; // Escaped char (`'`, `\`, `n`, `x`, `u`, etc.).
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

fn strip_cfg_test_items(code: &str) -> String {
    let bytes = code.as_bytes();
    let mut out = bytes.to_vec();
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'#'
            && let Some(after_attr) = match_cfg_test_attr(bytes, i)
        {
            let body_start = skip_following_attrs(bytes, after_attr);
            let item_end = find_item_end(bytes, body_start);
            for b in out.iter_mut().take(item_end).skip(i) {
                if *b != b'\n' {
                    *b = b' ';
                }
            }
            i = item_end;
            continue;
        }
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

fn match_cfg_test_attr(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start) != Some(&b'#') || bytes.get(start + 1) != Some(&b'[') {
        return None;
    }
    let mut p = start + 2;
    let mut bracket_depth = 1usize;
    while p < bytes.len() && bracket_depth > 0 {
        match bytes[p] {
            b'[' => bracket_depth += 1,
            b']' => bracket_depth -= 1,
            _ => {}
        }
        p += 1;
    }
    if bracket_depth != 0 {
        return None;
    }
    let inner = std::str::from_utf8(&bytes[start + 2..p - 1]).ok()?;
    let compact: String = inner.chars().filter(|c| !c.is_whitespace()).collect();
    if compact == "cfg(test)" || (compact.starts_with("cfg(all(") && compact.contains("test")) { Some(p) } else { None }
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
    use super::{
        ALLOWED_RAW_PROCESS_BASELINE, ALLOWED_UI_IO_BASELINE, Class, Crate, Dep, DepKind, TABLE, Violation, check, check_source_hygiene, classify,
        from_metadata,
    };
    use crate::json::parse;

    fn c(name: &str, deps: &[(&str, DepKind, bool)]) -> Crate {
        Crate { name: name.into(), deps: deps.iter().map(|(n, k, w)| Dep { name: (*n).into(), kind: *k, workspace: *w }).collect() }
    }

    #[test]
    fn clean_downward_graph_passes() {
        let g = [
            c("lumencraft-math", &[]),
            c("lumencraft-palette", &[("lumencraft-math", DepKind::Normal, true)]),
            c("lumencraft-doc", &[("lumencraft-math", DepKind::Normal, true)]),
            c("lumencraft-stats", &[("lumencraft-doc", DepKind::Normal, true)]),
            c("lumencraft-chart", &[("lumencraft-stats", DepKind::Normal, true)]),
            c("lumencraft-engine", &[("lumencraft-chart", DepKind::Normal, true), ("lumencraft-testkit", DepKind::Dev, true)]),
            c("lumencraft-ui-egui", &[("lumencraft-engine", DepKind::Normal, true), ("egui", DepKind::Normal, false)]),
            c("lumencraft-cli", &[("lumencraft-mcp", DepKind::Normal, true)]),
        ];
        assert!(check(&g).is_empty(), "{:?}", check(&g));
    }

    #[test]
    fn upward_and_sideways_dependencies_are_flagged() {
        let up = check(&[c("lumencraft-doc", &[("lumencraft-engine", DepKind::Normal, true)])]);
        assert!(matches!(up[..], [Violation::Upward { from: 1, to: 5, .. }]));

        let side = check(&[c("lumencraft-stats", &[("lumencraft-fit", DepKind::Normal, true)])]);
        assert!(matches!(side[..], [Violation::Upward { from: 2, to: 2, .. }]));
    }

    #[test]
    fn ui_crates_below_l6_and_testkit_normal_deps_are_flagged() {
        let ui = check(&[c("lumencraft-engine", &[("egui", DepKind::Normal, false)])]);
        assert!(matches!(ui[..], [Violation::UiBelowL6 { layer: 5, .. }]));

        let tk = check(&[c("lumencraft-stats", &[("lumencraft-testkit", DepKind::Normal, true)])]);
        assert!(matches!(tk[..], [Violation::TestkitAsNormalDep { .. }]));

        let suite_ui = check(&[c("lumencraft-ui-egui", &[("craft-ui", DepKind::Normal, false)])]);
        assert!(matches!(suite_ui[..], [Violation::SuiteCrate { .. }]));

        let suite_app = check(&[c("lumencraft", &[("craft-update", DepKind::Normal, false)])]);
        assert!(matches!(suite_app[..], [Violation::SuiteCrate { .. }]));
    }

    #[test]
    fn metadata_parsing_round_trip() {
        let meta = parse(
            r#"{"packages":[
                {"name":"lumencraft-doc","dependencies":[
                    {"name":"lumencraft-math","kind":null,"path":"/x/crates/math"},
                    {"name":"lumencraft-testkit","kind":"dev","path":"/x/crates/testkit"}]},
                {"name":"lumencraft-math","dependencies":[]}
            ]}"#,
        )
        .unwrap();
        let g = from_metadata(&meta).unwrap();
        assert_eq!(g.len(), 2);
        assert!(check(&g).is_empty());
    }

    #[test]
    fn table_registers_all_22_workspace_crates_including_platform() {
        assert_eq!(TABLE.len(), 22);
        assert_eq!(classify("lumencraft-platform"), Some(Class::Layer(0)));
        assert!(ALLOWED_RAW_PROCESS_BASELINE.is_empty());
        assert!(ALLOWED_UI_IO_BASELINE.is_empty());
    }

    #[test]
    fn source_hygiene_flags_raw_command_and_ui_io_but_allows_cfg_test_and_platform() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let root = std::env::temp_dir().join(format!("lumencraft-xtask-hygiene-{}-{nanos}", std::process::id()));
        let write_rel = |rel: &str, content: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        };

        write_rel("crates/engine/src/bad.rs", "pub fn f() {\n    let _ = std::process::Command::new(\"sh\");\n}\n");
        write_rel(
            "crates/engine/src/mid_cfg.rs",
            "#[cfg(test)]\nmod tests {\n    fn t() { let _ = std::process::Command::new(\"sh\"); }\n}\npub fn ok() -> &'static str { \"std::process::Command::new\" }\n",
        );
        write_rel(
            "crates/engine/src/after_cfg.rs",
            "#[cfg(test)]\nuse std::process::Command;\npub fn bad() { let _ = process::Command::new(\"ls\"); }\n",
        );
        write_rel("crates/platform/src/process.rs", "pub fn ok() { let _ = std::process::Command::new(\"sh\"); }\n");
        write_rel("crates/engine/tests/it.rs", "fn t() { let _ = std::process::Command::new(\"sh\"); }\n");
        write_rel("crates/engine/src/foo_test.rs", "fn t() { let _ = std::process::Command::new(\"sh\"); }\n");
        write_rel(
            "crates/ui-egui/src/bad_io.rs",
            "pub fn w() {\n    let _ = std::fs::write(\"a\", b\"b\");\n    let _ = File::create(\"c\");\n    let _ = OpenOptions::new();\n}\n",
        );
        write_rel(
            "crates/ui-egui/src/ok_platform.rs",
            "pub fn ok() {\n    let _ = lumencraft_platform::fs::write_atomic(std::path::Path::new(\"a\"), b\"b\");\n    let _ = lumencraft_platform::ProcessCommand::new(\"pbcopy\");\n}\n",
        );

        let hits = check_source_hygiene(&root);
        let _ = std::fs::remove_dir_all(&root);

        let summary: Vec<String> = hits.iter().map(ToString::to_string).collect();
        assert_eq!(
            summary,
            vec![
                "crates/engine/src/after_cfg.rs:3: [raw-process] forbidden `process::Command::new`",
                "crates/engine/src/bad.rs:2: [raw-process] forbidden `std::process::Command::new`",
                "crates/ui-egui/src/bad_io.rs:2: [ui-raw-io] forbidden `std::fs::write`",
                "crates/ui-egui/src/bad_io.rs:3: [ui-raw-io] forbidden `File::create`",
                "crates/ui-egui/src/bad_io.rs:4: [ui-raw-io] forbidden `OpenOptions`",
            ]
        );
    }
}
