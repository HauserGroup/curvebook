//! `cargo xtask workflows`: lint GitHub Actions workflows with `actionlint` (using
//! `.github/actionlint.yaml`) and packaging/hook shell scripts with `shellcheck` (`R0.6`).

use std::path::{Path, PathBuf};
use std::process::Command;

const ACTIONLINT_CONFIG: &str = ".github/actionlint.yaml";

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

    let shell_scripts = collect_shell_scripts(root)?;
    let in_ci = std::env::var_os("CI").is_some_and(|v| !v.is_empty());

    let actionlint_ran = run_actionlint_if_installed(root, &workflows, in_ci)?;
    let shellcheck_ran = run_shellcheck_if_installed(root, &shell_scripts, in_ci)?;

    if actionlint_ran && shellcheck_ran {
        println!("workflows: actionlint ({} workflow files) and shellcheck ({} shell scripts) passed", workflows.len(), shell_scripts.len());
    } else if actionlint_ran {
        println!("workflows: actionlint ({} workflow files) passed", workflows.len());
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
        if !path.is_file() {
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

/// Discover all `.sh` scripts under `packaging/` plus `.githooks/pre-commit` when present.
pub fn collect_shell_scripts(root: &Path) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    let packaging_dir = root.join("packaging");
    if packaging_dir.is_dir() {
        walk_sh_files(&packaging_dir, &mut files)?;
    }
    let pre_commit = root.join(".githooks").join("pre-commit");
    if pre_commit.is_file() {
        files.push(pre_commit);
    }
    let mut rels: Vec<String> = files.iter().map(|p| rel_slash(root, p)).collect();
    rels.sort();
    rels.dedup();
    Ok(rels)
}

fn walk_sh_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        let ft = entry.file_type().map_err(|e| format!("{}: {e}", path.display()))?;
        if ft.is_dir() {
            walk_sh_files(&path, out)?;
        } else if ft.is_file() && path.extension().and_then(|s| s.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("sh")) {
            out.push(path);
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

fn run_actionlint_if_installed(root: &Path, workflows: &[String], in_ci: bool) -> Result<bool, String> {
    if !has_tool("actionlint") {
        if in_ci {
            return Err(String::from("`actionlint` is not installed in PATH (required when CI=true)"));
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

fn run_shellcheck_if_installed(root: &Path, scripts: &[String], in_ci: bool) -> Result<bool, String> {
    if scripts.is_empty() {
        return Ok(false);
    }
    if !has_tool("shellcheck") {
        if in_ci {
            return Err(String::from("`shellcheck` is not installed in PATH (required when CI=true)"));
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
        let dir = std::env::temp_dir().join(format!("lc-xtask-wf-{tag}-{}-{nanos}", std::process::id()));
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
    fn passes_on_valid_workflow_and_catches_invalid_workflow_when_actionlint_present() {
        let dir = make_temp_dir("lint");
        std::fs::create_dir_all(dir.join(".github/workflows")).unwrap();
        std::fs::write(dir.join(".github/actionlint.yaml"), "config-variables: null\n").unwrap();
        std::fs::write(
            dir.join(".github/workflows/ci.yml"),
            "name: CI\non:\n  push:\n    branches: [main]\npermissions:\n  contents: read\njobs:\n  check:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo ok\n",
        )
        .unwrap();

        run(&dir, &[]).unwrap();

        if has_tool("actionlint") {
            std::fs::write(
                dir.join(".github/workflows/broken.yml"),
                "name: Broken\non: push\njobs:\n  bad:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v7\n        unknown_field: 123\n",
            )
            .unwrap();
            let err = run(&dir, &[]).unwrap_err();
            assert!(err.contains("actionlint"));
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
