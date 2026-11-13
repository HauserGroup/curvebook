//! Dependency layering rules (`docs/architecture.md` › Layers).
//!
//! Enforces that workspace crates depend strictly downward in the layer hierarchy, that external
//! suite crates (`craft-*`) never appear anywhere in the workspace, that UI toolkit crates (`egui`,
//! `eframe`, `winit`, `rfd`, `bevy*`) never appear below L6, and that `lumencraft-testkit` is used
//! solely as a `[dev-dependencies]` entry.

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

#[cfg(test)]
mod tests {
    use super::{Crate, Dep, DepKind, Violation, check, from_metadata};
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
}
