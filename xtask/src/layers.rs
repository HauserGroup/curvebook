//! Dependency layering, source hygiene, lint policy, and integration-test-binary rules
//! (`docs/architecture.md` › Layers).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use serde_json::Value;

use crate::rust_lex::{
    blank_comments_and_literals, find_matching_delim, is_cfg_test_only_submodule, is_ident_byte, strip_cfg_test_items, strip_non_prod_rust,
};
use crate::scan_util::{TomlEntry, has_ext_ci, is_real_dir, next_ident_pattern_match, parse_toml_entries, read_dir_paths_sorted, rel_slash};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Layer(u8),
    Standalone,
    Testkit,
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

pub const TABLE: &[(&str, Class)] = &[
    ("math", Class::Layer(0)),
    ("palette", Class::Layer(0)),
    ("platform", Class::Layer(0)),
    ("doc", Class::Layer(1)),
    ("transform", Class::Layer(2)),
    ("stats", Class::Layer(2)),
    ("fit", Class::Layer(2)),
    ("text", Class::Layer(2)),
    ("chart", Class::Layer(2)),
    ("render", Class::Layer(3)),
    ("format", Class::Layer(3)),
    ("svg", Class::Layer(3)),
    ("pdf", Class::Layer(3)),
    ("tools", Class::Layer(4)),
    ("engine", Class::Layer(5)),
    ("ui-egui", Class::Layer(6)),
    ("mcp", Class::Layer(6)),
    ("testkit", Class::Testkit),
    ("curvebook", Class::Exempt),
    ("cli", Class::Exempt),
    ("web", Class::Exempt),
    ("xtask", Class::Exempt),
];
pub const INTRA_LAYER_ORDER: &[&[&str]] =
    &[&["math", "palette"], &["transform", "chart"], &["stats", "chart"], &["fit", "chart"], &["text", "chart"]];
pub const UI_CRATES: &[&str] = &["egui*", "epaint*", "emath", "ecolor", "eframe", "winit", "rfd", "wgpu*", "bevy*"];
pub const UI_MIN_LAYER: u8 = 6;
pub const ALLOWED_RAW_PROCESS_BASELINE: &[&str] = &[];
pub const ALLOWED_UI_IO_BASELINE: &[&str] = &[];

#[must_use]
pub fn short_name(pkg: &str) -> &str {
    pkg.strip_prefix("curvebook-").unwrap_or(pkg)
}
#[must_use]
pub fn classify(pkg: &str) -> Option<Class> {
    let s = short_name(pkg);
    TABLE.iter().find(|(n, _)| *n == s).map(|(_, c)| *c)
}
fn intra_layer_allowed(from: &str, to: &str) -> bool {
    let (from, to) = (short_name(from), short_name(to));
    INTRA_LAYER_ORDER.iter().any(|c| matches!((c.iter().position(|n| *n == from), c.iter().position(|n| *n == to)), (Some(f), Some(t)) if t < f))
}
fn is_ui_crate(name: &str) -> bool {
    UI_CRATES.iter().any(|p| p.strip_suffix('*').map_or(name == *p, |pfx| name.starts_with(pfx)))
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
    IoError { path: String, detail: String },
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Violation::Unregistered { krate } => write!(f, "{krate}: unknown workspace crate; register it in xtask/src/layers.rs TABLE"),
            Violation::Upward { krate, dep, from, to, kind } => {
                write!(f, "{krate} (L{from}) -> {dep} (L{to}) [{kind:?}]: may only depend on strictly lower layers")
            }
            Violation::StandaloneHasWorkspaceDep { krate, dep } => write!(f, "{krate}: standalone crate must not depend on workspace crate {dep}"),
            Violation::TestkitAsNormalDep { krate } => write!(f, "{krate}: curvebook-testkit may only be a dev-dependency"),
            Violation::UiBelowL6 { krate, dep, layer } => {
                write!(f, "{krate} (L{layer}) depends on UI crate `{dep}`; UI toolkits are only allowed in L6+")
            }
            Violation::SuiteCrate { krate, dep } => write!(f, "{krate} depends on external suite crate `{dep}`"),
            Violation::IoError { path, detail } => write!(f, "{path}: cannot audit manifest/file: {detail}"),
        }
    }
}

#[must_use]
pub fn check(crates: &[Crate]) -> Vec<Violation> {
    let mut out = Vec::new();
    for c in crates {
        let Some(class) = classify(&c.name) else {
            out.push(Violation::Unregistered { krate: c.name.clone() });
            continue;
        };
        for d in c.deps.iter().filter(|d| d.name != c.name) {
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
            if layer < UI_MIN_LAYER && is_ui_crate(&d.name) {
                out.push(Violation::UiBelowL6 { krate: c.name.clone(), dep: d.name.clone(), layer });
            }
            if d.workspace {
                if class == Class::Standalone {
                    out.push(Violation::StandaloneHasWorkspaceDep { krate: c.name.clone(), dep: d.name.clone() });
                    continue;
                }
                match classify(&d.name) {
                    Some(Class::Testkit) if d.kind != DepKind::Dev => out.push(Violation::TestkitAsNormalDep { krate: c.name.clone() }),
                    Some(Class::Testkit) | None => {}
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
    }
    out.sort_by_key(ToString::to_string);
    out.dedup();
    out
}

pub fn from_metadata(meta: &Value) -> Result<Vec<Crate>, String> {
    let pkgs = meta.get("packages").and_then(Value::as_array).ok_or_else(|| String::from("metadata: no packages array"))?;
    if let Some(nodes) = meta.get("resolve").and_then(|r| r.get("nodes")).and_then(Value::as_array) {
        return from_resolved_metadata(meta, pkgs, nodes);
    }
    let member_ids: Option<BTreeSet<&str>> =
        meta.get("workspace_members").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect());
    let mut member_names = BTreeSet::new();
    for p in pkgs {
        let name = p.get("name").and_then(Value::as_str).ok_or_else(|| String::from("package without name"))?;
        if member_ids.as_ref().zip(p.get("id").and_then(Value::as_str)).is_none_or(|(ids, id)| ids.contains(id)) {
            member_names.insert(name);
        }
    }
    let mut out = Vec::new();
    for p in pkgs {
        let name = p.get("name").and_then(Value::as_str).ok_or_else(|| String::from("package without name"))?;
        if member_names.contains(name) {
            out.push(Crate { name: name.to_owned(), deps: collect_declared_deps(p, &member_names) });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn parse_dep_kind_str(kind: Option<&str>) -> DepKind {
    match kind {
        Some("dev") => DepKind::Dev,
        Some("build") => DepKind::Build,
        _ => DepKind::Normal,
    }
}

fn collect_declared_deps(pkg: &Value, member_names: &BTreeSet<&str>) -> Vec<Dep> {
    pkg.get("dependencies")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|d| {
            let name = d.get("name").and_then(Value::as_str).unwrap_or_default().to_owned();
            let kind = parse_dep_kind_str(d.get("kind").and_then(Value::as_str));
            let workspace = member_names.contains(name.as_str())
                || classify(&name).is_some()
                || (d.get("path").is_some_and(Value::is_string) && name.starts_with("curvebook-"));
            Dep { name, kind, workspace }
        })
        .collect()
}

fn from_resolved_metadata(meta: &Value, pkgs: &[Value], resolve_nodes: &[Value]) -> Result<Vec<Crate>, String> {
    let (mut pkg_by_id, mut name_by_id) = (BTreeMap::new(), BTreeMap::new());
    for p in pkgs {
        let id = p.get("id").and_then(Value::as_str).ok_or_else(|| String::from("package without id"))?;
        let name = p.get("name").and_then(Value::as_str).ok_or_else(|| String::from("package without name"))?;
        pkg_by_id.insert(id, p);
        name_by_id.insert(id, name);
    }
    let member_ids: Vec<&str> = meta
        .get("workspace_members")
        .and_then(Value::as_array)
        .map_or_else(|| pkg_by_id.keys().copied().collect(), |a| a.iter().filter_map(Value::as_str).collect());
    let (member_id_set, member_names): (BTreeSet<&str>, BTreeSet<&str>) =
        (member_ids.iter().copied().collect(), member_ids.iter().filter_map(|id| name_by_id.get(id).copied()).collect());
    let node_by_id: BTreeMap<&str, &Value> = resolve_nodes.iter().filter_map(|n| n.get("id").and_then(Value::as_str).map(|id| (id, n))).collect();

    let mut out = Vec::with_capacity(member_ids.len());
    for mid in member_ids {
        let Some(&mname) = name_by_id.get(mid) else { continue };
        let mut seen: BTreeSet<(String, DepKind, bool)> = pkg_by_id
            .get(mid)
            .map(|pkg| collect_declared_deps(pkg, &member_names))
            .unwrap_or_default()
            .into_iter()
            .map(|d| (d.name, d.kind, d.workspace))
            .collect();
        if let Some(&root_node) = node_by_id.get(mid) {
            let (mut queue, mut visited) = (VecDeque::new(), BTreeSet::new());
            for (dep_id, kinds) in node_edges(root_node) {
                kinds.into_iter().for_each(|kind| queue.push_back((dep_id, kind)));
            }
            while let Some((pid, root_kind)) = queue.pop_front() {
                if pid == mid || !visited.insert((pid, root_kind)) {
                    continue;
                }
                let Some(&pname) = name_by_id.get(pid) else { continue };
                let is_ws = member_id_set.contains(pid);
                seen.insert((pname.to_owned(), root_kind, is_ws));
                if is_ws && matches!(classify(pname), Some(Class::Testkit | Class::Exempt)) {
                    continue;
                }
                if let Some(&sub) = node_by_id.get(pid) {
                    for (sub_id, skinds) in node_edges(sub) {
                        if skinds.into_iter().any(|k| k != DepKind::Dev) {
                            queue.push_back((sub_id, root_kind));
                        }
                    }
                }
            }
        }
        out.push(Crate { name: mname.to_owned(), deps: seen.into_iter().map(|(name, kind, workspace)| Dep { name, kind, workspace }).collect() });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn node_edges(node: &Value) -> Vec<(&str, Vec<DepKind>)> {
    if let Some(deps) = node.get("deps").and_then(Value::as_array) {
        return deps
            .iter()
            .filter_map(|d| {
                let pkg = d.get("pkg").and_then(Value::as_str)?;
                let mut kinds: Vec<DepKind> = d
                    .get("dep_kinds")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|dk| parse_dep_kind_str(dk.get("kind").and_then(Value::as_str)))
                    .collect();
                if kinds.is_empty() {
                    kinds.push(DepKind::Normal);
                }
                Some((pkg, kinds))
            })
            .collect();
    }
    node.get("dependencies").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(|id| (id, vec![DepKind::Normal])).collect()
}

#[must_use]
pub fn check_manifest_patches(root: &Path) -> Vec<Violation> {
    let manifest = root.join("Cargo.toml");
    match std::fs::symlink_metadata(&manifest) {
        Ok(m) if m.file_type().is_symlink() => {
            vec![Violation::IoError { path: String::from("Cargo.toml"), detail: String::from("manifest is a symlink") }]
        }
        Err(e) => vec![Violation::IoError { path: String::from("Cargo.toml"), detail: e.to_string() }],
        Ok(_) => std::fs::read_to_string(&manifest)
            .map_or_else(|e| vec![Violation::IoError { path: String::from("Cargo.toml"), detail: e.to_string() }], |t| scan_patch_sections(&t)),
    }
}

#[must_use]
pub fn scan_patch_sections(cargo_toml: &str) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut push_suite = |dep: &str| {
        if is_suite_crate(dep) {
            let v = Violation::SuiteCrate { krate: String::from("Cargo.toml[patch/replace]"), dep: dep.to_owned() };
            if !violations.contains(&v) {
                violations.push(v);
            }
        }
    };
    for entry in parse_toml_entries(cargo_toml) {
        match entry {
            TomlEntry::Section { path, .. } if path.first().map(String::as_str) == Some("patch") => {
                if let Some(name) = path.get(2) {
                    push_suite(name);
                }
            }
            TomlEntry::Leaf { path, value, .. } if path.first().map(String::as_str) == Some("replace") => {
                if let Some(spec) = path.get(1) {
                    push_suite(spec.split(':').next().unwrap_or(spec).trim());
                }
                if path.last().map(String::as_str) == Some("package")
                    && let Some(pkg) = value.as_str()
                {
                    push_suite(pkg);
                }
            }
            TomlEntry::Leaf { path, value, .. } if path.first().map(String::as_str) == Some("patch") => {
                if let Some(name) = path.get(2) {
                    push_suite(name);
                }
                if path.get(3).map(String::as_str) == Some("package")
                    && let Some(pkg) = value.as_str()
                {
                    push_suite(pkg);
                }
            }
            _ => {}
        }
    }
    violations
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

const RAW_PROCESS_PATTERNS: &str = "std::process::Command::new std::process::Command process::Command::new process::Command std::process::*";
const UI_IO_SHARED_PATHS: &str = "std::fs::write fs::write std::fs::remove_file fs::remove_file std::fs::remove_dir_all fs::remove_dir_all std::fs::remove_dir fs::remove_dir std::fs::create_dir_all fs::create_dir_all std::fs::create_dir fs::create_dir std::fs::set_permissions fs::set_permissions std::fs::hard_link fs::hard_link std::os::unix::fs::symlink os::unix::fs::symlink unix::fs::symlink fs::symlink std::os::unix::fs::* os::unix::fs::* unix::fs::* std::os::windows::fs::symlink_file std::os::windows::fs::symlink_dir os::windows::fs::symlink_file os::windows::fs::symlink_dir windows::fs::symlink_file windows::fs::symlink_dir fs::symlink_file fs::symlink_dir std::os::windows::fs::* os::windows::fs::* windows::fs::* std::fs::copy fs::copy std::fs::rename fs::rename std::fs::* std::process::Command process::Command std::process::*";
const UI_IO_EXTRA_EXPR: &str = "std::fs::File::create_new File::create_new std::fs::File::create File::create OpenOptions Command::new";
const UI_IO_EXTRA_USE: &str = "std::fs::File fs::File std::fs::OpenOptions fs::OpenOptions";
const UI_IO_MOD_ALIASES: &str = "std::fs fs std::os os std::os::unix os::unix unix std::os::unix::fs os::unix::fs unix::fs std::os::windows os::windows windows std::os::windows::fs os::windows::fs windows::fs std::process process";

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

fn push_hyg(out: &mut Vec<HygieneViolation>, path: impl Into<String>, line: usize, rule: &'static str, pattern: &'static str) {
    out.push(HygieneViolation { path: path.into(), line, rule, pattern });
}

fn sort_dedup_hygiene(violations: &mut Vec<HygieneViolation>) {
    violations.sort_by(|a, b| {
        a.path.cmp(&b.path).then_with(|| a.line.cmp(&b.line)).then_with(|| a.rule.cmp(b.rule)).then_with(|| a.pattern.cmp(b.pattern))
    });
    violations.dedup();
}

#[must_use]
pub fn check_source_hygiene(root: &Path) -> Vec<HygieneViolation> {
    let mut violations = Vec::new();
    for rel in collect_rs_files_checked(root, &mut violations) {
        let exempt_process = rel.starts_with("crates/platform/src/")
            || rel.starts_with("xtask/")
            || rel.starts_with("tests/")
            || matches!(rel.split('/').collect::<Vec<_>>().as_slice(), ["crates" | "apps", _, "tests", ..])
            || is_cfg_test_only_submodule(root, &rel);
        let in_ui_src = rel.starts_with("crates/ui-egui/src/") && !is_cfg_test_only_submodule(root, &rel);
        if exempt_process && !in_ui_src {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(root.join(&rel)) else {
            push_hyg(&mut violations, rel, 1, "io-error", "unreadable source file");
            continue;
        };
        let prod = strip_non_prod_rust(&src);
        if !exempt_process && !ALLOWED_RAW_PROCESS_BASELINE.contains(&rel.as_str()) {
            scan_patterns(&rel, &prod, "raw-process", &[RAW_PROCESS_PATTERNS], &mut violations);
            scan_use_statements(
                &rel,
                &prod,
                "raw-process",
                &["std::process::Command process::Command std::process::*"],
                "std::process process",
                &mut violations,
            );
        }
        if in_ui_src && !ALLOWED_UI_IO_BASELINE.contains(&rel.as_str()) {
            scan_patterns(&rel, &prod, "ui-raw-io", &[UI_IO_SHARED_PATHS, UI_IO_EXTRA_EXPR], &mut violations);
            scan_use_statements(&rel, &prod, "ui-raw-io", &[UI_IO_SHARED_PATHS, UI_IO_EXTRA_USE], UI_IO_MOD_ALIASES, &mut violations);
        }
    }
    sort_dedup_hygiene(&mut violations);
    violations
}

const SPECIAL_INTEGRATION_BINARIES: &[(&str, &str)] = &[
    ("crates/math", "nist_strd"),
    ("crates/stats", "nist_strd"),
    ("crates/fit", "nist_strd"),
    ("apps/curvebook", "control_loopback"),
    ("apps/curvebook", "desktop_host_loop"),
    ("crates/ui-egui", "macos_clipboard_live"),
    ("crates/ui-egui", "install_fonts"),
    ("xtask", "r12_docs_site"),
];

#[must_use]
pub fn check_integration_test_binaries(root: &Path) -> Vec<HygieneViolation> {
    let mut pkg_dirs = Vec::new();
    for group in ["crates", "apps"] {
        let dir = root.join(group);
        if is_real_dir(&dir)
            && let Ok(entries) = read_dir_paths_sorted(&dir)
        {
            for n in entries.into_iter().filter(|p| is_real_dir(p)).filter_map(|p| p.file_name().and_then(|s| s.to_str()).map(str::to_owned)) {
                pkg_dirs.push(format!("{group}/{n}"));
            }
        }
    }
    if is_real_dir(&root.join("xtask")) {
        pkg_dirs.push(String::from("xtask"));
    }
    let mut violations = Vec::new();
    for pkg_rel in pkg_dirs {
        let tests_dir = root.join(&pkg_rel).join("tests");
        let Ok(entries) = (is_real_dir(&tests_dir)).then(|| read_dir_paths_sorted(&tests_dir)).transpose() else { continue };
        let Some(entries) = entries else { continue };
        let is_special = |name: &str| {
            name.starts_with("r10_")
                || name.starts_with("human_wf_")
                || name.contains("perf")
                || name == "r15_2_anova_and_1m_bench"
                || SPECIAL_INTEGRATION_BINARIES.contains(&(pkg_rel.as_str(), name))
        };
        let mut general_bins = Vec::new();
        for entry in entries {
            let (Ok(meta), Some(fname)) = (std::fs::symlink_metadata(&entry), entry.file_name().and_then(|n| n.to_str())) else { continue };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_file()
                && let Some(stem) = fname.strip_suffix(".rs")
                && !is_special(stem)
            {
                general_bins.push((true, format!("{pkg_rel}/tests/{fname}")));
            } else if meta.is_dir() && entry.join("main.rs").is_file() && !is_special(fname) {
                general_bins.push((false, format!("{pkg_rel}/tests/{fname}/main.rs")));
            }
        }
        general_bins.sort();
        for (_, extra) in general_bins.into_iter().skip(1) {
            push_hyg(
                &mut violations,
                extra,
                1,
                "test-binary-sprawl",
                "extra general integration test binary (add as a submodule of this crate's single integration suite)",
            );
        }
    }
    sort_dedup_hygiene(&mut violations);
    violations
}

const LEGACY_INDEXING_ALLOWLIST: &[&str] =
    &["crates/stats", "crates/fit", "crates/transform", "crates/chart", "crates/text", "crates/ui-egui", "crates/testkit", "xtask"];
const DOC_SCREENSHOTS_REL: &str = "crates/ui-egui/examples/doc_screenshots.rs";

#[must_use]
pub fn check_lint_policy(root: &Path) -> Vec<HygieneViolation> {
    let mut violations = Vec::new();
    let has_string_slice_deny = std::fs::read_to_string(root.join("Cargo.toml")).ok().is_some_and(|src| {
        parse_toml_entries(&src).into_iter().any(|e| matches!(e, TomlEntry::Leaf { path, value, .. } if path == ["workspace", "lints", "clippy", "string_slice"] && value.as_str() == Some("deny")))
    });
    if !has_string_slice_deny {
        push_hyg(&mut violations, "Cargo.toml", 1, "lint-policy", "string_slice = \"deny\"");
    }
    for group in ["crates", "apps"] {
        let group_dir = root.join(group);
        let Ok(meta) = std::fs::symlink_metadata(&group_dir) else { continue };
        if meta.file_type().is_symlink() {
            push_hyg(&mut violations, group, 1, "io-error", "symlinked directory");
            continue;
        }
        for path in read_dir_paths_sorted(&group_dir).unwrap_or_default() {
            let (Some(dir_str), Ok(emeta)) = (path.file_name().and_then(|n| n.to_str()), std::fs::symlink_metadata(&path)) else { continue };
            let pkg_rel = format!("{group}/{dir_str}");
            if emeta.file_type().is_symlink() {
                push_hyg(&mut violations, pkg_rel, 1, "io-error", "symlinked crate directory");
            } else if emeta.is_dir() && !LEGACY_INDEXING_ALLOWLIST.contains(&pkg_rel.as_str()) {
                for rf in ["src/lib.rs", "src/main.rs"] {
                    let rel = format!("{pkg_rel}/{rf}");
                    if std::fs::symlink_metadata(root.join(&rel)).is_ok() {
                        require_indexing_slicing_deny(root, &rel, &mut violations);
                    }
                }
            }
        }
    }
    if std::fs::symlink_metadata(root.join(DOC_SCREENSHOTS_REL)).is_ok() || root.join("crates/ui-egui").is_dir() {
        require_indexing_slicing_deny(root, DOC_SCREENSHOTS_REL, &mut violations);
    }
    for rel in collect_rs_files_checked(root, &mut violations) {
        let in_any = (rel.starts_with("crates/") || rel.starts_with("apps/")) && rel.contains("/src/") || rel.starts_with("xtask/src/");
        if !in_any && rel != DOC_SCREENSHOTS_REL {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(root.join(&rel)) else {
            push_hyg(&mut violations, rel, 1, "io-error", "unreadable source file");
            continue;
        };
        let blanked = blank_comments_and_literals(&src);
        if let Some(m) = find_lint_escape(&blanked, "clippy::string_slice") {
            let line = src.lines().enumerate().find(|(_, l)| l.contains(m)).map_or(1, |(i, _)| i + 1);
            push_hyg(&mut violations, rel.clone(), line, "lint-policy", "allow(clippy::string_slice)");
        }
        let in_strict = in_any && !LEGACY_INDEXING_ALLOWLIST.iter().any(|p| rel == *p || rel.starts_with(&format!("{p}/")));
        if (in_strict || rel == DOC_SCREENSHOTS_REL)
            && let Some(m) = find_lint_escape(&strip_cfg_test_items(&blanked), "clippy::indexing_slicing")
        {
            let line = src.lines().enumerate().find(|(_, l)| l.contains(m)).map_or(1, |(i, _)| i + 1);
            push_hyg(&mut violations, rel, line, "lint-policy", "allow(clippy::indexing_slicing)");
        }
    }
    sort_dedup_hygiene(&mut violations);
    violations
}

fn require_indexing_slicing_deny(root: &Path, rel: &str, violations: &mut Vec<HygieneViolation>) {
    let full = root.join(rel);
    if std::fs::symlink_metadata(&full).is_ok_and(|m| m.file_type().is_symlink()) {
        return push_hyg(violations, rel, 1, "io-error", "symlinked source file");
    }
    let Ok(src) = std::fs::read_to_string(&full) else {
        return push_hyg(violations, rel, 1, "lint-policy", "#![deny(clippy::indexing_slicing)]");
    };
    let prod = strip_non_prod_rust(&src);
    let (bytes, mut i) = (prod.as_bytes(), 0usize);
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes.get(i..i + 3) != Some(b"#![") {
            break;
        }
        let Some(close) = find_matching_delim(bytes, i + 2, b'[', b']') else { break };
        let compact: String = prod.get(i + 3..close).unwrap_or("").chars().filter(|c| !c.is_whitespace()).collect();
        if ["deny(", "forbid("].iter().any(|kw| {
            compact.strip_prefix(kw).and_then(|r| r.strip_suffix(')')).is_some_and(|l| l.split(',').any(|x| x == "clippy::indexing_slicing"))
        }) {
            return;
        }
        i = close + 1;
    }
    push_hyg(violations, rel, 1, "lint-policy", "#![deny(clippy::indexing_slicing)]");
}

fn find_lint_escape<'a>(src: &str, lint: &'a str) -> Option<&'a str> {
    let compact: String = src.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = compact.as_bytes();
    for attr in ["allow(", "expect("] {
        let mut s = 0usize;
        while let Some(rel) = compact.get(s..).and_then(|r| r.find(attr)) {
            let (pos, inner_start) = (s + rel, s + rel + attr.len());
            if (pos == 0 || !is_ident_byte(bytes[pos - 1]))
                && let Some(close) = compact.get(inner_start..).and_then(|r| r.find(')'))
            {
                for item in compact.get(inner_start..inner_start + close).unwrap_or("").split(',') {
                    if item == lint {
                        return Some(lint.rsplit("::").next().unwrap_or(lint));
                    }
                    if matches!(item, "clippy::restriction" | "clippy::all" | "warnings") {
                        return Some(if item == "warnings" { "warnings" } else { "clippy::" });
                    }
                }
                s = inner_start + close + 1;
            } else {
                s = inner_start;
            }
        }
    }
    None
}

fn collect_rs_files_checked(root: &Path, violations: &mut Vec<HygieneViolation>) -> Vec<String> {
    let mut files = Vec::new();
    walk_rs_fail_closed(root, root, &mut files, violations);
    files.sort();
    files
}

fn walk_rs_fail_closed(root: &Path, dir: &Path, out: &mut Vec<String>, violations: &mut Vec<HygieneViolation>) {
    let Ok(entries) = read_dir_paths_sorted(dir) else {
        return push_hyg(violations, rel_slash(root, dir).unwrap_or_else(|_| dir.display().to_string()), 1, "io-error", "unreadable directory");
    };
    for path in entries {
        let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if matches!(fname, "target" | "dist" | "corpus") || fname.starts_with('.') {
            continue;
        }
        let rel = rel_slash(root, &path).unwrap_or_else(|_| path.display().to_string());
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            push_hyg(violations, rel, 1, "io-error", "unreadable path");
            continue;
        };
        if meta.file_type().is_symlink() {
            if has_ext_ci(&path, &["rs"])
                || matches!(rel.as_str(), "crates" | "apps" | "xtask")
                || rel.starts_with("crates/")
                || rel.starts_with("apps/")
                || rel.starts_with("xtask/")
            {
                push_hyg(violations, rel, 1, "io-error", "symlinked source path");
            }
        } else if meta.is_dir() {
            walk_rs_fail_closed(root, &path, out, violations);
        } else if meta.is_file() && has_ext_ci(&path, &["rs"]) {
            out.push(rel);
        }
    }
}

pub(crate) fn collapse_colon_whitespace(src: &str) -> (String, Vec<usize>) {
    let (bytes, mut out, mut lines, mut i, mut line) =
        (src.as_bytes(), String::with_capacity(src.len()), Vec::with_capacity(src.len()), 0usize, 1usize);
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            let (mut j, mut nl) = (i, 0usize);
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                (nl, j) = (nl + usize::from(bytes[j] == b'\n'), j + 1);
            }
            if bytes[j..].starts_with(b"::") || out.ends_with("::") {
                (line, i) = (line + nl, j);
                continue;
            }
        }
        let ch = src.get(i..).and_then(|s| s.chars().next()).unwrap_or(' ');
        out.push(ch);
        lines.extend(std::iter::repeat_n(line, ch.len_utf8()));
        (line, i) = (line + usize::from(ch == '\n'), i + ch.len_utf8());
    }
    (out, lines)
}

fn scan_patterns(rel: &str, prod_src: &str, rule: &'static str, groups: &[&'static str], out: &mut Vec<HygieneViolation>) {
    let (compact, line_of_byte) = collapse_colon_whitespace(prod_src);
    let pats: Vec<&'static str> = groups.iter().flat_map(|g| g.split_ascii_whitespace()).collect();
    let mut pos = 0usize;
    while let Some((hit, pat)) = next_ident_pattern_match(&compact, pos, &pats) {
        push_hyg(out, rel, line_of_byte.get(hit).copied().unwrap_or(1), rule, pat);
        pos = hit + pat.len();
    }
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
    banned_mods: &'static str,
    out: &mut Vec<HygieneViolation>,
) {
    for (line, stmt) in collect_use_statements(prod_src) {
        let mut expanded = Vec::new();
        expand_use_tree_items("", &stmt, &mut expanded);
        for item in expanded {
            let norm = item.path.strip_prefix("::").unwrap_or(&item.path);
            for pat in banned_paths.iter().flat_map(|g| g.split_ascii_whitespace()).filter(|&p| norm == p) {
                push_hyg(out, rel, line, rule, pat);
            }
            if item.alias.is_some() {
                for mp in banned_mods.split_ascii_whitespace().filter(|&p| norm == p) {
                    push_hyg(out, rel, line, rule, mp);
                }
            }
        }
    }
}

pub(crate) fn collect_use_statements(prod_src: &str) -> Vec<(usize, String)> {
    let (bytes, mut stmts, mut i, mut line) = (prod_src.as_bytes(), Vec::new(), 0usize, 1usize);
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            (line, i) = (line + 1, i + 1);
            continue;
        }
        if bytes[i..].starts_with(b"use") && (i == 0 || !is_ident_byte(bytes[i - 1])) && bytes.get(i + 3).is_some_and(u8::is_ascii_whitespace) {
            let stmt_line = line;
            i += 3;
            let mut buf = String::new();
            while i < bytes.len() && bytes[i] != b';' {
                line += usize::from(bytes[i] == b'\n');
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
            continue;
        }
        i += 1;
    }
    stmts
}

pub(crate) fn expand_use_tree_items(prefix: &str, tree: &str, out: &mut Vec<UseBinding>) {
    let trimmed = tree.trim();
    if trimmed.is_empty() {
        return;
    }
    if let Some((head, inner)) = trimmed.split_once('{').and_then(|(h, r)| r.strip_suffix('}').map(|i| (h, i))) {
        let sub: String = head.chars().filter(|c| !c.is_whitespace()).collect();
        let seg = sub.trim_end_matches("::");
        let next_pfx = match (prefix.is_empty(), seg.is_empty()) {
            (true, _) => seg.to_owned(),
            (false, true) => prefix.to_owned(),
            (false, false) => format!("{prefix}::{seg}"),
        };
        let (mut depth, mut start) = (0usize, 0usize);
        for (i, ch) in inner.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    expand_use_tree_items(&next_pfx, inner.get(start..i).unwrap_or(""), out);
                    start = i + 1;
                }
                _ => {}
            }
        }
        return expand_use_tree_items(&next_pfx, inner.get(start..).unwrap_or(""), out);
    }
    let (item_path, alias) = trimmed
        .match_indices("as")
        .find(|&(i, _)| {
            i > 0 && trimmed.as_bytes()[i - 1].is_ascii_whitespace() && trimmed.as_bytes().get(i + 2).is_some_and(u8::is_ascii_whitespace)
        })
        .and_then(|(i, _)| {
            let (h, t) = trimmed.split_at_checked(i)?;
            let a = t.strip_prefix("as")?.trim();
            Some((h.trim(), (!a.is_empty()).then_some(a)))
        })
        .unwrap_or((trimmed, None));
    let compact: String = item_path.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.is_empty() {
        return;
    }
    let resolved = if compact == "self" {
        prefix.to_owned()
    } else if prefix.is_empty() {
        compact
    } else {
        format!("{prefix}::{compact}")
    };
    if !resolved.is_empty() {
        out.push(UseBinding { path: resolved, alias: alias.map(str::to_owned) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan_util::test_support::TestDir;

    fn c(name: &str, deps: &[(&str, DepKind, bool)]) -> Crate {
        Crate { name: name.into(), deps: deps.iter().map(|(n, k, w)| Dep { name: (*n).into(), kind: *k, workspace: *w }).collect() }
    }

    #[test]
    fn layering_metadata_patch_hygiene_lint_and_test_binary_rules() {
        assert_eq!((TABLE.len(), classify("curvebook-platform")), (22, Some(Class::Layer(0))));
        assert!(ALLOWED_RAW_PROCESS_BASELINE.is_empty() && ALLOWED_UI_IO_BASELINE.is_empty());
        let clean = [
            c("curvebook-math", &[]),
            c("curvebook-doc", &[("curvebook-math", DepKind::Normal, true)]),
            c("curvebook-chart", &[("curvebook-stats", DepKind::Normal, true)]),
            c("curvebook-engine", &[("curvebook-chart", DepKind::Normal, true), ("curvebook-testkit", DepKind::Dev, true)]),
            c("curvebook-ui-egui", &[("curvebook-engine", DepKind::Normal, true), ("egui", DepKind::Normal, false)]),
        ];
        assert!(check(&clean).is_empty());
        assert!(matches!(
            check(&[c("curvebook-doc", &[("curvebook-engine", DepKind::Normal, true)])])[..],
            [Violation::Upward { from: 1, to: 5, .. }]
        ));
        assert!(matches!(
            check(&[c("curvebook-stats", &[("curvebook-testkit", DepKind::Normal, true)])])[..],
            [Violation::TestkitAsNormalDep { .. }]
        ));
        assert!(matches!(check(&[c("curvebook-ui-egui", &[("craft-ui", DepKind::Normal, false)])])[..], [Violation::SuiteCrate { .. }]));
        assert!(matches!(check(&[c("curvebook-engine", &[("winit", DepKind::Build, true)])])[..], [Violation::UiBelowL6 { layer: 5, .. }]));

        let meta: serde_json::Value = serde_json::from_str(r#"{"packages":[{"name":"curvebook-stats","id":"s","dependencies":[]},{"name":"helper","id":"h","dependencies":[]},{"name":"winit","id":"w","dependencies":[]}],"workspace_members":["s"],"resolve":{"nodes":[{"id":"s","deps":[{"pkg":"h","dep_kinds":[{"kind":null}]}]},{"id":"h","deps":[{"pkg":"w","dep_kinds":[{"kind":null}]}]},{"id":"w","deps":[]}]}}"#).unwrap();
        assert!(matches!(check(&from_metadata(&meta).unwrap())[..], [Violation::UiBelowL6 { layer: 2, .. }]));
        assert_eq!(
            scan_patch_sections("[patch.crates-io]\ncraft-core = { path = \"x\" }\n[replace]\n\"craft_ui:0.1\" = { path = \"y\" }\n").len(),
            2
        );

        let dir = TestDir::new("layers-hyg");
        dir.write("Cargo.toml", "[workspace.lints.clippy]\nunwrap_used = \"deny\"\n");
        dir.write("crates/engine/src/bad.rs", "pub fn f() { let _ = std::process::Command::new(\"sh\"); }\n");
        dir.write("crates/engine/src/ok.rs", "#[cfg(test)] mod t { fn x() { let _ = std::process::Command::new(\"sh\"); } }\n");
        dir.write("crates/ui-egui/src/bad_io.rs", "pub fn w() { let _ = std::fs::write(\"a\", b\"b\"); }\n");
        dir.write("crates/format/src/lib.rs", "#![allow(clippy::string_slice)]\npub fn f() {}\n");
        dir.write("crates/engine/tests/suite/main.rs", "fn main() {}\n");
        dir.write("crates/engine/tests/r22_extra.rs", "#[test] fn t() {}\n");
        let hyg = check_source_hygiene(dir.path());
        assert!(
            hyg.iter().any(|h| h.path == "crates/engine/src/bad.rs")
                && !hyg.iter().any(|h| h.path == "crates/engine/src/ok.rs")
                && hyg.iter().any(|h| h.path == "crates/ui-egui/src/bad_io.rs")
        );
        let lint = check_lint_policy(dir.path());
        assert!(lint.iter().any(|h| h.path == "Cargo.toml") && lint.iter().any(|h| h.path == "crates/format/src/lib.rs"));
        assert_eq!(check_integration_test_binaries(dir.path()).len(), 1);
    }
}
