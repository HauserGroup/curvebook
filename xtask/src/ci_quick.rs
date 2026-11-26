//! `cargo xtask ci quick`: fast local tier — policy gates, reverse-dep clippy, and direct-crate
//! nextest (`--direct` / `--transitive` / `--base <ref>`).

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use serde_json::Value;

use crate::layers::Crate;
use crate::{cargo_metadata_resolved, ci, has_cargo_nextest, layers, run_cargo, run_cargo_strs};

const WORKSPACE_WIDE_FILES: &str = "Cargo.toml Cargo.lock rust-toolchain.toml rust-toolchain clippy.toml .clippy.toml .config/nextest.toml deny.toml docs/control-protocol.md docs/evidence/overview.md docs/release/github-pages.md docs/implementation/scripting-and-automation.md";
const WORKSPACE_WIDE_DIRS: &[&str] = &[".cargo/", ".github/", "assets/app-icon/", "packaging/", "docs/schemas/", "corpus/nist-strd/"];
const I18N_PACKAGE: &str = "curvebook-ui-egui";
const XTASK_PACKAGE: &str = "xtask";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    Workspace,
    Packages(BTreeSet<String>),
}

impl Selection {
    fn is_empty(&self) -> bool {
        matches!(self, Self::Packages(set) if set.is_empty())
    }
    fn contains_package(&self, pkg: &str) -> bool {
        match self {
            Self::Workspace => true,
            Self::Packages(s) => s.contains(pkg),
        }
    }
    fn cargo_scope(&self) -> Vec<String> {
        match self {
            Self::Workspace => vec![String::from("--workspace")],
            Self::Packages(set) => set.iter().flat_map(|p| [String::from("-p"), p.clone()]).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickPlan {
    pub workspace_wide: bool,
    pub direct_packages: BTreeSet<String>,
    pub lint_selection: Selection,
    pub test_selection: Selection,
    pub include_i18n: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeMode {
    Default,
    Direct,
    Transitive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QuickArgs {
    base: Option<String>,
    mode: ScopeMode,
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let (parsed, start) = (parse_args(args)?, Instant::now());
    let base = match parsed.base {
        Some(b) => b,
        None => default_base(root)?,
    };
    let (changed, meta) = (changed_files(root, &base)?, cargo_metadata_resolved(root)?);
    let (dirs, graph) = (package_dirs(&meta)?, layers::from_metadata(&meta)?);
    let quick_plan = plan(&changed, &dirs, &graph, parsed.mode);
    print_plan(&base, changed.len(), parsed.mode, &quick_plan);

    println!("ci quick: starting policy");
    ci::policy(root, quick_plan.include_i18n)?;
    if quick_plan.lint_selection.is_empty() && quick_plan.test_selection.is_empty() {
        println!("ci quick: no Rust package changed; skipping lint and tests");
    } else {
        if !quick_plan.lint_selection.is_empty() {
            let mut lint = vec![String::from("clippy"), String::from("--locked"), String::from("--all-targets")];
            lint.extend(quick_plan.lint_selection.cargo_scope());
            lint.extend([String::from("--"), String::from("-D"), String::from("warnings")]);
            run_cargo(root, &lint)?;
            if quick_plan.lint_selection.contains_package(XTASK_PACKAGE) {
                run_cargo_strs(root, ci::DOCS_SITE_CLIPPY_ARGS)?;
            }
        }
        if !quick_plan.test_selection.is_empty() {
            run_cargo(root, &test_args(root, &quick_plan.test_selection.cargo_scope()))?;
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
    let (mut base, mut mode, mut iter) = (None, None, args.iter());
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--direct" if mode.replace(ScopeMode::Direct).is_none() => {}
            "--transitive" if mode.replace(ScopeMode::Transitive).is_none() => {}
            "--base" if base.is_none() => {
                base = Some(iter.next().filter(|v| !v.is_empty() && !v.starts_with('-')).ok_or_else(|| String::from(USAGE))?.clone());
            }
            _ => return Err(String::from(USAGE)),
        }
    }
    Ok(QuickArgs { base, mode: mode.unwrap_or(ScopeMode::Default) })
}

fn test_args(root: &Path, scope: &[String]) -> Vec<String> {
    let mut args = if has_cargo_nextest() {
        let mut a = ci::test_args(root, false, None);
        a.retain(|arg| arg != "--workspace");
        a
    } else {
        println!("ci quick: cargo-nextest is not installed; falling back to cargo test, which also runs the performance binaries");
        vec![String::from("test"), String::from("--locked"), String::from("--all-targets")]
    };
    args.extend_from_slice(scope);
    args
}

fn print_plan(base: &str, changed: usize, mode: ScopeMode, quick_plan: &QuickPlan) {
    let label = match mode {
        ScopeMode::Default => "",
        ScopeMode::Direct => " (direct only)",
        ScopeMode::Transitive => " (transitive tests)",
    };
    println!("ci quick{label}: {changed} changed file(s) relative to {base}");
    if quick_plan.workspace_wide {
        return println!("ci quick: a workspace-wide build input changed; linting and testing --workspace");
    }
    if quick_plan.lint_selection.is_empty() {
        return println!("ci quick: no workspace package affected");
    }
    let fmt_sel = |s: &Selection| match s {
        Selection::Workspace => String::from("--workspace"),
        Selection::Packages(set) => set.iter().cloned().collect::<Vec<_>>().join(", "),
    };
    println!("ci quick: lint packages: {}", fmt_sel(&quick_plan.lint_selection));
    if quick_plan.test_selection != quick_plan.lint_selection {
        println!("ci quick: test packages: {}", fmt_sel(&quick_plan.test_selection));
        if mode == ScopeMode::Default {
            println!("ci quick: transitive reverse-dependent tests left to hosted CI (pass --transitive to run locally)");
        }
    }
    if !quick_plan.include_i18n {
        println!("ci quick: skipping i18n-report ({I18N_PACKAGE} not directly modified)");
    }
}

fn default_base(root: &Path) -> Result<String, String> {
    for candidate in ["main", "origin/main"] {
        if let Ok(base) = git_stdout(root, &["merge-base", "HEAD", candidate])
            && !base.trim().is_empty()
        {
            return Ok(base.trim().to_owned());
        }
    }
    Err(String::from("ci quick: no merge base with `main` or `origin/main`; pass `--base <git-ref>`"))
}

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

fn package_dirs(meta: &Value) -> Result<Vec<(String, String)>, String> {
    let ws_root = meta.get("workspace_root").and_then(Value::as_str).ok_or("metadata: no workspace_root")?;
    let pkgs = meta.get("packages").and_then(Value::as_array).ok_or("metadata: no packages array")?;
    let member_ids: Option<BTreeSet<&str>> =
        meta.get("workspace_members").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect());
    let mut dirs = Vec::with_capacity(member_ids.as_ref().map_or(pkgs.len(), BTreeSet::len));
    for p in pkgs {
        if let Some(ids) = &member_ids
            && !ids.contains(p.get("id").and_then(Value::as_str).ok_or("metadata: package without id")?)
        {
            continue;
        }
        let name = p.get("name").and_then(Value::as_str).ok_or("metadata: package without name")?;
        let manifest = p.get("manifest_path").and_then(Value::as_str).ok_or_else(|| format!("metadata: `{name}` has no manifest_path"))?;
        let rel = Path::new(manifest)
            .strip_prefix(ws_root)
            .ok()
            .and_then(Path::parent)
            .ok_or_else(|| format!("metadata: `{name}` manifest `{manifest}` is outside the workspace root"))?
            .to_string_lossy()
            .replace('\\', "/");
        dirs.push((name.to_owned(), if rel.is_empty() { String::new() } else { format!("{rel}/") }));
    }
    Ok(dirs)
}

fn is_workspace_wide(path: &str) -> bool {
    WORKSPACE_WIDE_FILES.split_ascii_whitespace().any(|f| f == path) || WORKSPACE_WIDE_DIRS.iter().any(|dir| path.starts_with(dir))
}

fn owning_package<'a>(path: &str, dirs: &'a [(String, String)]) -> Option<&'a str> {
    dirs.iter().filter(|(_, dir)| !dir.is_empty() && path.starts_with(dir.as_str())).max_by_key(|(_, dir)| dir.len()).map(|(name, _)| name.as_str())
}

fn collapse_selection(selected: BTreeSet<String>, graph: &[Crate]) -> Selection {
    if !graph.is_empty() && graph.iter().all(|k| selected.contains(&k.name)) { Selection::Workspace } else { Selection::Packages(selected) }
}

pub fn plan(changed: &[String], dirs: &[(String, String)], graph: &[Crate], mode: ScopeMode) -> QuickPlan {
    let direct_packages: BTreeSet<String> = changed.iter().filter_map(|p| owning_package(p, dirs)).map(str::to_owned).collect();
    if changed.iter().any(|p| is_workspace_wide(p)) {
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
                if !lint_packages.contains(&krate.name) && krate.deps.iter().any(|d| d.workspace && lint_packages.contains(&d.name)) {
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
    use super::{QuickArgs, ScopeMode, Selection, package_dirs, parse_args, plan};
    use crate::layers::{self, Crate, Dep, DepKind};
    use std::collections::BTreeSet;

    fn files(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|&p| p.to_owned()).collect()
    }
    fn set(names: &[&str]) -> Selection {
        Selection::Packages(names.iter().map(|&n| n.to_owned()).collect::<BTreeSet<_>>())
    }

    #[test]
    fn quick_plan_modes_workspace_wide_parse_args_and_metadata() {
        let mut d: Vec<_> =
            [("curvebook-math", "crates/math/"), ("curvebook-stats", "crates/stats/"), ("curvebook-ui-egui", "crates/ui-egui/"), ("xtask", "xtask/")]
                .into_iter()
                .map(|(n, p)| (n.to_owned(), p.to_owned()))
                .collect();
        let ws = |n: &str, k| Dep { name: n.to_owned(), kind: k, workspace: true };
        let g = vec![
            Crate { name: "curvebook-math".into(), deps: vec![Dep { name: "libm".into(), kind: DepKind::Normal, workspace: false }] },
            Crate { name: "curvebook-stats".into(), deps: vec![ws("curvebook-math", DepKind::Normal)] },
            Crate { name: "curvebook-ui-egui".into(), deps: vec![ws("curvebook-stats", DepKind::Normal)] },
            Crate { name: "xtask".into(), deps: vec![ws("curvebook-ui-egui", DepKind::Dev)] },
        ];
        assert_eq!(plan(&files(&["docs/ROADMAP.md", "AGENTS.md"]), &d, &g, ScopeMode::Default).lint_selection, set(&[]));
        for wide in [
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            "clippy.toml",
            ".cargo/config.toml",
            ".config/nextest.toml",
            "deny.toml",
            "assets/app-icon/icon.png",
            "docs/control-protocol.md",
            "docs/schemas/document.schema.json",
            "corpus/nist-strd/Misra1a.dat",
            ".github/workflows/ci.yml",
            "packaging/web/_headers",
        ] {
            let p = plan(&files(&[wide, "docs/x.md"]), &d, &g, ScopeMode::Direct);
            assert_eq!((p.lint_selection, p.test_selection, p.include_i18n), (Selection::Workspace, Selection::Workspace, true), "{wide}");
        }
        for (paths, mode, exp_l, exp_t, exp_i) in [
            (&["xtask/src/ci.rs"][..], ScopeMode::Default, set(&["xtask"]), set(&["xtask"]), false),
            (
                &["crates/stats/src/anova.rs"],
                ScopeMode::Default,
                set(&["curvebook-stats", "curvebook-ui-egui", "xtask"]),
                set(&["curvebook-stats"]),
                false,
            ),
            (&["crates/math/src/lib.rs"], ScopeMode::Default, Selection::Workspace, set(&["curvebook-math"]), false),
            (&["crates/ui-egui/Cargo.toml"], ScopeMode::Default, set(&["curvebook-ui-egui", "xtask"]), set(&["curvebook-ui-egui"]), true),
            (
                &["crates/math/src/lib.rs"],
                ScopeMode::Transitive,
                Selection::Workspace,
                set(&["curvebook-math", "curvebook-stats", "curvebook-ui-egui"]),
                false,
            ),
            (
                &["crates/stats/src/anova.rs", "xtask/src/ci.rs"],
                ScopeMode::Transitive,
                set(&["curvebook-stats", "curvebook-ui-egui", "xtask"]),
                set(&["curvebook-stats", "curvebook-ui-egui", "xtask"]),
                false,
            ),
            (&["crates/math/src/lib.rs"], ScopeMode::Direct, set(&["curvebook-math"]), set(&["curvebook-math"]), false),
            (&["crates/ui-egui/src/i18n.rs"], ScopeMode::Direct, set(&["curvebook-ui-egui"]), set(&["curvebook-ui-egui"]), true),
        ] {
            let p = plan(&files(paths), &d, &g, mode);
            assert_eq!((p.lint_selection, p.test_selection, p.include_i18n), (exp_l, exp_t, exp_i));
        }
        d.push(("curvebook-mathx".into(), "crates/mathx/".into()));
        assert_eq!(plan(&files(&["crates/mathx/src/lib.rs"]), &d, &g, ScopeMode::Default).lint_selection, set(&["curvebook-mathx"]));

        for (argv, exp) in [
            (&[][..], Ok(QuickArgs { base: None, mode: ScopeMode::Default })),
            (&["--direct"], Ok(QuickArgs { base: None, mode: ScopeMode::Direct })),
            (&["--base", "HEAD~2", "--transitive"], Ok(QuickArgs { base: Some("HEAD~2".into()), mode: ScopeMode::Transitive })),
        ] {
            assert_eq!(parse_args(&files(argv)), exp);
        }
        for bad in [&["--base"][..], &["--base", "--direct"], &["--direct", "--transitive"], &["--base", "a", "--base", "b"], &["--fast"]] {
            assert!(parse_args(&files(bad)).is_err());
        }
        let meta: serde_json::Value = serde_json::from_str(r#"{"workspace_root":"/repo","workspace_members":["math 0.1.0","stats 0.1.0"],"packages":[{"id":"math 0.1.0","name":"curvebook-math","manifest_path":"/repo/crates/math/Cargo.toml","dependencies":[]},{"id":"stats 0.1.0","name":"curvebook-stats","manifest_path":"/repo/crates/stats/Cargo.toml","dependencies":[]},{"id":"serde 1.0.0","name":"serde","manifest_path":"/cargo/registry/serde-1.0.0/Cargo.toml","dependencies":[]}],"resolve":{"nodes":[{"id":"math 0.1.0","deps":[{"pkg":"serde 1.0.0","dep_kinds":[{"kind":null}]}]},{"id":"stats 0.1.0","deps":[{"pkg":"math 0.1.0","dep_kinds":[{"kind":null}]}]},{"id":"serde 1.0.0","deps":[]}]}}"#).unwrap();
        let pd = package_dirs(&meta).unwrap();
        let pg = layers::from_metadata(&meta).unwrap();
        assert_eq!(
            (&pd, plan(&files(&["crates/math/src/lib.rs"]), &pd, &pg, ScopeMode::Default).lint_selection),
            (&vec![("curvebook-math".into(), "crates/math/".into()), ("curvebook-stats".into(), "crates/stats/".into())], Selection::Workspace)
        );
    }
}
