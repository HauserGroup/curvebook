//! `cargo xtask workflows`: lint GitHub Actions workflows with `actionlint` (using
//! `.github/actionlint.yaml`) and tracked/hook shell scripts with `shellcheck` (`R0.6`).

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use crate::scan_util::{has_ext_ci, is_real_dir, is_regular_file, read_dir_paths_sorted, rel_slash_lossy, walk_rel_files};

const ACTIONLINT_CONFIG: &str = ".github/actionlint.yaml";
const REQUIRE_WORKFLOW_LINT_ENV: &str = "CURVEBOOK_REQUIRE_WORKFLOW_LINT";

/// Parse whether the `CI` environment variable enables strict CI enforcement.
///
/// Unset, empty, `0`, `false`, `no`, and `off` (case-insensitive) return `false`.
#[must_use]
pub fn parse_ci_env_value(raw: Option<&OsStr>) -> bool {
    raw.is_some_and(|v| !matches!(v.to_string_lossy().trim().to_ascii_lowercase().as_str(), "" | "0" | "false" | "no" | "off"))
}

/// Return `true` when the process `CI` environment variable is truthy.
#[must_use]
pub(crate) fn is_ci_env() -> bool {
    parse_ci_env_value(std::env::var_os("CI").as_deref())
}

/// Execute `cargo xtask workflows`.
pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("usage: cargo xtask workflows (unexpected args: `{}`)", args.join(" ")));
    }

    let config_path = root.join(ACTIONLINT_CONFIG);
    if !config_path.is_file() {
        return Err(format!("missing `{ACTIONLINT_CONFIG}` in {}", root.display()));
    }

    let workflows = collect_workflow_files(root)?;
    if workflows.is_empty() {
        return Err(format!("no `.yml`/`.yaml` workflows found in `{}/.github/workflows`", root.display()));
    }
    check_workflows_offline_yaml(root, &workflows)?;
    let actions = collect_action_files(root)?;
    check_action_files_offline_yaml(root, &actions)?;

    let shell_scripts = collect_shell_scripts(root)?;
    let in_ci = is_ci_env();
    let require_lint = in_ci || parse_ci_env_value(std::env::var_os(REQUIRE_WORKFLOW_LINT_ENV).as_deref());

    let actionlint_ran =
        run_linter_if_installed(root, "actionlint", "actionlint", &["-config-file", ACTIONLINT_CONFIG], &workflows, in_ci, require_lint)?;
    let shellcheck_ran =
        !shell_scripts.is_empty() && run_linter_if_installed(root, "shellcheck", "shellcheck -x", &["-x"], &shell_scripts, in_ci, require_lint)?;

    if actionlint_ran && (shellcheck_ran || shell_scripts.is_empty()) {
        println!("workflows: actionlint ({} workflow files) and shellcheck ({} shell scripts) passed", workflows.len(), shell_scripts.len());
    } else if !actionlint_ran && !shellcheck_ran {
        return Err(String::from(
            "neither `actionlint` nor `shellcheck` is installed in PATH; cannot verify workflows/scripts (offline YAML check passed, both required when CI=true)",
        ));
    } else if !actionlint_ran {
        return Err(String::from(
            "`actionlint` is not installed in PATH while `shellcheck` ran; cannot skip workflow linting silently (install `actionlint`)",
        ));
    } else {
        return Err(String::from(
            "`shellcheck` is not installed in PATH while `actionlint` ran; cannot skip shell script linting silently (install `shellcheck`)",
        ));
    }
    Ok(())
}

/// Perform offline YAML syntax & structure checks so malformed workflows always fail even without `actionlint`.
pub fn check_workflows_offline_yaml(root: &Path, workflows: &[String]) -> Result<(), String> {
    for rel in workflows {
        check_yaml_file_offline(root, rel, true)?;
    }
    Ok(())
}

/// Perform offline YAML syntax checks on `.github/actions/**/*.{yml,yaml}` without requiring workflow keys.
pub fn check_action_files_offline_yaml(root: &Path, actions: &[String]) -> Result<(), String> {
    for rel in actions {
        check_yaml_file_offline(root, rel, false)?;
    }
    Ok(())
}

fn check_yaml_file_offline(root: &Path, rel: &str, require_workflow_keys: bool) -> Result<(), String> {
    let text = std::fs::read_to_string(root.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
    if text.trim().is_empty() {
        return Err(format!("{rel}: workflow YAML file is empty"));
    }
    let mut has_jobs = false;
    let mut has_on = false;
    let mut open_expr_line: Option<usize> = None;
    for (idx, line) in text.lines().enumerate() {
        let line_no = idx + 1;
        if line.trim_start_matches(' ').starts_with('\t') {
            return Err(format!("{rel}:{line_no}: YAML indentation cannot use tab characters"));
        }
        let trimmed = line.trim();
        if trimmed == "jobs:" || trimmed.starts_with("jobs:") {
            has_jobs = true;
        }
        if trimmed == "on:" || trimmed.starts_with("on:") || trimmed.starts_with("'on':") || trimmed.starts_with("\"on\":") {
            has_on = true;
        }
        scan_yaml_line_expressions(rel, line, line_no, &mut open_expr_line)?;
    }
    if let Some(open_line) = open_expr_line {
        return Err(format!("{rel}:{open_line}: unclosed GitHub Actions expression `${{{{`"));
    }
    if require_workflow_keys && (!has_on || !has_jobs) {
        return Err(format!("{rel}: workflow YAML must declare `on:` and `jobs:` keys"));
    }
    Ok(())
}

fn scan_yaml_line_expressions(rel: &str, line: &str, line_no: usize, open_expr_line: &mut Option<usize>) -> Result<(), String> {
    let mut rest = line;
    while !rest.is_empty() {
        if let Some(open_line) = *open_expr_line {
            if let Some((inside, after_close)) = rest.split_once("}}") {
                if inside.contains("${{") {
                    return Err(format!("{rel}:{open_line}: unclosed GitHub Actions expression `${{{{`"));
                }
                *open_expr_line = None;
                rest = after_close;
            } else {
                if rest.contains("${{") {
                    return Err(format!("{rel}:{open_line}: unclosed GitHub Actions expression `${{{{`"));
                }
                break;
            }
        } else if let Some((_, after_open)) = rest.split_once("${{") {
            *open_expr_line = Some(line_no);
            rest = after_open;
        } else {
            break;
        }
    }
    Ok(())
}

fn is_yaml_path(path: &Path) -> bool {
    has_ext_ci(path, &["yml", "yaml"])
}

/// Discover all `.yml` / `.yaml` files inside `.github/workflows` as slash-normalized relative paths.
pub fn collect_workflow_files(root: &Path) -> Result<Vec<String>, String> {
    let dir = root.join(".github/workflows");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out: Vec<String> =
        read_dir_paths_sorted(&dir)?.into_iter().filter(|p| is_regular_file(p) && is_yaml_path(p)).map(|p| rel_slash_lossy(root, &p)).collect();
    out.sort();
    Ok(out)
}

/// Discover all `.yml` / `.yaml` files inside `.github/actions` recursively as slash-normalized paths.
pub fn collect_action_files(root: &Path) -> Result<Vec<String>, String> {
    let dir = root.join(".github/actions");
    if !is_real_dir(&dir) {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    walk_rel_files(root, &dir, &|_| false, &is_yaml_path, &mut out)?;
    out.sort();
    Ok(out)
}

/// Discover all tracked `.sh` scripts and `.githooks/*` hooks (falling back to recursive filesystem scan outside git).
pub fn collect_shell_scripts(root: &Path) -> Result<Vec<String>, String> {
    let mut rels = if root.join(".git").exists() && has_tool("git") {
        crate::repo_files(root)?.into_iter().filter(|rel| is_shell_or_hook_rel(rel)).collect()
    } else {
        let mut all = Vec::new();
        walk_rel_files(
            root,
            root,
            &|p| {
                p.file_name().is_some_and(|n| {
                    let s = n.to_string_lossy();
                    matches!(s.as_ref(), "target" | "dist" | "corpus" | ".git") || (s.starts_with('.') && s != ".githooks" && s != ".github")
                })
            },
            &|p| is_shell_or_hook_rel(&rel_slash_lossy(root, p)),
            &mut all,
        )?;
        all
    };
    rels.sort();
    rels.dedup();
    Ok(rels)
}

fn is_shell_or_hook_rel(rel: &str) -> bool {
    if rel.to_ascii_lowercase().ends_with(".sh") {
        return true;
    }
    rel.strip_prefix(".githooks/").is_some_and(|rest| !rest.is_empty() && !rest.ends_with(".sample") && !rest.eq_ignore_ascii_case("README.md"))
}

fn has_tool(bin: &str) -> bool {
    Command::new(bin).arg("--version").output().is_ok_and(|o| o.status.success())
}

fn run_linter_if_installed(
    root: &Path,
    bin: &str,
    label: &str,
    pre_args: &[&str],
    files: &[String],
    in_ci: bool,
    require_lint: bool,
) -> Result<bool, String> {
    if !has_tool(bin) {
        if in_ci {
            return Err(format!("`{bin}` is not installed in PATH (required when CI=true)"));
        }
        if require_lint {
            return Err(format!("`{bin}` is not installed in PATH (required when {REQUIRE_WORKFLOW_LINT_ENV}=1)"));
        }
        println!("workflows: `{bin}` not found in PATH; skipping lint across {} file(s) (enforced when CI=true)", files.len());
        return Ok(false);
    }
    let out = Command::new(bin).current_dir(root).args(pre_args).args(files).output().map_err(|e| format!("{bin}: {e}"))?;
    if !out.status.success() {
        let combined = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        return Err(format!("`{label}` failed ({}):\n{}", out.status, combined.trim()));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::{collect_shell_scripts, collect_workflow_files, has_tool, run};
    use crate::scan_util::test_support::TestDir;

    #[test]
    fn discovers_workflows_and_shell_scripts_deterministically() {
        let dir = TestDir::new("wf-discover");
        for (rel, body) in [
            (".github/actionlint.yaml", "config-variables: null\n"),
            (".github/workflows/b.yaml", "name: B\n"),
            (".github/workflows/a.yml", "name: A\n"),
            (".github/workflows/notes.txt", "ignored\n"),
            ("packaging/macos/verify.sh", "#!/usr/bin/env bash\nset -euo pipefail\n"),
            (".githooks/pre-commit", "#!/usr/bin/env sh\nset -eu\n"),
        ] {
            dir.write(rel, body);
        }
        assert_eq!(collect_workflow_files(dir.path()).unwrap(), vec![".github/workflows/a.yml", ".github/workflows/b.yaml"]);
        assert_eq!(collect_shell_scripts(dir.path()).unwrap(), vec![".githooks/pre-commit", "packaging/macos/verify.sh"]);
    }

    #[test]
    fn rejects_unexpected_cli_arguments_and_missing_config() {
        let dir = TestDir::new("wf-bad-args");
        assert!(run(dir.path(), &[String::from("--bogus")]).unwrap_err().contains("usage: cargo xtask workflows"));
        assert!(run(dir.path(), &[]).unwrap_err().contains(".github/actionlint.yaml"));
    }

    #[test]
    fn parses_ci_env_truthiness_strictly() {
        assert!(!super::parse_ci_env_value(None));
        for falsy in ["", "false", "FALSE", "0", "no", "off"] {
            assert!(!super::parse_ci_env_value(Some(OsStr::new(falsy))), "{falsy}");
        }
        for truthy in ["true", "1"] {
            assert!(super::parse_ci_env_value(Some(OsStr::new(truthy))), "{truthy}");
        }
    }

    #[test]
    fn passes_on_valid_workflow_and_catches_invalid_workflow_when_actionlint_present() {
        let dir = TestDir::new("wf-lint");
        dir.write(".github/actionlint.yaml", "config-variables: null\n");
        dir.write(
            ".github/workflows/ci.yml",
            "name: CI\non:\n  push:\n    branches: [main]\npermissions:\n  contents: read\njobs:\n  check:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo ok\n",
        );

        if has_tool("actionlint") {
            run(dir.path(), &[]).unwrap();
            dir.write(
                ".github/workflows/broken.yml",
                "name: Broken\non: push\njobs:\n  bad:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v7\n        unknown_field: 123\n",
            );
            assert!(run(dir.path(), &[]).unwrap_err().contains("actionlint"));
        }

        // Offline YAML check catches malformed YAML, space-indented tabs, unclosed ${{, and invalid action YAML.
        dir.write(".github/workflows/ci.yml", "name: Broken\non:\n\tbad: true\njobs:\n  a:\n");
        let yaml_err = run(dir.path(), &[]).unwrap_err();
        assert!(yaml_err.contains("tab") || yaml_err.contains("actionlint"), "{yaml_err}");

        for (body, needle) in [
            ("name: IndentedTab\non:\n  \tbad: true\njobs:\n  a:\n", "tab"),
            ("name: Unclosed2\non: push\njobs:\n  a:\n    name: ${{ a }} ${{ b\n", "unclosed GitHub Actions expression"),
        ] {
            dir.write(".github/workflows/ci.yml", body);
            let err = super::check_workflows_offline_yaml(dir.path(), &[String::from(".github/workflows/ci.yml")]).unwrap_err();
            assert!(err.contains(needle), "{err}");
        }

        dir.write(".github/actions/setup/action.yml", "name: Setup\nruns:\n  using: composite\n  steps:\n    - run: echo ${{ a }} ${{ unclosed\n");
        let actions = super::collect_action_files(dir.path()).unwrap();
        assert_eq!(actions, vec![".github/actions/setup/action.yml"]);
        assert!(super::check_action_files_offline_yaml(dir.path(), &actions).unwrap_err().contains("unclosed GitHub Actions expression"));

        if has_tool("git") {
            let corrupt = TestDir::new("wf-corrupt-git");
            std::fs::create_dir_all(corrupt.join(".git")).unwrap();
            assert!(collect_shell_scripts(corrupt.path()).is_err(), "corrupt .git must propagate git ls-files failure");
        }
    }
}
