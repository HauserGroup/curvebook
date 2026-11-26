//! `cargo xtask ci quick`: the fast local tier of the CI gates.
//!
//! Runs every policy gate, then:
//! - By default: runs Clippy (`--all-targets -D warnings`) across directly modified packages and
//!   their workspace reverse dependents (catching downstream compile/lint regressions), and runs
//!   the non-performance nextest suite for **directly modified packages only** (leaving full
//!   transitive test fan-out to hosted CI on the pull request).
//! - With `--direct`: restricts both Clippy and tests to directly modified packages for inner-loop
//!   iteration.
//! - With `--transitive`: runs both Clippy and tests across directly modified packages and their
//!   workspace reverse dependents (omitting `xtask` tests when `xtask` is only pulled in
//!   transitively), for local verification of shared wire/schema/API contract changes.
//!
//! Changes are measured against the merge base with `main` (or `--base <ref>`), including
//! staged, unstaged, and untracked files. Edits to workspace-wide build inputs (the root
//! manifest, lockfile, toolchain, or lint configuration) select the whole workspace.
//!
//! To avoid over-triggering expensive checks on foundational-crate edits, `i18n-report`
//! runs only when `curvebook-ui-egui` is directly modified or a workspace-wide input
//! changed, and `xtask` tests are omitted when `xtask` is only pulled in as a transitive
//! reverse dependent.
//!
//! The full `cargo xtask ci` run (and hosted CI on every pull request) remains the complete
//! gate: this tier skips doctests/rustdoc, the `--no-default-features` builds, and the serial
//! performance/budget binaries.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use serde_json::Value;

use crate::layers::Crate;
use crate::{cargo_metadata_resolved, ci, has_cargo_nextest, layers, run_cargo, run_cargo_strs};

/// Root-level files whose edits can change how every workspace package builds, tests, or verifies.
const WORKSPACE_WIDE_FILES: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "rust-toolchain",
    "clippy.toml",
    ".clippy.toml",
    ".config/nextest.toml",
    "deny.toml",
    "docs/control-protocol.md",
    "docs/evidence/overview.md",
    "docs/release/github-pages.md",
    "docs/implementation/scripting-and-automation.md",
];
/// Directories whose contents can change how every workspace package builds, tests, or packages.
const WORKSPACE_WIDE_DIRS: &[&str] = &[".cargo/", ".github/", "assets/app-icon/", "packaging/", "docs/schemas/", "corpus/nist-strd/"];
/// The package whose test library hosts the untranslated-locale report run by `policy`.
const I18N_PACKAGE: &str = "curvebook-ui-egui";
/// The workspace task runner package; its test suite (including release-scan `.dmg` checks)
/// is omitted when `xtask` is only pulled in as a transitive reverse dependent.
const XTASK_PACKAGE: &str = "xtask";

/// Which workspace packages the quick tier lints and tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// A workspace-wide build input changed: lint and test `--workspace`.
    Workspace,
    /// Only these packages (possibly none, for documentation-only changes).
    Packages(BTreeSet<String>),
}

impl Selection {
    fn is_empty(&self) -> bool {
        matches!(self, Self::Packages(set) if set.is_empty())
    }

    fn contains_package(&self, package: &str) -> bool {
        match self {
            Self::Workspace => true,
            Self::Packages(set) => set.contains(package),
        }
    }

    fn cargo_scope(&self) -> Vec<String> {
        match self {
            Self::Workspace => vec![String::from("--workspace")],
            Self::Packages(set) => set.iter().flat_map(|p| [String::from("-p"), p.clone()]).collect(),
        }
    }
}

/// Resolved execution plan for `cargo xtask ci quick`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickPlan {
    /// True when a workspace-wide build input changed (root manifest, lockfile, toolchain, etc.).
    pub workspace_wide: bool,
    /// Workspace packages owning at least one changed file directly.
    pub direct_packages: BTreeSet<String>,
    /// Packages to lint with Clippy.
    pub lint_selection: Selection,
    /// Packages to test with nextest or `cargo test`.
    pub test_selection: Selection,
    /// Whether `ci::policy` should run `cmd_i18n_report`.
    pub include_i18n: bool,
}

/// Package-selection mode for `cargo xtask ci quick`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeMode {
    /// Default pre-PR tier: Clippy all-targets on direct + reverse dependents, tests on direct packages only.
    Default,
    /// `--direct`: Clippy and tests on directly modified packages only.
    Direct,
    /// `--transitive`: Clippy and tests on direct + reverse dependents (omitting transitive `xtask` tests).
    Transitive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QuickArgs {
    base: Option<String>,
    mode: ScopeMode,
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let parsed = parse_args(args)?;
    let start = Instant::now();
    let base = match parsed.base {
        Some(base) => base,
        None => default_base(root)?,
    };
    let changed = changed_files(root, &base)?;
    let meta = cargo_metadata_resolved(root)?;
    let dirs = package_dirs(&meta)?;
    let graph = layers::from_metadata(&meta)?;
    let quick_plan = plan(&changed, &dirs, &graph, parsed.mode);
    print_plan(&base, changed.len(), parsed.mode, &quick_plan);

    println!("ci quick: starting policy");
    ci::policy(root, quick_plan.include_i18n)?;
    if quick_plan.lint_selection.is_empty() && quick_plan.test_selection.is_empty() {
        println!("ci quick: no Rust package changed; skipping lint and tests");
    } else {
        if !quick_plan.lint_selection.is_empty() {
            let lint_scope = quick_plan.lint_selection.cargo_scope();
            let mut lint = vec![String::from("clippy"), String::from("--locked"), String::from("--all-targets")];
            lint.extend(lint_scope);
            lint.extend([String::from("--"), String::from("-D"), String::from("warnings")]);
            run_cargo(root, &lint)?;
            if quick_plan.lint_selection.contains_package(XTASK_PACKAGE) {
                run_cargo_strs(root, ci::DOCS_SITE_CLIPPY_ARGS)?;
            }
        }
        if !quick_plan.test_selection.is_empty() {
            let test_scope = quick_plan.test_selection.cargo_scope();
            run_cargo(root, &test_args(root, &test_scope))?;
            if quick_plan.test_selection.contains_package(XTASK_PACKAGE) {
                run_cargo(root, &ci::docs_site_test_args(has_cargo_nextest()))?;
            }
        }
    }
    println!("ci quick: passed in {:.2}s (full gate: `cargo xtask ci` or hosted CI on the pull request)", start.elapsed().as_secs_f64());
    Ok(())
}

fn parse_args(args: &[String]) -> Result<QuickArgs, String> {
    const USAGE: &str = "usage: cargo xtask ci quick [--direct|--transitive] [--base <git-ref>]";
    let mut base: Option<String> = None;
    let mut mode: Option<ScopeMode> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--direct" => {
                if mode.is_some() {
                    return Err(String::from(USAGE));
                }
                mode = Some(ScopeMode::Direct);
            }
            "--transitive" => {
                if mode.is_some() {
                    return Err(String::from(USAGE));
                }
                mode = Some(ScopeMode::Transitive);
            }
            "--base" => {
                if base.is_some() {
                    return Err(String::from(USAGE));
                }
                let Some(value) = iter.next() else {
                    return Err(String::from(USAGE));
                };
                if value.is_empty() || value.starts_with('-') {
                    return Err(String::from(USAGE));
                }
                base = Some(value.clone());
            }
            _ => return Err(String::from(USAGE)),
        }
    }
    Ok(QuickArgs { base, mode: mode.unwrap_or(ScopeMode::Default) })
}

fn test_args(root: &Path, scope: &[String]) -> Vec<String> {
    if has_cargo_nextest() {
        let mut args = ci::test_args(root, false, None);
        args.retain(|arg| arg != "--workspace");
        args.extend(scope.iter().cloned());
        args
    } else {
        println!("ci quick: cargo-nextest is not installed; falling back to cargo test, which also runs the performance binaries");
        let mut args = vec![String::from("test"), String::from("--locked"), String::from("--all-targets")];
        args.extend(scope.iter().cloned());
        args
    }
}

fn print_plan(base: &str, changed: usize, mode: ScopeMode, quick_plan: &QuickPlan) {
    let mode_label = match mode {
        ScopeMode::Default => "",
        ScopeMode::Direct => " (direct only)",
        ScopeMode::Transitive => " (transitive tests)",
    };
    println!("ci quick{mode_label}: {changed} changed file(s) relative to {base}");
    if quick_plan.workspace_wide {
        println!("ci quick: a workspace-wide build input changed; linting and testing --workspace");
        return;
    }
    match &quick_plan.lint_selection {
        Selection::Workspace => println!("ci quick: lint packages: --workspace"),
        Selection::Packages(set) if set.is_empty() => {
            println!("ci quick: no workspace package affected");
            return;
        }
        Selection::Packages(set) => println!("ci quick: lint packages: {}", set.iter().cloned().collect::<Vec<_>>().join(", ")),
    }
    if quick_plan.test_selection != quick_plan.lint_selection {
        match &quick_plan.test_selection {
            Selection::Workspace => println!("ci quick: test packages: --workspace"),
            Selection::Packages(set) => println!("ci quick: test packages: {}", set.iter().cloned().collect::<Vec<_>>().join(", ")),
        }
        if mode == ScopeMode::Default {
            println!("ci quick: transitive reverse-dependent tests left to hosted CI (pass --transitive to run locally)");
        }
    }
    if !quick_plan.include_i18n {
        println!("ci quick: skipping i18n-report ({I18N_PACKAGE} not directly modified)");
    }
}

/// The merge base of `HEAD` with `main`, falling back to `origin/main`.
fn default_base(root: &Path) -> Result<String, String> {
    for candidate in ["main", "origin/main"] {
        if let Ok(base) = git_stdout(root, &["merge-base", "HEAD", candidate]) {
            let base = base.trim();
            if !base.is_empty() {
                return Ok(base.to_owned());
            }
        }
    }
    Err(String::from("ci quick: no merge base with `main` or `origin/main`; pass `--base <git-ref>`"))
}

/// Tracked files that differ from `base` in the working tree, plus untracked files.
fn changed_files(root: &Path, base: &str) -> Result<Vec<String>, String> {
    let diff = git_stdout(root, &["-c", "core.quotePath=false", "diff", "--name-only", "-z", base, "--"])?;
    let untracked = git_stdout(root, &["-c", "core.quotePath=false", "ls-files", "-z", "--others", "--exclude-standard"])?;
    let mut files: Vec<String> = diff.split('\0').chain(untracked.split('\0')).filter(|p| !p.is_empty()).map(str::to_owned).collect();
    files.sort();
    files.dedup();
    Ok(files)
}

fn git_stdout(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git").current_dir(root).args(args).output().map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("git {} utf-8: {e}", args.join(" ")))
}

/// `(package name, package directory relative to the workspace root with a trailing `/`)`.
///
/// When `workspace_members` is present (such as in a resolved `cargo metadata` snapshot),
/// directory ownership is restricted to workspace member package IDs.
fn package_dirs(meta: &Value) -> Result<Vec<(String, String)>, String> {
    let ws_root = meta.get("workspace_root").and_then(Value::as_str).ok_or_else(|| String::from("metadata: no workspace_root"))?;
    let pkgs = meta.get("packages").and_then(Value::as_array).ok_or_else(|| String::from("metadata: no packages array"))?;
    let member_ids: Option<BTreeSet<&str>> =
        meta.get("workspace_members").and_then(Value::as_array).map(|arr| arr.iter().filter_map(Value::as_str).collect());
    let mut dirs = Vec::with_capacity(member_ids.as_ref().map_or(pkgs.len(), BTreeSet::len));
    for p in pkgs {
        if let Some(ids) = &member_ids {
            let id = p.get("id").and_then(Value::as_str).ok_or_else(|| String::from("metadata: package without id"))?;
            if !ids.contains(id) {
                continue;
            }
        }
        let name = p.get("name").and_then(Value::as_str).ok_or_else(|| String::from("metadata: package without name"))?;
        let manifest = p.get("manifest_path").and_then(Value::as_str).ok_or_else(|| format!("metadata: `{name}` has no manifest_path"))?;
        let rel = Path::new(manifest)
            .strip_prefix(ws_root)
            .ok()
            .and_then(Path::parent)
            .ok_or_else(|| format!("metadata: `{name}` manifest `{manifest}` is outside the workspace root"))?;
        let rel = rel.to_string_lossy().replace('\\', "/");
        let dir = if rel.is_empty() { String::new() } else { format!("{rel}/") };
        dirs.push((name.to_owned(), dir));
    }
    Ok(dirs)
}

fn is_workspace_wide(path: &str) -> bool {
    WORKSPACE_WIDE_FILES.contains(&path) || WORKSPACE_WIDE_DIRS.iter().any(|dir| path.starts_with(dir))
}

/// The package owning `path`: the one with the longest directory prefix. A package rooted at
/// the workspace root (empty directory) never claims files, since its manifest is workspace-wide.
fn owning_package<'a>(path: &str, dirs: &'a [(String, String)]) -> Option<&'a str> {
    dirs.iter().filter(|(_, dir)| !dir.is_empty() && path.starts_with(dir.as_str())).max_by_key(|(_, dir)| dir.len()).map(|(name, _)| name.as_str())
}

fn collapse_selection(selected: BTreeSet<String>, graph: &[Crate]) -> Selection {
    if !graph.is_empty() && graph.iter().all(|krate| selected.contains(&krate.name)) { Selection::Workspace } else { Selection::Packages(selected) }
}

/// Build the full quick-tier execution plan from changed paths, package directories, and the
/// workspace dependency graph.
///
/// - Workspace-wide inputs select `--workspace` for lint + tests and enable `i18n-report`.
/// - `ScopeMode::Default`: `lint_selection` closes over workspace reverse dependencies (normal,
///   dev, and build) while `test_selection` stays scoped to `direct_packages`.
/// - `ScopeMode::Direct`: both `lint_selection` and `test_selection` stay scoped to
///   `direct_packages`.
/// - `ScopeMode::Transitive`: both `lint_selection` and `test_selection` close over workspace
///   reverse dependencies, omitting `xtask` from `test_selection` when only pulled in transitively.
/// - `include_i18n` is true only on workspace-wide edits or direct edits to `curvebook-ui-egui`.
pub fn plan(changed: &[String], dirs: &[(String, String)], graph: &[Crate], mode: ScopeMode) -> QuickPlan {
    let workspace_wide = changed.iter().any(|path| is_workspace_wide(path));
    let direct_packages: BTreeSet<String> = changed.iter().filter_map(|path| owning_package(path, dirs)).map(str::to_owned).collect();
    if workspace_wide {
        return QuickPlan {
            workspace_wide: true,
            direct_packages,
            lint_selection: Selection::Workspace,
            test_selection: Selection::Workspace,
            include_i18n: true,
        };
    }

    let include_i18n = direct_packages.contains(I18N_PACKAGE);
    let mut lint_packages = direct_packages.clone();
    if mode != ScopeMode::Direct {
        loop {
            let before = lint_packages.len();
            for krate in graph {
                if !lint_packages.contains(&krate.name) && krate.deps.iter().any(|dep| dep.workspace && lint_packages.contains(&dep.name)) {
                    lint_packages.insert(krate.name.clone());
                }
            }
            if lint_packages.len() == before {
                break;
            }
        }
    }

    let test_packages = match mode {
        ScopeMode::Default | ScopeMode::Direct => direct_packages.clone(),
        ScopeMode::Transitive => {
            let mut set = lint_packages.clone();
            if !direct_packages.contains(XTASK_PACKAGE) {
                set.remove(XTASK_PACKAGE);
            }
            set
        }
    };

    QuickPlan {
        workspace_wide: false,
        direct_packages,
        lint_selection: collapse_selection(lint_packages, graph),
        test_selection: collapse_selection(test_packages, graph),
        include_i18n,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{QuickArgs, ScopeMode, Selection, package_dirs, parse_args, plan};
    use crate::layers::{self, Crate, Dep, DepKind};

    fn select(changed: &[String], dirs: &[(String, String)], graph: &[Crate]) -> Selection {
        plan(changed, dirs, graph, ScopeMode::Default).lint_selection
    }

    fn dirs() -> Vec<(String, String)> {
        [("curvebook-math", "crates/math/"), ("curvebook-stats", "crates/stats/"), ("curvebook-ui-egui", "crates/ui-egui/"), ("xtask", "xtask/")]
            .into_iter()
            .map(|(n, d)| (n.to_owned(), d.to_owned()))
            .collect()
    }

    fn dep(name: &str, kind: DepKind) -> Dep {
        Dep { name: name.to_owned(), kind, workspace: true }
    }

    fn graph() -> Vec<Crate> {
        vec![
            Crate { name: String::from("curvebook-math"), deps: vec![Dep { name: String::from("libm"), kind: DepKind::Normal, workspace: false }] },
            Crate { name: String::from("curvebook-stats"), deps: vec![dep("curvebook-math", DepKind::Normal)] },
            Crate { name: String::from("curvebook-ui-egui"), deps: vec![dep("curvebook-stats", DepKind::Normal)] },
            Crate { name: String::from("xtask"), deps: vec![dep("curvebook-ui-egui", DepKind::Dev)] },
        ]
    }

    fn files(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|&p| p.to_owned()).collect()
    }

    fn set(names: &[&str]) -> Selection {
        Selection::Packages(names.iter().map(|&n| n.to_owned()).collect::<BTreeSet<_>>())
    }

    #[test]
    fn documentation_only_changes_select_no_package() {
        let changed = files(&["docs/ROADMAP.md", "AGENTS.md"]);
        assert_eq!(select(&changed, &dirs(), &graph()), set(&[]));
    }

    #[test]
    fn policy_and_shared_harness_inputs_select_workspace() {
        for path in [
            ".config/nextest.toml",
            "deny.toml",
            "assets/app-icon/icon.png",
            "docs/control-protocol.md",
            "docs/evidence/overview.md",
            "docs/release/github-pages.md",
            "docs/implementation/scripting-and-automation.md",
            "docs/schemas/document.schema.json",
            "corpus/nist-strd/Misra1a.dat",
            ".github/workflows/ci.yml",
            "packaging/windows/installer.wxs",
            "packaging/macos/Info.plist.in",
            "packaging/linux/package.sh",
            "packaging/web/_headers",
            "packaging/release-channel.sh",
        ] {
            assert_eq!(select(&files(&[path]), &dirs(), &graph()), Selection::Workspace, "{path}");
        }
    }

    #[test]
    fn a_leaf_change_selects_only_that_package() {
        assert_eq!(select(&files(&["xtask/src/ci.rs"]), &dirs(), &graph()), set(&["xtask"]));
    }

    #[test]
    fn a_change_selects_transitive_reverse_dependents_including_dev_dependents() {
        let changed = files(&["crates/stats/src/anova.rs"]);
        assert_eq!(select(&changed, &dirs(), &graph()), set(&["curvebook-stats", "curvebook-ui-egui", "xtask"]));
    }

    #[test]
    fn a_change_reaching_every_package_or_a_build_input_selects_the_workspace() {
        assert_eq!(select(&files(&["crates/math/src/lib.rs"]), &dirs(), &graph()), Selection::Workspace);
        for wide in ["Cargo.lock", "Cargo.toml", "rust-toolchain.toml", "clippy.toml", ".cargo/config.toml"] {
            assert_eq!(select(&files(&[wide, "docs/x.md"]), &dirs(), &graph()), Selection::Workspace, "{wide}");
        }
    }

    #[test]
    fn a_crate_manifest_is_owned_by_its_package_not_the_workspace() {
        assert_eq!(select(&files(&["crates/ui-egui/Cargo.toml"]), &dirs(), &graph()), set(&["curvebook-ui-egui", "xtask"]));
    }

    #[test]
    fn prefix_matching_respects_directory_boundaries() {
        let mut dirs = dirs();
        dirs.push((String::from("curvebook-mathx"), String::from("crates/mathx/")));
        let changed = files(&["crates/mathx/src/lib.rs"]);
        assert_eq!(select(&changed, &dirs, &graph()), set(&["curvebook-mathx"]));
    }

    #[test]
    fn base_direct_and_transitive_argument_parsing() {
        assert_eq!(parse_args(&[]), Ok(QuickArgs { base: None, mode: ScopeMode::Default }));
        assert_eq!(parse_args(&[String::from("--direct")]), Ok(QuickArgs { base: None, mode: ScopeMode::Direct }));
        assert_eq!(parse_args(&[String::from("--transitive")]), Ok(QuickArgs { base: None, mode: ScopeMode::Transitive }));
        assert_eq!(
            parse_args(&[String::from("--base"), String::from("HEAD~2")]),
            Ok(QuickArgs { base: Some(String::from("HEAD~2")), mode: ScopeMode::Default })
        );
        assert_eq!(
            parse_args(&[String::from("--direct"), String::from("--base"), String::from("HEAD~2")]),
            Ok(QuickArgs { base: Some(String::from("HEAD~2")), mode: ScopeMode::Direct })
        );
        assert_eq!(
            parse_args(&[String::from("--base"), String::from("HEAD~2"), String::from("--transitive")]),
            Ok(QuickArgs { base: Some(String::from("HEAD~2")), mode: ScopeMode::Transitive })
        );
        assert!(parse_args(&[String::from("--base")]).is_err());
        assert!(parse_args(&[String::from("--base"), String::from("--direct")]).is_err());
        assert!(parse_args(&[String::from("--base"), String::from("--workspace")]).is_err());
        assert!(parse_args(&[String::from("--direct"), String::from("--direct")]).is_err());
        assert!(parse_args(&[String::from("--direct"), String::from("--transitive")]).is_err());
        assert!(parse_args(&[String::from("--transitive"), String::from("--direct")]).is_err());
        assert!(parse_args(&[String::from("--base"), String::from("HEAD~1"), String::from("--base"), String::from("HEAD~2")]).is_err());
        assert!(parse_args(&[String::from("--fast")]).is_err());
    }

    #[test]
    fn i18n_report_requires_direct_ui_touch_or_workspace_wide_input() {
        let math_plan = plan(&files(&["crates/math/src/lib.rs"]), &dirs(), &graph(), ScopeMode::Default);
        assert!(!math_plan.include_i18n, "transitive UI pull from math edit must skip i18n-report");

        let ui_plan = plan(&files(&["crates/ui-egui/src/i18n.rs"]), &dirs(), &graph(), ScopeMode::Default);
        assert!(ui_plan.include_i18n, "direct curvebook-ui-egui edit must enable i18n-report");

        let root_manifest_plan = plan(&files(&["Cargo.toml"]), &dirs(), &graph(), ScopeMode::Default);
        assert!(root_manifest_plan.include_i18n, "workspace-wide input must enable i18n-report");
    }

    #[test]
    fn default_mode_lints_reverse_dependents_and_tests_direct_packages_only() {
        let math_plan = plan(&files(&["crates/math/src/lib.rs"]), &dirs(), &graph(), ScopeMode::Default);
        assert_eq!(math_plan.lint_selection, Selection::Workspace);
        assert_eq!(math_plan.test_selection, set(&["curvebook-math"]));

        let stats_plan = plan(&files(&["crates/stats/src/anova.rs"]), &dirs(), &graph(), ScopeMode::Default);
        assert_eq!(stats_plan.lint_selection, set(&["curvebook-stats", "curvebook-ui-egui", "xtask"]));
        assert_eq!(stats_plan.test_selection, set(&["curvebook-stats"]));

        let xtask_plan = plan(&files(&["xtask/src/ci.rs"]), &dirs(), &graph(), ScopeMode::Default);
        assert_eq!(xtask_plan.lint_selection, set(&["xtask"]));
        assert_eq!(xtask_plan.test_selection, set(&["xtask"]));

        let wide_plan = plan(&files(&["Cargo.toml"]), &dirs(), &graph(), ScopeMode::Default);
        assert_eq!(wide_plan.lint_selection, Selection::Workspace);
        assert_eq!(wide_plan.test_selection, Selection::Workspace);
    }

    #[test]
    fn transitive_mode_tests_reverse_dependents_and_omits_transitive_xtask() {
        let math_plan = plan(&files(&["crates/math/src/lib.rs"]), &dirs(), &graph(), ScopeMode::Transitive);
        assert_eq!(math_plan.lint_selection, Selection::Workspace);
        assert_eq!(math_plan.test_selection, set(&["curvebook-math", "curvebook-stats", "curvebook-ui-egui"]));

        let stats_plan = plan(&files(&["crates/stats/src/anova.rs"]), &dirs(), &graph(), ScopeMode::Transitive);
        assert_eq!(stats_plan.lint_selection, set(&["curvebook-stats", "curvebook-ui-egui", "xtask"]));
        assert_eq!(stats_plan.test_selection, set(&["curvebook-stats", "curvebook-ui-egui"]));

        let mixed_plan = plan(&files(&["crates/stats/src/anova.rs", "xtask/src/ci.rs"]), &dirs(), &graph(), ScopeMode::Transitive);
        assert_eq!(mixed_plan.test_selection, set(&["curvebook-stats", "curvebook-ui-egui", "xtask"]));
    }

    #[test]
    fn direct_flag_restricts_selection_to_directly_modified_crates_except_workspace_wide_inputs() {
        let math_direct = plan(&files(&["crates/math/src/lib.rs"]), &dirs(), &graph(), ScopeMode::Direct);
        assert_eq!(math_direct.lint_selection, set(&["curvebook-math"]));
        assert_eq!(math_direct.test_selection, set(&["curvebook-math"]));
        assert!(!math_direct.include_i18n);

        let ui_direct = plan(&files(&["crates/ui-egui/src/i18n.rs"]), &dirs(), &graph(), ScopeMode::Direct);
        assert_eq!(ui_direct.lint_selection, set(&["curvebook-ui-egui"]));
        assert_eq!(ui_direct.test_selection, set(&["curvebook-ui-egui"]));
        assert!(ui_direct.include_i18n);

        let wide_direct = plan(&files(&["Cargo.toml"]), &dirs(), &graph(), ScopeMode::Direct);
        assert_eq!(wide_direct.lint_selection, Selection::Workspace);
        assert_eq!(wide_direct.test_selection, Selection::Workspace);
        assert!(wide_direct.include_i18n);
    }

    #[test]
    fn package_dirs_restricts_resolved_metadata_to_workspace_members() {
        let meta: serde_json::Value = serde_json::from_str(
            r#"{
                "workspace_root": "/repo",
                "workspace_members": ["math 0.1.0", "stats 0.1.0"],
                "packages": [
                    {"id": "math 0.1.0", "name": "curvebook-math", "manifest_path": "/repo/crates/math/Cargo.toml", "dependencies": []},
                    {"id": "stats 0.1.0", "name": "curvebook-stats", "manifest_path": "/repo/crates/stats/Cargo.toml", "dependencies": []},
                    {"id": "serde 1.0.0", "name": "serde", "manifest_path": "/cargo/registry/serde-1.0.0/Cargo.toml", "dependencies": []}
                ],
                "resolve": {
                    "nodes": [
                        {"id": "math 0.1.0", "deps": [{"pkg": "serde 1.0.0", "dep_kinds": [{"kind": null}]}]},
                        {"id": "stats 0.1.0", "deps": [{"pkg": "math 0.1.0", "dep_kinds": [{"kind": null}]}]},
                        {"id": "serde 1.0.0", "deps": []}
                    ]
                }
            }"#,
        )
        .unwrap();
        let dirs = package_dirs(&meta).unwrap();
        assert_eq!(
            dirs,
            vec![(String::from("curvebook-math"), String::from("crates/math/")), (String::from("curvebook-stats"), String::from("crates/stats/")),]
        );
        let graph = layers::from_metadata(&meta).unwrap();
        assert_eq!(select(&files(&["crates/math/src/lib.rs"]), &dirs, &graph), Selection::Workspace);
        assert_eq!(select(&files(&["crates/stats/src/lib.rs"]), &dirs, &graph), set(&["curvebook-stats"]));
    }
}
