//! `cargo xtask workflows`: lint GitHub Actions workflows with `actionlint` (using
//! `.github/actionlint.yaml`) and tracked/hook shell scripts with `shellcheck` (`R0.6`).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

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

    let actionlint_ran = run_actionlint_if_installed(root, &workflows, in_ci, require_lint)?;
    let shellcheck_ran = run_shellcheck_if_installed(root, &shell_scripts, in_ci, require_lint)?;

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

/// Discover all `.yml` / `.yaml` files inside `.github/workflows` as slash-normalized relative paths.
pub fn collect_workflow_files(root: &Path) -> Result<Vec<String>, String> {
    let dir = root.join(".github").join("workflows");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        if !entry.file_type().is_ok_and(|ft| ft.is_file()) {
            continue;
        }
        let is_yaml =
            path.extension().and_then(|s| s.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("yml") || ext.eq_ignore_ascii_case("yaml"));
        if is_yaml {
            out.push(rel_slash(root, &path));
        }
    }
    out.sort();
    Ok(out)
}

/// Discover all `.yml` / `.yaml` files inside `.github/actions` recursively as slash-normalized paths.
pub fn collect_action_files(root: &Path) -> Result<Vec<String>, String> {
    let dir = root.join(".github").join("actions");
    if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir()) {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    collect_yaml_tree(root, &dir, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect_yaml_tree(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let ft = entry.file_type().map_err(|e| format!("{}: {e}", entry.path().display()))?;
        let path = entry.path();
        if ft.is_dir() {
            collect_yaml_tree(root, &path, out)?;
        } else if ft.is_file()
            && path.extension().and_then(|s| s.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("yml") || ext.eq_ignore_ascii_case("yaml"))
        {
            out.push(rel_slash(root, &path));
        }
    }
    Ok(())
}

/// Discover all tracked `.sh` scripts and `.githooks/*` hooks (falling back to recursive filesystem scan outside git).
pub fn collect_shell_scripts(root: &Path) -> Result<Vec<String>, String> {
    let mut rels = Vec::new();
    if root.join(".git").exists() && has_tool("git") {
        let tracked = crate::repo_files(root)?;
        for rel in tracked {
            if is_shell_or_hook_rel(&rel) {
                rels.push(rel);
            }
        }
        rels.sort();
        rels.dedup();
        return Ok(rels);
    }

    let mut files = Vec::new();
    walk_shell_targets(root, root, &mut files)?;
    rels.extend(files.iter().map(|p| rel_slash(root, p)).filter(|rel| is_shell_or_hook_rel(rel)));
    rels.sort();
    rels.dedup();
    Ok(rels)
}

fn is_shell_or_hook_rel(rel: &str) -> bool {
    if rel.to_ascii_lowercase().ends_with(".sh") {
        return true;
    }
    if let Some(rest) = rel.strip_prefix(".githooks/") {
        return !rest.is_empty() && !rest.ends_with(".sample") && !rest.eq_ignore_ascii_case("README.md");
    }
    false
}

fn walk_shell_targets(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let ft = entry.file_type().map_err(|e| format!("{}: {e}", path.display()))?;
        if ft.is_dir() {
            if matches!(name_str.as_ref(), "target" | "dist" | "corpus" | ".git")
                || (name_str.starts_with('.') && name_str != ".githooks" && name_str != ".github")
            {
                continue;
            }
            walk_shell_targets(root, &path, out)?;
        } else if ft.is_file() {
            let rel = rel_slash(root, &path);
            if is_shell_or_hook_rel(&rel) {
                out.push(path);
            }
        }
    }
    Ok(())
}

fn rel_slash(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

fn has_tool(bin: &str) -> bool {
    Command::new(bin).arg("--version").output().is_ok_and(|o| o.status.success())
}

fn run_actionlint_if_installed(root: &Path, workflows: &[String], in_ci: bool, require_lint: bool) -> Result<bool, String> {
    if !has_tool("actionlint") {
        if in_ci {
            return Err(String::from("`actionlint` is not installed in PATH (required when CI=true)"));
        }
        if require_lint {
            return Err(String::from("`actionlint` is not installed in PATH (required when CURVEBOOK_REQUIRE_WORKFLOW_LINT=1)"));
        }
        println!("workflows: `actionlint` not found in PATH; skipping live workflow lint across {} file(s) (enforced when CI=true)", workflows.len());
        return Ok(false);
    }

    let mut cmd = Command::new("actionlint");
    cmd.current_dir(root).args(["-config-file", ACTIONLINT_CONFIG]).args(workflows);
    let out = cmd.output().map_err(|e| format!("actionlint: {e}"))?;
    if !out.status.success() {
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let combined = format!("{stdout}{stderr}");
        return Err(format!("`actionlint` failed ({}):\n{}", out.status, combined.trim()));
    }
    Ok(true)
}

fn run_shellcheck_if_installed(root: &Path, scripts: &[String], in_ci: bool, require_lint: bool) -> Result<bool, String> {
    if scripts.is_empty() {
        return Ok(false);
    }
    if !has_tool("shellcheck") {
        if in_ci {
            return Err(String::from("`shellcheck` is not installed in PATH (required when CI=true)"));
        }
        if require_lint {
            return Err(String::from("`shellcheck` is not installed in PATH (required when CURVEBOOK_REQUIRE_WORKFLOW_LINT=1)"));
        }
        println!("workflows: `shellcheck` not found in PATH; skipping shell script lint across {} file(s) (enforced when CI=true)", scripts.len());
        return Ok(false);
    }

    let mut cmd = Command::new("shellcheck");
    cmd.current_dir(root).arg("-x").args(scripts);
    let out = cmd.output().map_err(|e| format!("shellcheck: {e}"))?;
    if !out.status.success() {
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let combined = format!("{stdout}{stderr}");
        return Err(format!("`shellcheck -x` failed ({}):\n{}", out.status, combined.trim()));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{collect_shell_scripts, collect_workflow_files, has_tool, run};

    fn make_temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("cb-xtask-wf-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn discovers_workflows_and_shell_scripts_deterministically() {
        let dir = make_temp_dir("discover");
        std::fs::create_dir_all(dir.join(".github/workflows")).unwrap();
        std::fs::create_dir_all(dir.join("packaging/macos")).unwrap();
        std::fs::create_dir_all(dir.join(".githooks")).unwrap();

        std::fs::write(dir.join(".github/actionlint.yaml"), "config-variables: null\n").unwrap();
        std::fs::write(dir.join(".github/workflows/b.yaml"), "name: B\n").unwrap();
        std::fs::write(dir.join(".github/workflows/a.yml"), "name: A\n").unwrap();
        std::fs::write(dir.join(".github/workflows/notes.txt"), "ignored\n").unwrap();
        std::fs::write(dir.join("packaging/macos/verify.sh"), "#!/usr/bin/env bash\nset -euo pipefail\n").unwrap();
        std::fs::write(dir.join(".githooks/pre-commit"), "#!/usr/bin/env sh\nset -eu\n").unwrap();

        let wfs = collect_workflow_files(&dir).unwrap();
        assert_eq!(wfs, vec![".github/workflows/a.yml", ".github/workflows/b.yaml"]);

        let scripts = collect_shell_scripts(&dir).unwrap();
        assert_eq!(scripts, vec![".githooks/pre-commit", "packaging/macos/verify.sh"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_unexpected_cli_arguments_and_missing_config() {
        let dir = make_temp_dir("bad-args");
        let err = run(&dir, &[String::from("--bogus")]).unwrap_err();
        assert!(err.contains("usage: cargo xtask workflows"));

        let missing_cfg = run(&dir, &[]).unwrap_err();
        assert!(missing_cfg.contains(".github/actionlint.yaml"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_ci_env_truthiness_strictly() {
        use std::ffi::OsStr;

        assert!(!super::parse_ci_env_value(None));
        assert!(!super::parse_ci_env_value(Some(OsStr::new(""))));
        assert!(!super::parse_ci_env_value(Some(OsStr::new("false"))));
        assert!(!super::parse_ci_env_value(Some(OsStr::new("FALSE"))));
        assert!(!super::parse_ci_env_value(Some(OsStr::new("0"))));
        assert!(!super::parse_ci_env_value(Some(OsStr::new("no"))));
        assert!(!super::parse_ci_env_value(Some(OsStr::new("off"))));
        assert!(super::parse_ci_env_value(Some(OsStr::new("true"))));
        assert!(super::parse_ci_env_value(Some(OsStr::new("1"))));
    }

    #[test]
    fn passes_on_valid_workflow_and_catches_invalid_workflow_when_actionlint_present() {
        let dir = make_temp_dir("lint");
        std::fs::create_dir_all(dir.join(".github/workflows")).unwrap();
        std::fs::write(dir.join(".github/actionlint.yaml"), "config-variables: null\n").unwrap();
        std::fs::write(
            dir.join(".github/workflows/ci.yml"),
            "name: CI\non:\n  push:\n    branches: [main]\npermissions:\n  contents: read\njobs:\n  check:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo ok\n",
        )
        .unwrap();

        if has_tool("actionlint") {
            run(&dir, &[]).unwrap();

            std::fs::write(
                dir.join(".github/workflows/broken.yml"),
                "name: Broken\non: push\njobs:\n  bad:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v7\n        unknown_field: 123\n",
            )
            .unwrap();
            let err = run(&dir, &[]).unwrap_err();
            assert!(err.contains("actionlint"));
        }

        // Offline YAML check catches malformed YAML even without actionlint.
        std::fs::write(dir.join(".github/workflows/ci.yml"), "name: Broken\non:\n\tbad: true\njobs:\n  a:\n").unwrap();
        let yaml_err = run(&dir, &[]).unwrap_err();
        assert!(yaml_err.contains("tab") || yaml_err.contains("actionlint"), "{yaml_err}");

        // Offline YAML check also catches space-indented tabs, second unclosed ${{, and invalid action YAML.
        std::fs::write(dir.join(".github/workflows/ci.yml"), "name: IndentedTab\non:\n  \tbad: true\njobs:\n  a:\n").unwrap();
        let tab_err = super::check_workflows_offline_yaml(&dir, &[String::from(".github/workflows/ci.yml")]).unwrap_err();
        assert!(tab_err.contains("tab"), "{tab_err}");

        std::fs::write(dir.join(".github/workflows/ci.yml"), "name: Unclosed2\non: push\njobs:\n  a:\n    name: ${{ a }} ${{ b\n").unwrap();
        let expr_err = super::check_workflows_offline_yaml(&dir, &[String::from(".github/workflows/ci.yml")]).unwrap_err();
        assert!(expr_err.contains("unclosed GitHub Actions expression"), "{expr_err}");

        std::fs::create_dir_all(dir.join(".github/actions/setup")).unwrap();
        std::fs::write(
            dir.join(".github/actions/setup/action.yml"),
            "name: Setup\nruns:\n  using: composite\n  steps:\n    - run: echo ${{ a }} ${{ unclosed\n",
        )
        .unwrap();
        let actions = super::collect_action_files(&dir).unwrap();
        assert_eq!(actions, vec![".github/actions/setup/action.yml"]);
        let act_err = super::check_action_files_offline_yaml(&dir, &actions).unwrap_err();
        assert!(act_err.contains("unclosed GitHub Actions expression"), "{act_err}");

        if has_tool("git") {
            let corrupt = make_temp_dir("corrupt-git");
            std::fs::create_dir_all(corrupt.join(".git")).unwrap();
            assert!(collect_shell_scripts(&corrupt).is_err(), "corrupt .git must propagate git ls-files failure");
            let _ = std::fs::remove_dir_all(&corrupt);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
