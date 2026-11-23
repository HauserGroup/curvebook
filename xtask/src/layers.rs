//! Dependency layering rules (`docs/architecture.md` › Layers).
//!
//! Enforces that workspace crates depend strictly downward in the layer hierarchy, that external
//! suite crates (`craft-*`) never appear anywhere in the workspace, that UI toolkit crates (`egui`,
//! `eframe`, `winit`, `rfd`, `bevy*`) never appear below L6, and that `curvebook-testkit` is used
//! solely as a `[dev-dependencies]` entry.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use serde_json::Value;

use crate::rust_lex::{blank_comments_and_literals, is_ident_byte, strip_cfg_test_items, strip_non_prod_rust};

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

/// Registered workspace crates (package names without the `curvebook-` prefix).
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
    ("curvebook", Class::Exempt),
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
pub const UI_CRATES: &[&str] = &["egui*", "epaint*", "emath", "ecolor", "eframe", "winit", "rfd", "wgpu*", "bevy*"];

/// First layer permitted to depend on [`UI_CRATES`].
pub const UI_MIN_LAYER: u8 = 6;

#[must_use]
pub fn short_name(pkg: &str) -> &str {
    pkg.strip_prefix("curvebook-").unwrap_or(pkg)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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
                write!(f, "{krate}: curvebook-testkit may only be a dev-dependency")
            }
            Violation::UiBelowL6 { krate, dep, layer } => {
                write!(f, "{krate} (L{layer}) depends on UI crate `{dep}`; UI toolkits are only allowed in L6+")
            }
            Violation::SuiteCrate { krate, dep } => {
                write!(f, "{krate} depends on external suite crate `{dep}`; CurveBook is standalone and may not depend on `craft-*` crates")
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
            if class == Class::Exempt {
                if classify(&d.name) == Some(Class::Testkit) && d.kind != DepKind::Dev && short_name(&c.name) != "xtask" {
                    out.push(Violation::TestkitAsNormalDep { krate: c.name.clone() });
                }
                continue;
            }
            let layer = class.layer().unwrap_or(0);
            check_dep(c, class, layer, d, &mut out);
        }
    }
    out.sort_by_key(ToString::to_string);
    out.dedup();
    out
}

fn check_dep(c: &Crate, class: Class, layer: u8, d: &Dep, out: &mut Vec<Violation>) {
    if layer < UI_MIN_LAYER && is_ui_crate(&d.name) {
        out.push(Violation::UiBelowL6 { krate: c.name.clone(), dep: d.name.clone(), layer });
    }
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
    }
}

/// Build the crate graph from `cargo metadata --format-version 1` (resolved graph when present,
/// or direct manifest dependencies for minimal synthetic test JSON).
pub fn from_metadata(meta: &Value) -> Result<Vec<Crate>, String> {
    let pkgs = meta.get("packages").and_then(Value::as_array).ok_or_else(|| String::from("metadata: no packages array"))?;
    if let Some(resolve_nodes) = meta.get("resolve").and_then(|r| r.get("nodes")).and_then(Value::as_array) {
        return from_resolved_metadata(meta, pkgs, resolve_nodes);
    }
    from_declared_metadata(meta, pkgs)
}

fn from_declared_metadata(meta: &Value, pkgs: &[Value]) -> Result<Vec<Crate>, String> {
    let member_ids: Option<BTreeSet<&str>> =
        meta.get("workspace_members").and_then(Value::as_array).map(|arr| arr.iter().filter_map(Value::as_str).collect());
    let mut member_names = BTreeSet::new();
    for p in pkgs {
        let name = p.get("name").and_then(Value::as_str).ok_or_else(|| String::from("package without name"))?;
        let id = p.get("id").and_then(Value::as_str);
        let is_member = match (&member_ids, id) {
            (Some(ids), Some(pid)) => ids.contains(pid),
            _ => true,
        };
        if is_member {
            member_names.insert(name);
        }
    }

    let mut out = Vec::new();
    for p in pkgs {
        let name = p.get("name").and_then(Value::as_str).ok_or_else(|| String::from("package without name"))?.to_owned();
        if !member_names.contains(name.as_str()) {
            continue;
        }
        let deps = collect_declared_deps(p, &member_names);
        out.push(Crate { name, deps });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn collect_declared_deps(pkg: &Value, member_names: &BTreeSet<&str>) -> Vec<Dep> {
    let mut deps = Vec::new();
    for d in pkg.get("dependencies").and_then(Value::as_array).into_iter().flatten() {
        let dname = d.get("name").and_then(Value::as_str).unwrap_or_default().to_owned();
        let kind = parse_dep_kind_str(d.get("kind").and_then(Value::as_str));
        let is_local_curvebook = d.get("path").is_some_and(Value::is_string) && dname.starts_with("curvebook-");
        let workspace = member_names.contains(dname.as_str()) || classify(&dname).is_some() || is_local_curvebook;
        deps.push(Dep { name: dname, kind, workspace });
    }
    deps
}

fn parse_dep_kind_str(kind: Option<&str>) -> DepKind {
    match kind {
        Some("dev") => DepKind::Dev,
        Some("build") => DepKind::Build,
        _ => DepKind::Normal,
    }
}

fn from_resolved_metadata(meta: &Value, pkgs: &[Value], resolve_nodes: &[Value]) -> Result<Vec<Crate>, String> {
    let mut pkg_by_id = BTreeMap::new();
    let mut pkg_name_by_id = BTreeMap::new();
    for p in pkgs {
        let id = p.get("id").and_then(Value::as_str).ok_or_else(|| String::from("package without id"))?;
        let name = p.get("name").and_then(Value::as_str).ok_or_else(|| String::from("package without name"))?;
        pkg_by_id.insert(id, p);
        pkg_name_by_id.insert(id, name);
    }

    let member_ids: Vec<&str> = match meta.get("workspace_members").and_then(Value::as_array) {
        Some(arr) => arr.iter().filter_map(Value::as_str).collect(),
        None => pkg_by_id.keys().copied().collect(),
    };
    let member_id_set: BTreeSet<&str> = member_ids.iter().copied().collect();
    let member_names: BTreeSet<&str> = member_ids.iter().filter_map(|id| pkg_name_by_id.get(id).copied()).collect();

    let mut node_by_id = BTreeMap::new();
    for node in resolve_nodes {
        if let Some(id) = node.get("id").and_then(Value::as_str) {
            node_by_id.insert(id, node);
        }
    }

    let mut out = Vec::with_capacity(member_ids.len());
    for mid in member_ids {
        let Some(&mname) = pkg_name_by_id.get(mid) else {
            continue;
        };
        let mut seen_deps: BTreeSet<(String, DepKind, bool)> = BTreeSet::new();
        if let Some(&pkg) = pkg_by_id.get(mid) {
            for dep in collect_declared_deps(pkg, &member_names) {
                seen_deps.insert((dep.name, dep.kind, dep.workspace));
            }
        }
        collect_resolved_transitive_deps(mid, &node_by_id, &pkg_name_by_id, &member_id_set, &mut seen_deps);
        let deps = seen_deps.into_iter().map(|(name, kind, workspace)| Dep { name, kind, workspace }).collect();
        out.push(Crate { name: mname.to_owned(), deps });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn collect_resolved_transitive_deps(
    root_id: &str,
    node_by_id: &BTreeMap<&str, &Value>,
    pkg_name_by_id: &BTreeMap<&str, &str>,
    member_id_set: &BTreeSet<&str>,
    out: &mut BTreeSet<(String, DepKind, bool)>,
) {
    let Some(&root_node) = node_by_id.get(root_id) else {
        return;
    };
    let mut queue: VecDeque<(&str, DepKind)> = VecDeque::new();
    for (dep_id, kinds) in node_edges(root_node) {
        for kind in kinds {
            queue.push_back((dep_id, kind));
        }
    }
    let mut visited: BTreeSet<(&str, DepKind)> = BTreeSet::new();
    while let Some((pid, root_kind)) = queue.pop_front() {
        if pid == root_id || !visited.insert((pid, root_kind)) {
            continue;
        }
        let Some(&pname) = pkg_name_by_id.get(pid) else {
            continue;
        };
        let is_ws = member_id_set.contains(pid);
        out.insert((pname.to_owned(), root_kind, is_ws));

        // Do not walk past Testkit or Exempt boundaries for actual workspace crates:
        // a lower layer dev-depending on testkit does not inherit testkit's own dependencies on
        // higher layers, while non-workspace crates named like a workspace crate are still walked.
        if is_ws && matches!(classify(pname), Some(Class::Testkit | Class::Exempt)) {
            continue;
        }
        if let Some(&sub_node) = node_by_id.get(pid) {
            for (sub_id, skinds) in node_edges(sub_node) {
                if skinds.into_iter().any(|k| k != DepKind::Dev) {
                    queue.push_back((sub_id, root_kind));
                }
            }
        }
    }
}

fn node_edges(node: &Value) -> Vec<(&str, Vec<DepKind>)> {
    if let Some(deps) = node.get("deps").and_then(Value::as_array) {
        let mut edges = Vec::with_capacity(deps.len());
        for d in deps {
            let Some(pkg) = d.get("pkg").and_then(Value::as_str) else {
                continue;
            };
            let mut kinds = Vec::new();
            for dk in d.get("dep_kinds").and_then(Value::as_array).into_iter().flatten() {
                kinds.push(parse_dep_kind_str(dk.get("kind").and_then(Value::as_str)));
            }
            if kinds.is_empty() {
                kinds.push(DepKind::Normal);
            }
            edges.push((pkg, kinds));
        }
        return edges;
    }
    node.get("dependencies").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(|id| (id, vec![DepKind::Normal])).collect()
}

/// Inspect workspace `Cargo.toml` `[patch]` and `[replace]` sections for banned suite crates
/// or attempts to patch UI/workspace crates invisibly.
#[must_use]
pub fn check_manifest_patches(root: &Path) -> Vec<Violation> {
    let Ok(text) = std::fs::read_to_string(root.join("Cargo.toml")) else {
        return Vec::new();
    };
    scan_patch_sections(&text)
}

#[must_use]
pub fn scan_patch_sections(cargo_toml: &str) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut in_patch_or_replace = false;
    let mut in_patch_crate_subtable = false;
    for raw in cargo_toml.lines() {
        let line = raw.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        if let Some(inner) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let sec = inner.trim();
            let segs = split_toml_table_segments(sec);
            in_patch_or_replace = sec == "replace" || segs.first().is_some_and(|s| s == "patch");
            in_patch_crate_subtable = false;
            if segs.first().is_some_and(|s| s == "patch")
                && let Some(crate_name) = segs.get(2)
            {
                in_patch_crate_subtable = true;
                if is_suite_crate(crate_name) {
                    violations.push(Violation::SuiteCrate { krate: String::from("Cargo.toml[patch/replace]"), dep: crate_name.clone() });
                }
            }
            continue;
        }
        if !in_patch_or_replace {
            continue;
        }
        if let Some((lhs, rhs)) = line.split_once('=') {
            let lhs_clean = lhs.trim().trim_matches('"').trim_matches('\'');
            if in_patch_crate_subtable && lhs_clean == "package" {
                let pkg_name = rhs.trim().trim_matches('"').trim_matches('\'');
                if is_suite_crate(pkg_name) {
                    violations.push(Violation::SuiteCrate { krate: String::from("Cargo.toml[patch/replace]"), dep: pkg_name.to_owned() });
                }
                continue;
            }
            if !in_patch_crate_subtable {
                let pkg_name = lhs_clean.split(':').next().unwrap_or(lhs_clean).trim();
                if is_suite_crate(pkg_name) {
                    violations.push(Violation::SuiteCrate { krate: String::from("Cargo.toml[patch/replace]"), dep: pkg_name.to_owned() });
                }
            }
        }
    }
    violations
}

fn split_toml_table_segments(sec: &str) -> Vec<String> {
    let mut segs = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in sec.chars() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                cur.push(ch);
            }
        } else if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else if ch == '.' {
            let trimmed = cur.trim();
            if !trimmed.is_empty() {
                segs.push(trimmed.to_owned());
            }
            cur.clear();
        } else {
            cur.push(ch);
        }
    }
    let trimmed = cur.trim();
    if !trimmed.is_empty() {
        segs.push(trimmed.to_owned());
    }
    segs
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
/// ban (`R0.10`). Keep empty once all production callers migrate to `curvebook-platform`.
pub const ALLOWED_RAW_PROCESS_BASELINE: &[&str] = &[];

/// Shrink-only baseline of `crates/ui-egui/src` files temporarily exempted from raw filesystem or
/// process bans (`R0.10`). Keep empty once `os_integration.rs` and `control_io.rs` migrate.
pub const ALLOWED_UI_IO_BASELINE: &[&str] = &[];

const RAW_PROCESS_PATTERNS: &[&str] =
    &["std::process::Command::new", "std::process::Command", "process::Command::new", "process::Command", "std::process::*"];

const UI_IO_PATTERNS: &[&str] = &[
    "std::fs::write",
    "fs::write",
    "std::fs::File::create_new",
    "File::create_new",
    "std::fs::File::create",
    "File::create",
    "std::fs::remove_file",
    "fs::remove_file",
    "std::fs::remove_dir_all",
    "fs::remove_dir_all",
    "std::fs::remove_dir",
    "fs::remove_dir",
    "std::fs::create_dir_all",
    "fs::create_dir_all",
    "std::fs::create_dir",
    "fs::create_dir",
    "std::fs::set_permissions",
    "fs::set_permissions",
    "std::fs::hard_link",
    "fs::hard_link",
    "std::os::unix::fs::symlink",
    "os::unix::fs::symlink",
    "unix::fs::symlink",
    "std::os::windows::fs::symlink_file",
    "std::os::windows::fs::symlink_dir",
    "windows::fs::symlink_file",
    "windows::fs::symlink_dir",
    "std::fs::copy",
    "fs::copy",
    "std::fs::rename",
    "fs::rename",
    "OpenOptions",
    "std::process::Command",
    "process::Command",
    "Command::new",
    "std::process::*",
    "std::fs::*",
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
            scan_use_statements(
                &rel,
                &prod,
                "raw-process",
                &["std::process::Command", "process::Command", "std::process::*"],
                &["std::process", "process"],
                &mut violations,
            );
        }
        if in_ui_src && !ALLOWED_UI_IO_BASELINE.contains(&rel.as_str()) {
            scan_patterns(&rel, &prod, "ui-raw-io", UI_IO_PATTERNS, &mut violations);
            scan_use_statements(
                &rel,
                &prod,
                "ui-raw-io",
                &[
                    "std::fs::write",
                    "fs::write",
                    "std::fs::File",
                    "fs::File",
                    "std::fs::OpenOptions",
                    "fs::OpenOptions",
                    "std::fs::remove_file",
                    "fs::remove_file",
                    "std::fs::remove_dir_all",
                    "fs::remove_dir_all",
                    "std::fs::remove_dir",
                    "fs::remove_dir",
                    "std::fs::create_dir_all",
                    "fs::create_dir_all",
                    "std::fs::create_dir",
                    "fs::create_dir",
                    "std::fs::set_permissions",
                    "fs::set_permissions",
                    "std::fs::hard_link",
                    "fs::hard_link",
                    "std::os::unix::fs::symlink",
                    "os::unix::fs::symlink",
                    "unix::fs::symlink",
                    "std::os::windows::fs::symlink_file",
                    "std::os::windows::fs::symlink_dir",
                    "windows::fs::symlink_file",
                    "windows::fs::symlink_dir",
                    "std::fs::copy",
                    "fs::copy",
                    "std::fs::rename",
                    "fs::rename",
                    "std::fs::*",
                    "std::process::Command",
                    "process::Command",
                    "std::process::*",
                ],
                &["std::fs", "fs", "std::os::unix::fs", "os::unix::fs", "unix::fs", "std::os::windows::fs", "std::process", "process"],
                &mut violations,
            );
        }
    }
    violations.sort_by(|a, b| {
        a.path.cmp(&b.path).then_with(|| a.line.cmp(&b.line)).then_with(|| a.rule.cmp(b.rule)).then_with(|| a.pattern.cmp(b.pattern))
    });
    violations.dedup();
    violations
}

const LEGACY_INDEXING_ALLOWLIST: &[&str] =
    &["crates/stats", "crates/fit", "crates/transform", "crates/chart", "crates/text", "crates/ui-egui", "crates/testkit", "xtask"];

const DOC_SCREENSHOTS_REL: &str = "crates/ui-egui/examples/doc_screenshots.rs";

/// Check workspace Clippy lint policy (`FA-SEC23-03`):
/// - `Cargo.toml` denies `string_slice` globally under `[workspace.lints.clippy]`.
/// - All non-allowlisted strict crate roots (`crates/*/src/{lib,main}.rs`, `apps/*/src/{lib,main}.rs`)
///   and `crates/ui-egui/examples/doc_screenshots.rs` enforce `#![deny(clippy::indexing_slicing)]`.
/// - No `src/**` file (or `doc_screenshots.rs`) allows `clippy::string_slice`, and no strict crate
///   `src/**` file (or `doc_screenshots.rs`) allows `clippy::indexing_slicing`.
#[must_use]
pub fn check_lint_policy(root: &Path) -> Vec<HygieneViolation> {
    let mut violations = Vec::new();
    check_workspace_string_slice_lint(root, &mut violations);
    check_strict_crate_indexing_deny(root, &mut violations);
    check_lint_allow_escapes(root, &mut violations);
    violations.sort_by(|a, b| {
        a.path.cmp(&b.path).then_with(|| a.line.cmp(&b.line)).then_with(|| a.rule.cmp(b.rule)).then_with(|| a.pattern.cmp(b.pattern))
    });
    violations.dedup();
    violations
}

fn check_workspace_string_slice_lint(root: &Path, violations: &mut Vec<HygieneViolation>) {
    let cargo_path = root.join("Cargo.toml");
    let Ok(toml_src) = std::fs::read_to_string(&cargo_path) else {
        violations.push(HygieneViolation { path: String::from("Cargo.toml"), line: 1, rule: "lint-policy", pattern: "string_slice = \"deny\"" });
        return;
    };
    let mut in_clippy_section = false;
    for raw in toml_src.lines() {
        let code = raw.split_once('#').map_or(raw, |(head, _)| head).trim();
        if code.is_empty() {
            continue;
        }
        if code.starts_with('[') && code.ends_with(']') {
            in_clippy_section = code == "[workspace.lints.clippy]";
            continue;
        }
        if in_clippy_section
            && let Some((key, val)) = code.split_once('=')
            && key.trim() == "string_slice"
            && val.trim() == "\"deny\""
        {
            return;
        }
    }
    violations.push(HygieneViolation { path: String::from("Cargo.toml"), line: 1, rule: "lint-policy", pattern: "string_slice = \"deny\"" });
}

fn check_strict_crate_indexing_deny(root: &Path, violations: &mut Vec<HygieneViolation>) {
    for group in ["crates", "apps"] {
        let Ok(entries) = std::fs::read_dir(root.join(group)) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|ft| ft.is_dir()) {
                continue;
            }
            let dir_name = entry.file_name();
            let Some(dir_str) = dir_name.to_str() else {
                continue;
            };
            let pkg_rel = format!("{group}/{dir_str}");
            if LEGACY_INDEXING_ALLOWLIST.contains(&pkg_rel.as_str()) {
                continue;
            }
            for root_file in ["src/lib.rs", "src/main.rs"] {
                let rel = format!("{pkg_rel}/{root_file}");
                if root.join(&rel).is_file() {
                    require_indexing_slicing_deny(root, &rel, violations);
                }
            }
        }
    }
    let screenshots = root.join(DOC_SCREENSHOTS_REL);
    if screenshots.is_file() || root.join("crates/ui-egui").is_dir() {
        require_indexing_slicing_deny(root, DOC_SCREENSHOTS_REL, violations);
    }
}

fn require_indexing_slicing_deny(root: &Path, rel: &str, violations: &mut Vec<HygieneViolation>) {
    let Ok(src) = std::fs::read_to_string(root.join(rel)) else {
        violations.push(HygieneViolation { path: rel.to_owned(), line: 1, rule: "lint-policy", pattern: "#![deny(clippy::indexing_slicing)]" });
        return;
    };
    let prod = strip_non_prod_rust(&src);
    let compact: String = prod.chars().filter(|ch| !ch.is_whitespace()).collect();
    if !compact.contains("#![deny(clippy::indexing_slicing)]") {
        violations.push(HygieneViolation { path: rel.to_owned(), line: 1, rule: "lint-policy", pattern: "#![deny(clippy::indexing_slicing)]" });
    }
}

fn check_lint_allow_escapes(root: &Path, violations: &mut Vec<HygieneViolation>) {
    let mut files = Vec::new();
    collect_rs_files(root, root, &mut files);
    files.sort();
    for rel in files {
        let in_strict_src = is_strict_src_path(&rel);
        let in_any_src = is_workspace_src_path(&rel);
        if !in_any_src && rel != DOC_SCREENSHOTS_REL {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(root.join(&rel)) else {
            continue;
        };
        let blanked = blank_comments_and_literals(&src);
        if has_lint_allow(&blanked, "clippy::string_slice") {
            let line = first_matching_line(&src, "string_slice").unwrap_or(1);
            violations.push(HygieneViolation { path: rel.clone(), line, rule: "lint-policy", pattern: "allow(clippy::string_slice)" });
        }
        if in_strict_src || rel == DOC_SCREENSHOTS_REL {
            let prod = strip_cfg_test_items(&blanked);
            if has_lint_allow(&prod, "clippy::indexing_slicing") {
                let line = first_matching_line(&src, "indexing_slicing").unwrap_or(1);
                violations.push(HygieneViolation { path: rel, line, rule: "lint-policy", pattern: "allow(clippy::indexing_slicing)" });
            }
        }
    }
}

fn is_workspace_src_path(rel: &str) -> bool {
    (rel.starts_with("crates/") || rel.starts_with("apps/")) && rel.contains("/src/") || rel.starts_with("xtask/src/")
}

fn is_strict_src_path(rel: &str) -> bool {
    if !is_workspace_src_path(rel) {
        return false;
    }
    !LEGACY_INDEXING_ALLOWLIST.iter().any(|prefix| rel == *prefix || rel.starts_with(&format!("{prefix}/")))
}

fn has_lint_allow(src: &str, lint: &str) -> bool {
    let compact: String = src.chars().filter(|ch| !ch.is_whitespace()).collect();
    let mut rest = compact.as_str();
    while let Some((_, after)) = rest.split_once("allow(") {
        if let Some((inside, tail)) = after.split_once(')') {
            if inside.split(',').any(|item| item == lint) {
                return true;
            }
            rest = tail;
        } else {
            break;
        }
    }
    false
}

fn first_matching_line(src: &str, needle: &str) -> Option<usize> {
    src.lines().enumerate().find(|(_, line)| line.contains(needle)).map(|(idx, _)| idx + 1)
}

fn collect_rs_files(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if ft.is_dir() {
            if name_str == "target" || name_str == "dist" || name_str == "corpus" || name_str.starts_with('.') {
                continue;
            }
            collect_rs_files(root, &path, out);
        } else if ft.is_file()
            && path.extension().and_then(|e| e.to_str()) == Some("rs")
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

pub(crate) fn collapse_colon_whitespace(src: &str) -> (String, Vec<usize>) {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut lines = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    let mut line = 1usize;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            let mut j = i;
            let mut skipped_lines = 0usize;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                if bytes[j] == b'\n' {
                    skipped_lines += 1;
                }
                j += 1;
            }
            if bytes[j..].starts_with(b"::") || out.ends_with("::") {
                line += skipped_lines;
                i = j;
                continue;
            }
        }
        let ch = src.get(i..).and_then(|s| s.chars().next()).unwrap_or(' ');
        let ch_len = ch.len_utf8();
        out.push(ch);
        for _ in 0..ch_len {
            lines.push(line);
        }
        if ch == '\n' {
            line += 1;
        }
        i += ch_len;
    }
    (out, lines)
}

fn scan_patterns(rel: &str, prod_src: &str, rule: &'static str, patterns: &[&'static str], out: &mut Vec<HygieneViolation>) {
    let (compact, line_of_byte) = collapse_colon_whitespace(prod_src);
    let mut pos = 0usize;
    while pos < compact.len() {
        if let Some((hit_pos, pat)) = next_pattern_match(&compact, pos, patterns) {
            let line = line_of_byte.get(hit_pos).copied().unwrap_or(1);
            out.push(HygieneViolation { path: rel.to_owned(), line, rule, pattern: pat });
            pos = hit_pos + pat.len();
        } else {
            break;
        }
    }
}

fn next_pattern_match(line: &str, from: usize, patterns: &[&'static str]) -> Option<(usize, &'static str)> {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UseBinding {
    pub(crate) path: String,
    pub(crate) alias: Option<String>,
}

fn scan_use_statements(
    rel: &str,
    prod_src: &str,
    rule: &'static str,
    banned_paths: &[&'static str],
    banned_module_aliases: &[&'static str],
    out: &mut Vec<HygieneViolation>,
) {
    for (line, stmt) in collect_use_statements(prod_src) {
        let mut expanded = Vec::new();
        expand_use_tree_items("", &stmt, &mut expanded);
        for item in expanded {
            let norm = item.path.strip_prefix("::").unwrap_or(&item.path);
            for &pat in banned_paths {
                if norm == pat {
                    out.push(HygieneViolation { path: rel.to_owned(), line, rule, pattern: pat });
                }
            }
            if item.alias.is_some() {
                for &mod_pat in banned_module_aliases {
                    if norm == mod_pat {
                        out.push(HygieneViolation { path: rel.to_owned(), line, rule, pattern: mod_pat });
                    }
                }
            }
        }
    }
}

pub(crate) fn collect_use_statements(prod_src: &str) -> Vec<(usize, String)> {
    let bytes = prod_src.as_bytes();
    let mut stmts = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        let is_kw =
            bytes[i..].starts_with(b"use") && (i == 0 || !is_ident_byte(bytes[i - 1])) && bytes.get(i + 3).is_some_and(|b| b.is_ascii_whitespace());
        if !is_kw {
            i += 1;
            continue;
        }
        let stmt_line = line;
        i += 3;
        let mut buf = String::new();
        while i < bytes.len() && bytes[i] != b';' {
            if bytes[i] == b'\n' {
                line += 1;
            }
            if bytes[i].is_ascii_whitespace() {
                if !buf.is_empty() && !buf.ends_with(' ') {
                    buf.push(' ');
                }
            } else {
                buf.push(char::from(bytes[i]));
            }
            i += 1;
        }
        if bytes.get(i) == Some(&b';') {
            stmts.push((stmt_line, buf));
            i += 1;
        }
    }
    stmts
}

pub(crate) fn expand_use_tree_items(prefix: &str, tree: &str, out: &mut Vec<UseBinding>) {
    let trimmed = tree.trim();
    if trimmed.is_empty() {
        return;
    }
    if let Some((head, inner_with_brace)) = trimmed.split_once('{')
        && let Some(inner) = inner_with_brace.strip_suffix('}')
    {
        let sub_prefix: String = head.chars().filter(|c| !c.is_whitespace()).collect();
        let next_prefix = join_use_path(prefix, sub_prefix.trim_end_matches("::"));
        for part in split_brace_items(inner) {
            expand_use_tree_items(&next_prefix, part, out);
        }
        return;
    }
    let (item_path, alias) = split_use_alias(trimmed);
    let compact: String = item_path.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.is_empty() {
        return;
    }
    let resolved = if compact == "self" { prefix.to_owned() } else { join_use_path(prefix, &compact) };
    if !resolved.is_empty() {
        out.push(UseBinding { path: resolved, alias: alias.map(str::to_owned) });
    }
}

fn split_use_alias(item: &str) -> (&str, Option<&str>) {
    let bytes = item.as_bytes();
    let mut i = 0usize;
    while i + 2 <= bytes.len() {
        if &bytes[i..i + 2] == b"as" {
            let left_ok = i > 0 && bytes[i - 1].is_ascii_whitespace();
            let right_ok = bytes.get(i + 2).is_some_and(u8::is_ascii_whitespace);
            if left_ok
                && right_ok
                && let Some((head, after_as)) = item.split_at_checked(i)
                && let Some(tail) = after_as.strip_prefix("as")
            {
                let alias = tail.trim();
                return (head.trim(), (!alias.is_empty()).then_some(alias));
            }
        }
        i += 1;
    }
    (item.trim(), None)
}

fn join_use_path(prefix: &str, seg: &str) -> String {
    match (prefix.is_empty(), seg.is_empty()) {
        (true, _) => seg.to_owned(),
        (false, true) => prefix.to_owned(),
        (false, false) => format!("{prefix}::{seg}"),
    }
}

fn split_brace_items(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, ch) in s.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(s.get(start..i).unwrap_or(""));
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(s.get(start..).unwrap_or(""));
    parts
}

#[cfg(test)]
mod tests {
    use super::{
        ALLOWED_RAW_PROCESS_BASELINE, ALLOWED_UI_IO_BASELINE, Class, Crate, Dep, DepKind, TABLE, Violation, check, check_lint_policy,
        check_source_hygiene, classify, from_metadata, scan_patch_sections,
    };
    fn parse(s: &str) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::from_str(s)
    }

    fn c(name: &str, deps: &[(&str, DepKind, bool)]) -> Crate {
        Crate { name: name.into(), deps: deps.iter().map(|(n, k, w)| Dep { name: (*n).into(), kind: *k, workspace: *w }).collect() }
    }

    #[test]
    fn clean_downward_graph_passes() {
        let g = [
            c("curvebook-math", &[]),
            c("curvebook-palette", &[("curvebook-math", DepKind::Normal, true)]),
            c("curvebook-doc", &[("curvebook-math", DepKind::Normal, true)]),
            c("curvebook-stats", &[("curvebook-doc", DepKind::Normal, true)]),
            c("curvebook-chart", &[("curvebook-stats", DepKind::Normal, true)]),
            c("curvebook-engine", &[("curvebook-chart", DepKind::Normal, true), ("curvebook-testkit", DepKind::Dev, true)]),
            c("curvebook-ui-egui", &[("curvebook-engine", DepKind::Normal, true), ("egui", DepKind::Normal, false)]),
            c("curvebook-cli", &[("curvebook-mcp", DepKind::Normal, true)]),
        ];
        assert!(check(&g).is_empty(), "{:?}", check(&g));
    }

    #[test]
    fn upward_and_sideways_dependencies_are_flagged() {
        let up = check(&[c("curvebook-doc", &[("curvebook-engine", DepKind::Normal, true)])]);
        assert!(matches!(up[..], [Violation::Upward { from: 1, to: 5, .. }]));

        let side = check(&[c("curvebook-stats", &[("curvebook-fit", DepKind::Normal, true)])]);
        assert!(matches!(side[..], [Violation::Upward { from: 2, to: 2, .. }]));
    }

    #[test]
    fn ui_crates_below_l6_and_testkit_normal_deps_are_flagged() {
        let ui = check(&[c("curvebook-engine", &[("egui", DepKind::Normal, false)])]);
        assert!(matches!(ui[..], [Violation::UiBelowL6 { layer: 5, .. }]));

        let tk = check(&[c("curvebook-stats", &[("curvebook-testkit", DepKind::Normal, true)])]);
        assert!(matches!(tk[..], [Violation::TestkitAsNormalDep { .. }]));

        let suite_ui = check(&[c("curvebook-ui-egui", &[("craft-ui", DepKind::Normal, false)])]);
        assert!(matches!(suite_ui[..], [Violation::SuiteCrate { .. }]));

        let suite_app = check(&[c("curvebook", &[("craft-update", DepKind::Normal, false)])]);
        assert!(matches!(suite_app[..], [Violation::SuiteCrate { .. }]));
    }

    #[test]
    fn metadata_parsing_round_trip() {
        let meta = parse(
            r#"{"packages":[
                {"name":"curvebook-doc","dependencies":[
                    {"name":"curvebook-math","kind":null,"path":"/x/crates/math"},
                    {"name":"curvebook-testkit","kind":"dev","path":"/x/crates/testkit"}]},
                {"name":"curvebook-math","dependencies":[]}
            ]}"#,
        )
        .unwrap();
        let g = from_metadata(&meta).unwrap();
        assert_eq!(g.len(), 2);
        assert!(check(&g).is_empty());
    }

    #[test]
    fn ui_crate_family_and_vendored_path_deps_below_l6_are_flagged() {
        for ui_dep in
            ["egui", "egui_extras", "epaint", "epaint_default_fonts", "emath", "ecolor", "eframe", "winit", "rfd", "wgpu", "wgpu-core", "bevy_math"]
        {
            let hits = check(&[c("curvebook-engine", &[(ui_dep, DepKind::Build, true)])]);
            assert!(matches!(hits[..], [Violation::UiBelowL6 { layer: 5, .. }]), "expected UiBelowL6 for {ui_dep}, got {hits:?}");
        }
        // L6 crates may depend on UI crates.
        assert!(check(&[c("curvebook-ui-egui", &[("egui-winit", DepKind::Normal, false)])]).is_empty());

        // Vendored path dependency in metadata JSON has a `"path"` field without making it a workspace crate,
        // and UI checks still flag it below L6 (e.g., `egui = { path = "/x/vendor/egui" }`).
        let meta = parse(
            r#"{"packages":[
                {"name":"curvebook-fit","id":"fit 0.1.0","dependencies":[
                    {"name":"egui","kind":null,"path":"/x/vendor/egui"}
                ]}
            ],"workspace_members":["fit 0.1.0"]}"#,
        )
        .unwrap();
        let g = from_metadata(&meta).unwrap();
        let hits = check(&g);
        assert!(matches!(hits[..], [Violation::UiBelowL6 { layer: 2, .. }]), "got {hits:?}");
    }

    #[test]
    fn resolved_metadata_catches_transitive_ui_and_skips_dev_testkit_propagation() {
        let meta = parse(
            r#"{
                "packages": [
                    {"name":"curvebook-math","id":"math 0.1.0","dependencies":[]},
                    {"name":"curvebook-doc","id":"doc 0.1.0","dependencies":[]},
                    {"name":"curvebook-stats","id":"stats 0.1.0","dependencies":[]},
                    {"name":"curvebook-testkit","id":"testkit 0.1.0","dependencies":[]},
                    {"name":"helper-crate","id":"helper 0.1.0","dependencies":[]},
                    {"name":"winit","id":"winit 0.30.0","dependencies":[]}
                ],
                "workspace_members": ["math 0.1.0", "doc 0.1.0", "stats 0.1.0", "testkit 0.1.0"],
                "resolve": {
                    "nodes": [
                        {
                            "id": "math 0.1.0",
                            "deps": [
                                {"pkg": "testkit 0.1.0", "dep_kinds": [{"kind": "dev"}]}
                            ]
                        },
                        {
                            "id": "testkit 0.1.0",
                            "deps": [
                                {"pkg": "doc 0.1.0", "dep_kinds": [{"kind": null}]}
                            ]
                        },
                        {"id": "doc 0.1.0", "deps": []},
                        {
                            "id": "stats 0.1.0",
                            "deps": [
                                {"pkg": "helper 0.1.0", "dep_kinds": [{"kind": null}]}
                            ]
                        },
                        {
                            "id": "helper 0.1.0",
                            "deps": [
                                {"pkg": "winit 0.30.0", "dep_kinds": [{"kind": null}]}
                            ]
                        },
                        {"id": "winit 0.30.0", "deps": []}
                    ]
                }
            }"#,
        )
        .unwrap();
        let g = from_metadata(&meta).unwrap();
        let hits = check(&g);
        assert_eq!(hits.len(), 1, "expected only transitive stats->winit violation, got {hits:?}");
        assert!(matches!(&hits[0], Violation::UiBelowL6 { krate, dep, layer: 2 } if krate == "curvebook-stats" && dep == "winit"));
    }

    #[test]
    fn exempt_apps_forbid_testkit_as_normal_dep_except_xtask() {
        let app_hit = check(&[c("curvebook", &[("curvebook-testkit", DepKind::Normal, true)])]);
        assert!(matches!(app_hit[..], [Violation::TestkitAsNormalDep { .. }]), "got {app_hit:?}");

        let cli_hit = check(&[c("curvebook-cli", &[("curvebook-testkit", DepKind::Build, true)])]);
        assert!(matches!(cli_hit[..], [Violation::TestkitAsNormalDep { .. }]), "got {cli_hit:?}");

        assert!(check(&[c("curvebook", &[("curvebook-testkit", DepKind::Dev, true)])]).is_empty());
        assert!(check(&[c("xtask", &[("curvebook-testkit", DepKind::Normal, true)])]).is_empty());
    }

    #[test]
    fn scan_patch_sections_flags_suite_crates_in_patch_and_replace() {
        let toml = r#"
[workspace]
members = ["crates/math"]

[patch.crates-io]
craft-core = { path = "vendor/craft-core" }

[replace]
"craft_ui:0.1.0" = { path = "vendor/craft_ui" }

[workspace.dependencies]
serde = "1"
"#;
        let hits = scan_patch_sections(toml);
        assert_eq!(
            hits,
            vec![
                Violation::SuiteCrate { krate: String::from("Cargo.toml[patch/replace]"), dep: String::from("craft-core") },
                Violation::SuiteCrate { krate: String::from("Cargo.toml[patch/replace]"), dep: String::from("craft_ui") },
            ]
        );
        assert!(scan_patch_sections("[workspace.dependencies]\nserde = \"1\"\n").is_empty());
    }

    #[test]
    fn table_registers_all_22_workspace_crates_including_platform() {
        assert_eq!(TABLE.len(), 22);
        assert_eq!(classify("curvebook-platform"), Some(Class::Layer(0)));
        assert!(ALLOWED_RAW_PROCESS_BASELINE.is_empty());
        assert!(ALLOWED_UI_IO_BASELINE.is_empty());
    }

    #[test]
    fn source_hygiene_flags_raw_command_and_ui_io_but_allows_cfg_test_and_platform() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let root = std::env::temp_dir().join(format!("curvebook-xtask-hygiene-{}-{nanos}", std::process::id()));
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
            "pub fn ok() {\n    let _ = curvebook_platform::fs::write_atomic(std::path::Path::new(\"a\"), b\"b\");\n    let _ = curvebook_platform::ProcessCommand::new(\"pbcopy\");\n}\n",
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

    #[test]
    fn r2_s13_06_cfg_not_test_not_masked_and_braced_command_imports_caught() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let root = std::env::temp_dir().join(format!("cb-xtask-r2-s13-06-{}-{nanos}", std::process::id()));
        let write_rel = |rel: &str, content: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        };

        write_rel(
            "crates/engine/src/guarded.rs",
            "#[cfg(all(unix, not(test)))]\npub fn prod_only() { let _ = std::process::Command::new(\"sh\"); }\n\
             #[cfg(all(feature = \"latest\"))]\npub fn feat() { let _ = std::process::Command::new(\"ls\"); }\n",
        );
        write_rel("crates/engine/src/braced.rs", "use std::process::{ Command as Proc };\npub fn run() { let _ = Proc::new(\"sh\"); }\n");
        write_rel("crates/format/src/glob_proc.rs", "use std::process::*;\npub fn run() {}\n");
        write_rel(
            "crates/ui-egui/src/extra_io.rs",
            "use std::fs::{File, write, OpenOptions};\npub fn ops() { let _ = File::create_new(\"a\"); let _ = std::fs::remove_file(\"b\"); }\n",
        );
        write_rel(
            "crates/engine/src/real_test_only.rs",
            "#[cfg(all(unix, test))]\nmod t { fn f() { let _ = std::process::Command::new(\"sh\"); } }\n",
        );

        let hits = check_source_hygiene(&root);
        let _ = std::fs::remove_dir_all(&root);

        assert!(hits.iter().any(|h| h.path == "crates/engine/src/guarded.rs" && h.line == 2 && h.rule == "raw-process"), "{hits:?}");
        assert!(hits.iter().any(|h| h.path == "crates/engine/src/guarded.rs" && h.line == 4 && h.rule == "raw-process"), "{hits:?}");
        assert!(hits.iter().any(|h| h.path == "crates/engine/src/braced.rs" && h.rule == "raw-process"), "{hits:?}");
        assert!(hits.iter().any(|h| h.path == "crates/format/src/glob_proc.rs" && h.rule == "raw-process"), "{hits:?}");
        assert!(hits.iter().any(|h| h.path == "crates/ui-egui/src/extra_io.rs" && h.pattern == "File::create_new"), "{hits:?}");
        assert!(hits.iter().any(|h| h.path == "crates/ui-egui/src/extra_io.rs" && h.pattern == "std::fs::remove_file"), "{hits:?}");
        assert!(hits.iter().any(|h| h.path == "crates/ui-egui/src/extra_io.rs" && h.pattern == "std::fs::write"), "{hits:?}");
        assert!(!hits.iter().any(|h| h.path == "crates/engine/src/real_test_only.rs"), "{hits:?}");
    }

    #[test]
    fn sec22_catches_patch_subtable_aliases_multiline_paths_and_dir_perm_link_ops() {
        let patch_sub =
            scan_patch_sections("[patch.crates-io.craft-ui]\npath = \"../x\"\n[patch.\"https://example.com\".alias]\npackage = \"craft-update\"\n");
        assert_eq!(patch_sub.len(), 2, "{patch_sub:?}");

        let meta = parse(
            r#"{
                "packages": [
                    {"id": "engine 0.1.0", "name": "curvebook-engine", "dependencies": []},
                    {"id": "ext-tk 1.0.0", "name": "curvebook-testkit", "dependencies": []},
                    {"id": "egui 0.31.0", "name": "egui", "dependencies": []}
                ],
                "workspace_members": ["engine 0.1.0"],
                "resolve": {
                    "nodes": [
                        {"id": "engine 0.1.0", "deps": [{"pkg": "ext-tk 1.0.0", "dep_kinds": [{"kind": null}]}]},
                        {"id": "ext-tk 1.0.0", "deps": [{"pkg": "egui 0.31.0", "dep_kinds": [{"kind": null}]}]},
                        {"id": "egui 0.31.0", "deps": []}
                    ]
                }
            }"#,
        )
        .unwrap();
        let g = from_metadata(&meta).unwrap();
        let dep_hits = check(&g);
        assert!(dep_hits.iter().any(|v| matches!(v, Violation::UiBelowL6 { dep, .. } if dep == "egui")), "{dep_hits:?}");

        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let root = std::env::temp_dir().join(format!("cb-xtask-sec22-layers-{}-{nanos}", std::process::id()));
        let write_rel = |rel: &str, content: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        };
        write_rel(
            "crates/engine/src/mod_alias.rs",
            "use std::process as proc;\npub fn f() { let _ = std\n    ::process\n    ::Command::new(\"ls\"); }\n",
        );
        write_rel(
            "crates/ui-egui/src/dir_ops.rs",
            "use std::fs as disk;\npub fn f(p: &std::path::Path) {\n    let _ = std\n        ::fs\n        ::create_dir_all(p);\n    let _ = fs::remove_dir(p);\n    let _ = std::fs::remove_dir_all(p);\n    let _ = std::fs::hard_link(p, p);\n    let _ = std::os::unix::fs::symlink(p, p);\n}\n",
        );
        let hits = check_source_hygiene(&root);
        let _ = std::fs::remove_dir_all(&root);
        assert!(hits.iter().any(|h| h.path.ends_with("mod_alias.rs") && h.pattern == "std::process"), "{hits:?}");
        assert!(hits.iter().any(|h| h.path.ends_with("mod_alias.rs") && h.pattern == "std::process::Command::new"), "{hits:?}");
        for expected in
            ["std::fs", "std::fs::create_dir_all", "fs::remove_dir", "std::fs::remove_dir_all", "std::fs::hard_link", "std::os::unix::fs::symlink"]
        {
            assert!(hits.iter().any(|h| h.path.ends_with("dir_ops.rs") && h.pattern == expected), "missing `{expected}` in {hits:?}");
        }
    }

    #[test]
    fn lint_policy_passes_on_workspace_and_catches_violations() {
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let real_hits = check_lint_policy(workspace_root);
        assert!(real_hits.is_empty(), "expected clean lint policy on workspace root: {real_hits:?}");

        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let root = std::env::temp_dir().join(format!("cb-xtask-lint-policy-{}-{nanos}", std::process::id()));
        let write_rel = |rel: &str, content: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        };

        write_rel("Cargo.toml", "[workspace.lints.clippy]\nunwrap_used = \"deny\"\n");
        write_rel("crates/platform/src/lib.rs", "#![forbid(unsafe_code)]\n");
        write_rel("crates/format/src/lib.rs", "#![forbid(unsafe_code)]\n#![deny(clippy::indexing_slicing)]\n");
        write_rel("crates/format/src/bad.rs", "#![allow(clippy::string_slice)]\npub fn f() {}\n");
        write_rel("crates/format/src/bad_idx.rs", "#![allow(clippy::indexing_slicing)]\npub fn g() {}\n");
        write_rel("crates/stats/src/lib.rs", "#![allow(clippy::indexing_slicing)]\npub fn ok() {}\n");

        let hits = check_lint_policy(&root);
        let _ = std::fs::remove_dir_all(&root);

        assert!(hits.iter().any(|h| h.path == "Cargo.toml" && h.rule == "lint-policy"), "{hits:?}");
        assert!(hits.iter().any(|h| h.path == "crates/platform/src/lib.rs" && h.pattern == "#![deny(clippy::indexing_slicing)]"), "{hits:?}");
        assert!(hits.iter().any(|h| h.path == "crates/format/src/bad.rs" && h.pattern == "allow(clippy::string_slice)"), "{hits:?}");
        assert!(hits.iter().any(|h| h.path == "crates/format/src/bad_idx.rs" && h.pattern == "allow(clippy::indexing_slicing)"), "{hits:?}");
        assert!(!hits.iter().any(|h| h.path == "crates/stats/src/lib.rs"), "{hits:?}");
    }
}
