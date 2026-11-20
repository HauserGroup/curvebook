//! `cargo xtask ci quick`: the fast local tier of the CI gates.
//!
//! Runs every policy gate, then Clippy and the non-performance nextest suite for only the
//! workspace packages a change touches plus the workspace packages that depend on them.
//! Changes are measured against the merge base with `main` (or `--base <ref>`), including
//! staged, unstaged, and untracked files. Edits to workspace-wide build inputs (the root
//! manifest, lockfile, toolchain, or lint configuration) select the whole workspace.
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
use crate::{cargo_metadata, cargo_metadata_resolved, ci, has_cargo_nextest, layers, run_cargo};

/// Root-level files whose edits can change how every workspace package builds or lints.
const WORKSPACE_WIDE_FILES: &[&str] = &["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "rust-toolchain", "clippy.toml", ".clippy.toml"];
/// Directories whose contents can change how every workspace package builds.
const WORKSPACE_WIDE_DIRS: &[&str] = &[".cargo/"];
/// The package whose test library hosts the untranslated-locale report run by `policy`.
const I18N_PACKAGE: &str = "curvebook-ui-egui";

/// Which workspace packages the quick tier lints and tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// A workspace-wide build input changed: lint and test `--workspace`.
    Workspace,
    /// Only these packages (possibly none, for documentation-only changes).
    Packages(BTreeSet<String>),
}

impl Selection {
    fn includes(&self, package: &str) -> bool {
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

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let base_arg = parse_args(args)?;
    let start = Instant::now();
    let base = match base_arg {
        Some(base) => base,
        None => default_base(root)?,
    };
    let changed = changed_files(root, &base)?;
    let dirs = package_dirs(&cargo_metadata(root)?)?;
    let graph = layers::from_metadata(&cargo_metadata_resolved(root)?)?;
    let selection = select(&changed, &dirs, &graph);
    print_plan(&base, changed.len(), &selection);

    println!("ci quick: starting policy");
    ci::policy(root, selection.includes(I18N_PACKAGE))?;
    if selection == Selection::Packages(BTreeSet::new()) {
        println!("ci quick: no Rust package changed; skipping lint and tests");
    } else {
        let scope = selection.cargo_scope();
        let mut lint = vec![String::from("clippy"), String::from("--locked"), String::from("--all-targets")];
        lint.extend(scope.iter().cloned());
        lint.extend([String::from("--"), String::from("-D"), String::from("warnings")]);
        run_cargo(root, &lint)?;
        run_cargo(root, &test_args(root, &scope))?;
    }
    println!("ci quick: passed in {:.2}s (full gate: `cargo xtask ci` or hosted CI on the pull request)", start.elapsed().as_secs_f64());
    Ok(())
}

fn parse_args(args: &[String]) -> Result<Option<String>, String> {
    match args {
        [] => Ok(None),
        [flag, base] if flag == "--base" && !base.is_empty() && !base.starts_with('-') => Ok(Some(base.clone())),
        _ => Err(String::from("usage: cargo xtask ci quick [--base <git-ref>]")),
    }
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

fn print_plan(base: &str, changed: usize, selection: &Selection) {
    println!("ci quick: {changed} changed file(s) relative to {base}");
    match selection {
        Selection::Workspace => println!("ci quick: a workspace-wide build input changed; linting and testing --workspace"),
        Selection::Packages(set) if set.is_empty() => println!("ci quick: no workspace package affected"),
        Selection::Packages(set) => println!("ci quick: affected packages: {}", set.iter().cloned().collect::<Vec<_>>().join(", ")),
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
fn package_dirs(meta: &Value) -> Result<Vec<(String, String)>, String> {
    let ws_root = meta.get("workspace_root").and_then(Value::as_str).ok_or_else(|| String::from("metadata: no workspace_root"))?;
    let pkgs = meta.get("packages").and_then(Value::as_array).ok_or_else(|| String::from("metadata: no packages array"))?;
    let mut dirs = Vec::with_capacity(pkgs.len());
    for p in pkgs {
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

/// Packages owning a changed file, closed over workspace reverse dependencies of every kind
/// (normal, dev, and build), so dependents' tests run against the change.
pub fn select(changed: &[String], dirs: &[(String, String)], graph: &[Crate]) -> Selection {
    if changed.iter().any(|path| is_workspace_wide(path)) {
        return Selection::Workspace;
    }
    let mut selected: BTreeSet<String> = changed.iter().filter_map(|path| owning_package(path, dirs)).map(str::to_owned).collect();
    loop {
        let before = selected.len();
        for krate in graph {
            if !selected.contains(&krate.name) && krate.deps.iter().any(|dep| dep.workspace && selected.contains(&dep.name)) {
                selected.insert(krate.name.clone());
            }
        }
        if selected.len() == before {
            break;
        }
    }
    if !graph.is_empty() && graph.iter().all(|krate| selected.contains(&krate.name)) {
        return Selection::Workspace;
    }
    Selection::Packages(selected)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{Selection, parse_args, select};
    use crate::layers::{Crate, Dep, DepKind};

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
        let changed = files(&["docs/ROADMAP.md", "AGENTS.md", ".github/workflows/ci.yml"]);
        assert_eq!(select(&changed, &dirs(), &graph()), set(&[]));
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
    fn base_argument_parsing_rejects_missing_or_flag_values() {
        assert_eq!(parse_args(&[]), Ok(None));
        assert_eq!(parse_args(&[String::from("--base"), String::from("HEAD~2")]), Ok(Some(String::from("HEAD~2"))));
        assert!(parse_args(&[String::from("--base")]).is_err());
        assert!(parse_args(&[String::from("--base"), String::from("--workspace")]).is_err());
        assert!(parse_args(&[String::from("--fast")]).is_err());
    }
}
