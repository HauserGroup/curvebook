//! `cargo xtask workflows`: lint GitHub Actions workflows with `actionlint` (using
//! `.github/actionlint.yaml`) and tracked/hook shell scripts with `shellcheck` (`R0.6`).

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use crate::scan_util::{collect_files_no_symlinks, has_ext_ci, read_dir_paths_sorted, rel_slash_lossy, walk_rel_files};

const ACTIONLINT_CONFIG: &str = ".github/actionlint.yaml";
const REQUIRE_WORKFLOW_LINT_ENV: &str = "CURVEBOOK_REQUIRE_WORKFLOW_LINT";

#[must_use]
pub fn parse_ci_env_value(raw: Option<&OsStr>) -> bool {
    raw.is_some_and(|v| !matches!(v.to_string_lossy().trim().to_ascii_lowercase().as_str(), "" | "0" | "false" | "no" | "off"))
}

#[must_use]
pub(crate) fn is_ci_env() -> bool {
    parse_ci_env_value(std::env::var_os("CI").as_deref())
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("usage: cargo xtask workflows (unexpected args: `{}`)", args.join(" ")));
    }
    if !root.join(ACTIONLINT_CONFIG).is_file() {
        return Err(format!("missing `{ACTIONLINT_CONFIG}` in {}", root.display()));
    }
    let workflows = collect_workflow_files(root)?;
    if workflows.is_empty() {
        return Err(format!("no `.yml`/`.yaml` workflows found in `{}/.github/workflows`", root.display()));
    }
    check_workflows_offline_yaml(root, &workflows)?;
    check_action_files_offline_yaml(root, &collect_action_files(root)?)?;

    let shell_scripts = collect_shell_scripts(root)?;
    let in_ci = is_ci_env();
    let require_lint = in_ci || parse_ci_env_value(std::env::var_os(REQUIRE_WORKFLOW_LINT_ENV).as_deref());
    let actionlint_ran =
        run_linter_if_installed(root, "actionlint", "actionlint", &["-config-file", ACTIONLINT_CONFIG], &workflows, in_ci, require_lint)?;
    let shellcheck_ran =
        !shell_scripts.is_empty() && run_linter_if_installed(root, "shellcheck", "shellcheck -x", &["-x"], &shell_scripts, in_ci, require_lint)?;
    report_lint_outcome(actionlint_ran, shellcheck_ran, workflows.len(), shell_scripts.len())
}

fn report_lint_outcome(actionlint_ran: bool, shellcheck_ran: bool, workflow_count: usize, shell_script_count: usize) -> Result<(), String> {
    let shellcheck_ok = shellcheck_ran || shell_script_count == 0;
    if actionlint_ran && shellcheck_ok {
        println!("workflows: actionlint ({workflow_count} workflow files) and shellcheck ({shell_script_count} shell scripts) passed");
        Ok(())
    } else if !actionlint_ran && !shellcheck_ok {
        Err(String::from(
            "neither `actionlint` nor `shellcheck` is installed in PATH; cannot verify workflows/scripts (offline YAML check passed, both required when CI=true)",
        ))
    } else if !actionlint_ran {
        Err(String::from("`actionlint` is not installed in PATH; cannot skip workflow linting silently (install `actionlint`)"))
    } else {
        Err(String::from(
            "`shellcheck` is not installed in PATH while `actionlint` ran; cannot skip shell script linting silently (install `shellcheck`)",
        ))
    }
}

pub fn check_workflows_offline_yaml(root: &Path, workflows: &[String]) -> Result<(), String> {
    workflows.iter().try_for_each(|rel| check_yaml_file_offline(root, rel, true))
}

pub fn check_action_files_offline_yaml(root: &Path, actions: &[String]) -> Result<(), String> {
    actions.iter().try_for_each(|rel| check_yaml_file_offline(root, rel, false))
}

fn check_yaml_file_offline(root: &Path, rel: &str, require_workflow_keys: bool) -> Result<(), String> {
    let text = std::fs::read_to_string(root.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
    if text.trim().is_empty() {
        return Err(format!("{rel}: workflow YAML file is empty"));
    }
    let (mut has_jobs, mut has_on, mut open_expr_line) = (false, false, None::<usize>);
    for (idx, line) in text.lines().enumerate() {
        let line_no = idx + 1;
        if line.trim_start_matches(' ').starts_with('\t') {
            return Err(format!("{rel}:{line_no}: YAML indentation cannot use tab characters"));
        }
        let t = line.trim();
        has_jobs |= t.starts_with("jobs:");
        has_on |= t.starts_with("on:") || t.starts_with("'on':") || t.starts_with("\"on\":");
        let mut rest = line;
        while !rest.is_empty() {
            if let Some(open_line) = open_expr_line {
                if let Some((inside, after)) = rest.split_once("}}") {
                    if inside.contains("${{") {
                        return Err(format!("{rel}:{open_line}: unclosed GitHub Actions expression `${{{{`"));
                    }
                    open_expr_line = None;
                    rest = after;
                } else {
                    if rest.contains("${{") {
                        return Err(format!("{rel}:{open_line}: unclosed GitHub Actions expression `${{{{`"));
                    }
                    break;
                }
            } else if let Some((_, after)) = rest.split_once("${{") {
                open_expr_line = Some(line_no);
                rest = after;
            } else {
                break;
            }
        }
    }
    if let Some(open_line) = open_expr_line {
        return Err(format!("{rel}:{open_line}: unclosed GitHub Actions expression `${{{{`"));
    }
    if require_workflow_keys && (!has_on || !has_jobs) {
        return Err(format!("{rel}: workflow YAML must declare `on:` and `jobs:` keys"));
    }
    Ok(())
}

fn checked_github_subdir(root: &Path, rel: &str) -> Result<Option<std::path::PathBuf>, String> {
    let dir = root.join(rel);
    match std::fs::symlink_metadata(&dir) {
        Ok(m) if m.file_type().is_symlink() => Err(format!("{rel}: symlink directory is forbidden")),
        Ok(m) if m.is_dir() => Ok(Some(dir)),
        Ok(_) => Err(format!("{rel}: expected directory")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("stat {rel}: {e}")),
    }
}

pub fn collect_workflow_files(root: &Path) -> Result<Vec<String>, String> {
    let Some(dir) = checked_github_subdir(root, ".github/workflows")? else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for path in read_dir_paths_sorted(&dir)? {
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("stat {}: {e}", path.display()))?;
        if meta.file_type().is_symlink() {
            return Err(format!("{}: symlink is forbidden inside `.github/workflows`", rel_slash_lossy(root, &path)));
        }
        if meta.is_file() && has_ext_ci(&path, &["yml", "yaml"]) {
            out.push(rel_slash_lossy(root, &path));
        }
    }
    out.sort();
    Ok(out)
}

pub fn collect_action_files(root: &Path) -> Result<Vec<String>, String> {
    let Some(dir) = checked_github_subdir(root, ".github/actions")? else { return Ok(Vec::new()) };
    let mut all = Vec::new();
    collect_files_no_symlinks(root, &dir, "symlink is forbidden inside `.github/actions`", &mut all)?;
    all.retain(|p| has_ext_ci(Path::new(p), &["yml", "yaml"]));
    all.sort();
    Ok(all)
}

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
    rel.to_ascii_lowercase().ends_with(".sh")
        || rel.strip_prefix(".githooks/").is_some_and(|r| !r.is_empty() && !r.ends_with(".sample") && !r.eq_ignore_ascii_case("README.md"))
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
        return Err(format!(
            "`{label}` failed ({}):\n{}",
            out.status,
            format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)).trim()
        ));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;
    use crate::scan_util::test_support::TestDir;

    #[test]
    fn discovery_ci_env_yaml_checks_and_symlinks() {
        assert!(!parse_ci_env_value(None));
        for f in ["", "false", "FALSE", "0", "no", "off"] {
            assert!(!parse_ci_env_value(Some(OsStr::new(f))), "{f}");
        }
        for t in ["true", "1"] {
            assert!(parse_ci_env_value(Some(OsStr::new(t))), "{t}");
        }

        let dir = TestDir::new("wf-all");
        assert!(run(dir.path(), &[String::from("--bogus")]).unwrap_err().contains("usage: cargo xtask workflows"));
        assert!(run(dir.path(), &[]).unwrap_err().contains(".github/actionlint.yaml"));
        for (rel, body) in [
            (".github/actionlint.yaml", "config-variables: null\n"),
            (".github/workflows/b.yaml", "name: B\non: push\njobs:\n  j:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo b\n"),
            (
                ".github/workflows/a.yml",
                "name: A\non:\n  push:\n    branches: [main]\npermissions:\n  contents: read\njobs:\n  check:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo ok\n",
            ),
            (".github/workflows/notes.txt", "ignored\n"),
            ("packaging/macos/verify.sh", "#!/usr/bin/env bash\nset -euo pipefail\n"),
            (".githooks/pre-commit", "#!/usr/bin/env sh\nset -eu\n"),
        ] {
            dir.write(rel, body);
        }
        assert_eq!(collect_workflow_files(dir.path()).unwrap(), vec![".github/workflows/a.yml", ".github/workflows/b.yaml"]);
        assert_eq!(collect_shell_scripts(dir.path()).unwrap(), vec![".githooks/pre-commit", "packaging/macos/verify.sh"]);
        for (body, needle) in [
            ("name: IndentedTab\non:\n  \tbad: true\njobs:\n  a:\n", "tab"),
            ("name: Unclosed2\non: push\njobs:\n  a:\n    name: ${{ a }} ${{ b\n", "unclosed GitHub Actions expression"),
        ] {
            dir.write(".github/workflows/a.yml", body);
            assert!(check_workflows_offline_yaml(dir.path(), &[String::from(".github/workflows/a.yml")]).unwrap_err().contains(needle));
        }
        dir.write(".github/actions/setup/action.yml", "name: Setup\nruns:\n  using: composite\n  steps:\n    - run: echo ${{ a }} ${{ unclosed\n");
        let actions = collect_action_files(dir.path()).unwrap();
        assert_eq!(actions, vec![".github/actions/setup/action.yml"]);
        assert!(check_action_files_offline_yaml(dir.path(), &actions).unwrap_err().contains("unclosed GitHub Actions expression"));

        assert!(report_lint_outcome(false, false, 1, 0).unwrap_err().contains("actionlint"));
        assert!(report_lint_outcome(false, false, 1, 2).unwrap_err().contains("neither `actionlint` nor `shellcheck`"));

        if has_tool("git") {
            let corrupt = TestDir::new("wf-corrupt-git");
            std::fs::create_dir_all(corrupt.join(".git")).unwrap();
            assert!(collect_shell_scripts(corrupt.path()).is_err());
        }
        #[cfg(unix)]
        {
            let s = TestDir::new("wf-symlink");
            s.write("outside.yml", "name: O\non: push\njobs: {}\n");
            std::fs::create_dir_all(s.join(".github/workflows")).unwrap();
            std::os::unix::fs::symlink(s.join("outside.yml"), s.join(".github/workflows/link.yml")).unwrap();
            assert!(collect_workflow_files(s.path()).unwrap_err().contains("symlink"));
            std::fs::create_dir_all(s.join(".github/actions/setup")).unwrap();
            std::os::unix::fs::symlink(s.join("outside.yml"), s.join(".github/actions/setup/action.yml")).unwrap();
            assert!(collect_action_files(s.path()).unwrap_err().contains("symlink"));
        }
    }
}
