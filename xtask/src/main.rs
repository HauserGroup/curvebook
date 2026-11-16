//! Workspace quality gates, packaging helpers, and architectural checks for LumenCraft.
#![forbid(unsafe_code)]

mod assets;
mod brands;
mod bundle;
mod cleanroom;
mod commands;
mod contributors;
mod corpus;
mod coverage;
mod cursors;
mod ico;
mod json;
mod layers;
mod licenses;
mod pending;
mod schemas;
mod screenshots;
mod secrets;
mod stats;
mod status;
mod telemetry;
mod ui_commands;
mod version;
mod web_fonts;
mod workflows;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const HELP: &str = "\
usage: cargo xtask <command> [args...]

commands:
  ci                    run full local/CI gate (fmt, clippy, tests, doctests, feature checks, policy gates, wasm)
  r10-gate              run R10.2 anti-phantom parity gate & R10.22 frame/latency budgets
  test [args...]        run workspace test suite (uses cargo-nextest when installed, else cargo test)
  layers                verify L0..L7 dependency rules, UI-crate isolation, and source hygiene
  secrets [--history]   scan repo (or git history additions) for API tokens, private keys, and local paths
  pending [--count]     list or count open ROADMAP.md checklist items with R<m>.<n> section IDs
  status                verify STATUS.md and ROADMAP.md tables have unique first-column keys
  licenses [--html [out]] verify deny policy/lockfile sources offline and optionally emit HTML report
  telemetry             verify zero-telemetry policy across crates, Rust sources, workflows, and web headers
  workflows             lint GitHub Actions workflows (actionlint) and shell scripts (shellcheck)
  screenshots [--check] generate (or verify) clean-room 1440x900 README workspace PNGs in docs/images/
  assets                verify all non-code assets are attributed in ASSETS.md
  brands                verify user-facing strings and packaging contain no competitor trademarks
  cleanroom             verify no vendor copyright headers or vendor-created fixtures exist
  cursors               verify Cursor variants match crates/tools/src/cursors.toml
  i18n-report           report untranslated locale keys in lumencraft-ui-egui
  ui-commands [--check] sync (or verify) crates/mcp/src/ui_catalog.rs from crates/ui-egui
  commands [--check]    regenerate (or verify) docs/commands.md from lumencraft-engine
  schemas [--check]     regenerate (or verify) docs/schemas/*.schema.json from lumencraft-format
  wasm                  check wasm32-unknown-unknown compilation across web-targeted crates
  corpus [--fetch]      run offline corpus & NIST StRD suites (optionally fetch corpus/nist-strd)
  coverage [-p <crate>] write a local cargo-llvm-cov lcov + HTML report (diagnostic; no threshold)
  bundle                build release binaries and assemble dist/LumenCraft.app
  ico <out> <png...>    pack PNG frames (<=256px) into a Windows .ico container
  version [set <ver>]   print or update [workspace.package] version
  stats [--exact]       print line and test counts across workspace crates
  contributors          refresh contributors/contributors.json from git history and people.toml
  web-fonts             stage optional LUMENCRAFT_FONTS_DIR faces into dist/web/fonts
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("xtask error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(args: &[String]) -> Result<(), String> {
    let root = workspace_root()?;
    let Some((cmd, rest)) = args.split_first() else {
        print!("{HELP}");
        return Ok(());
    };
    match cmd.as_str() {
        "-h" | "--help" | "help" => {
            print!("{HELP}");
            Ok(())
        }
        "ci" => cmd_ci(&root),
        "r10-gate" => cmd_r10_gate(&root),
        "test" => cmd_test(&root, rest),
        "layers" => cmd_layers(&root),
        "secrets" => secrets::run_cli(&root, rest),
        "pending" => pending::run(&root, rest),
        "status" => status::run(&root),
        "licenses" => licenses::run(&root, rest),
        "telemetry" => telemetry::run(&root, rest),
        "workflows" => workflows::run(&root, rest),
        "screenshots" => screenshots::run(&root, rest),
        "assets" => assets::run(&root),
        "brands" => brands::run(&root),
        "cleanroom" => cleanroom::run(&root),
        "cursors" => cursors::run(&root),
        "i18n-report" => cmd_i18n_report(&root),
        "ui-commands" => ui_commands::run(&root, rest.iter().any(|a| a == "--check")),
        "commands" => commands::run_cli(&root, rest),
        "schemas" => schemas::run_cli(&root, rest),
        "wasm" => cmd_wasm(&root),
        "corpus" => corpus::run(&root, rest),
        "coverage" => coverage::run(&root, rest),
        "bundle" => bundle::run(&root),
        "ico" => ico::run(rest),
        "version" => version::run(&root, rest),
        "stats" => stats::run(&root, rest.iter().any(|a| a == "--exact")),
        "contributors" => contributors::run(&root),
        "web-fonts" => web_fonts::run(&root),
        other => Err(format!("unknown xtask command `{other}`\n\n{HELP}")),
    }
}

pub(crate) fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| String::from("xtask manifest has no parent workspace directory"))
}

pub(crate) fn repo_files(root: &Path) -> Result<Vec<String>, String> {
    let listed = git_ls_files_nul(root, &["-c", "core.quotePath=false", "ls-files", "-z", "--cached", "--others", "--exclude-standard"])?;
    let deleted: std::collections::HashSet<String> =
        git_ls_files_nul(root, &["-c", "core.quotePath=false", "ls-files", "-z", "--deleted"])?.into_iter().collect();

    let mut files: Vec<String> = listed.into_iter().filter(|p| !deleted.contains(p)).collect();
    files.sort();
    files.dedup();

    for path in &files {
        let full = root.join(path);
        let meta = std::fs::symlink_metadata(&full).map_err(|e| format!("repo file `{path}` is not accessible: {e}"))?;
        if !meta.file_type().is_file() {
            return Err(format!("repo path `{path}` is not a regular file"));
        }
        std::fs::File::open(&full).map_err(|e| format!("repo file `{path}` cannot be opened for reading: {e}"))?;
    }
    Ok(files)
}

fn git_ls_files_nul(root: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let out = Command::new("git").current_dir(root).args(args).output().map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr)));
    }
    let mut paths = Vec::new();
    for raw in out.stdout.split(|&b| b == 0) {
        if raw.is_empty() {
            continue;
        }
        let path = std::str::from_utf8(raw).map_err(|e| format!("git ls-files utf-8: {e}"))?;
        paths.push(path.to_owned());
    }
    Ok(paths)
}

fn run_cargo_metadata(root: &Path, args: &[&str]) -> Result<json::Json, String> {
    let out = Command::new("cargo").current_dir(root).args(args).output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let text = String::from_utf8(out.stdout).map_err(|e| format!("cargo metadata utf-8: {e}"))?;
    json::parse(&text)
}

pub(crate) fn cargo_metadata(root: &Path) -> Result<json::Json, String> {
    run_cargo_metadata(root, &["metadata", "--format-version", "1", "--no-deps"])
}

pub(crate) fn cargo_metadata_resolved(root: &Path) -> Result<json::Json, String> {
    run_cargo_metadata(root, &["metadata", "--format-version", "1"])
}

fn cmd_layers(root: &Path) -> Result<(), String> {
    let meta = cargo_metadata_resolved(root)?;
    let graph = layers::from_metadata(&meta)?;
    let mut violations = layers::check(&graph);
    violations.extend(layers::check_manifest_patches(root));
    let hygiene = layers::check_source_hygiene(root);
    if violations.is_empty() && hygiene.is_empty() {
        println!("layers: all {} workspace packages satisfy L0..L7 dependency rules and source hygiene", graph.len());
        return Ok(());
    }
    let mut lines: Vec<String> = violations.iter().map(ToString::to_string).collect();
    lines.extend(hygiene.iter().map(ToString::to_string));
    Err(format!("{} layer/hygiene violation(s):\n  {}", lines.len(), lines.join("\n  ")))
}

fn cmd_test(root: &Path, extra: &[String]) -> Result<(), String> {
    if has_cargo_nextest() {
        let mut args = vec![String::from("nextest"), String::from("run"), String::from("--workspace")];
        args.extend_from_slice(extra);
        run_cargo(root, &args)
    } else {
        let mut args = vec![String::from("test"), String::from("--workspace"), String::from("--all-targets")];
        args.extend_from_slice(extra);
        run_cargo(root, &args)
    }
}

fn cmd_i18n_report(root: &Path) -> Result<(), String> {
    const TEST_PATH: &str = "i18n::tests::untranslated_report";
    let args = ["test", "-p", "lumencraft-ui-egui", "--lib", TEST_PATH, "--", "--exact", "--nocapture"];
    println!("+ cargo {}", args.join(" "));
    let out = Command::new("cargo").current_dir(root).args(args).output().map_err(|e| format!("cargo i18n-report: {e}"))?;
    let _ = std::io::stdout().write_all(&out.stdout);
    let _ = std::io::stderr().write_all(&out.stderr);
    if !out.status.success() {
        return Err(format!("cargo i18n-report failed with {}", out.status));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    verify_exact_single_test_passed(&stdout, TEST_PATH)
}

fn verify_exact_single_test_passed(stdout: &str, test_name: &str) -> Result<(), String> {
    let summary = stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix("test result: ok."))
        .ok_or_else(|| format!("i18n-report: expected `test result: ok.` summary for `{test_name}`"))?;

    let mut passed: Option<usize> = None;
    let mut failed: Option<usize> = None;
    let mut ignored: Option<usize> = None;
    for part in summary.split(';') {
        let p = part.trim();
        if let Some(num) = p.strip_suffix(" passed") {
            passed = num.trim().parse::<usize>().ok();
        } else if let Some(num) = p.strip_suffix(" failed") {
            failed = num.trim().parse::<usize>().ok();
        } else if let Some(num) = p.strip_suffix(" ignored") {
            ignored = num.trim().parse::<usize>().ok();
        }
    }

    if passed != Some(1) || failed != Some(0) || ignored != Some(0) {
        return Err(format!(
            "i18n-report: expected exact single passing test for `{test_name}` (1 passed; 0 failed; 0 ignored), got `{}`",
            summary.trim()
        ));
    }

    let test_prefix = format!("test {test_name} ...");
    if !stdout.lines().any(|line| line.trim_start().starts_with(&test_prefix)) {
        return Err(format!("i18n-report: output did not execute `{test_name}`"));
    }

    Ok(())
}

fn cmd_wasm(root: &Path) -> Result<(), String> {
    run_cargo_strs(root, &["check", "--target", "wasm32-unknown-unknown", "-p", "lumencraft-web", "-p", "lumencraft-engine", "--keep-going"])
}

fn cmd_r10_gate(root: &Path) -> Result<(), String> {
    run_cargo_strs(root, &["test", "-p", "lumencraft-ui-egui", "--test", "r10_core_and_spike"])
}

fn cmd_ci(root: &Path) -> Result<(), String> {
    run_cargo_strs(root, &["fmt", "--all", "--", "--check"])?;
    run_cargo_strs(root, &["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])?;
    cmd_test(root, &[])?;
    cmd_r10_gate(root)?;
    run_cargo_strs(root, &["test", "--workspace", "--doc"])?;

    // Verify embedders can build and test text/ui/mcp without bundled fonts.
    run_cargo_strs(root, &["test", "-p", "lumencraft-text", "--no-default-features"])?;
    run_cargo_strs(root, &["test", "-p", "lumencraft-ui-egui", "--no-default-features"])?;
    run_cargo_strs(root, &["check", "-p", "lumencraft-mcp", "--no-default-features"])?;

    assets::run(root)?;
    brands::run(root)?;
    cleanroom::run(root)?;
    cursors::run(root)?;
    cmd_i18n_report(root)?;
    ui_commands::run(root, true)?;
    commands::run(root, true)?;
    schemas::run(root, true)?;
    cmd_layers(root)?;
    secrets::run(root)?;
    licenses::run(root, &[])?;
    telemetry::run(root, &[])?;
    workflows::run(root, &[])?;
    screenshots::run(root, &[String::from("--check")])?;
    pending::run_count(root)?;
    status::run(root)?;

    let in_ci = workflows::is_ci_env();
    if in_ci || wasm_target_installed() {
        cmd_wasm(root)?;
    } else {
        println!("wasm: wasm32-unknown-unknown target sysroot not installed locally; skipping (enforced when CI=true)");
    }
    println!("ci: all checks passed");
    Ok(())
}

fn run_cargo_strs(root: &Path, args: &[&str]) -> Result<(), String> {
    let owned: Vec<String> = args.iter().map(|&s| s.to_owned()).collect();
    run_cargo(root, &owned)
}

fn run_cargo(root: &Path, args: &[String]) -> Result<(), String> {
    println!("+ cargo {}", args.join(" "));
    let status = Command::new("cargo").current_dir(root).args(args).status().map_err(|e| format!("cargo {}: {e}", args.join(" ")))?;
    if !status.success() {
        return Err(format!("`cargo {}` failed with {status}", args.join(" ")));
    }
    Ok(())
}

fn has_cargo_nextest() -> bool {
    Command::new("cargo").args(["nextest", "--version"]).output().is_ok_and(|o| o.status.success())
}

fn wasm_target_installed() -> bool {
    let Ok(out) = Command::new("rustc").args(["--print", "target-libdir", "--target", "wasm32-unknown-unknown"]).output() else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let dir = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    std::fs::read_dir(dir).ok().into_iter().flatten().flatten().any(|e| e.file_name().to_string_lossy().starts_with("libcore"))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::{repo_files, verify_exact_single_test_passed};

    fn temp_git_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lc-xtask-main-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        run_git(&dir, &["init"]);
        run_git(&dir, &["config", "user.name", "Test"]);
        run_git(&dir, &["config", "user.email", "test@example.com"]);
        dir
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let out = Command::new("git").current_dir(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {:?} failed: {}", args, String::from_utf8_lossy(&out.stderr));
    }

    #[test]
    fn repo_files_preserves_non_ascii_and_quoted_paths() {
        let dir = temp_git_dir("non-ascii");
        let tracked = ["assets/logó.png", "assets/a\"b.png", "crates/format/tests/fixtures/données.pzfx"];
        for rel in tracked {
            let path = dir.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"fixture").unwrap();
        }
        run_git(&dir, &["add", "--", "assets", "crates"]);

        let untracked = "packaging/linux/résumé.desktop";
        let untracked_path = dir.join(untracked);
        std::fs::create_dir_all(untracked_path.parent().unwrap()).unwrap();
        std::fs::write(&untracked_path, b"[Desktop Entry]\n").unwrap();

        let files = repo_files(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            files,
            vec![
                String::from("assets/a\"b.png"),
                String::from("assets/logó.png"),
                String::from("crates/format/tests/fixtures/données.pzfx"),
                String::from("packaging/linux/résumé.desktop"),
            ]
        );
    }

    #[test]
    fn repo_files_skips_deleted_tracked_files_without_failing() {
        let dir = temp_git_dir("deleted");
        std::fs::write(dir.join("kept.txt"), b"ok").unwrap();
        std::fs::write(dir.join("removed.txt"), b"gone").unwrap();
        run_git(&dir, &["add", "--", "kept.txt", "removed.txt"]);
        run_git(&dir, &["commit", "-m", "initial"]);

        std::fs::remove_file(dir.join("removed.txt")).unwrap();
        let files = repo_files(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(files, vec![String::from("kept.txt")]);
    }

    #[cfg(unix)]
    #[test]
    fn repo_files_rejects_broken_symlink_and_unreadable_file() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let dir_sym = temp_git_dir("symlink");
        symlink("/nonexistent/target", dir_sym.join("broken.txt")).unwrap();
        run_git(&dir_sym, &["add", "--", "broken.txt"]);
        let sym_res = repo_files(&dir_sym);
        let _ = std::fs::remove_dir_all(&dir_sym);
        assert!(sym_res.is_err(), "expected broken symlink to fail repo_files: {sym_res:?}");

        let dir_perm = temp_git_dir("unreadable");
        let secret = dir_perm.join("unreadable.txt");
        std::fs::write(&secret, b"data").unwrap();
        run_git(&dir_perm, &["add", "--", "unreadable.txt"]);
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o000)).unwrap();
        let perm_res = repo_files(&dir_perm);
        let _ = std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644));
        let _ = std::fs::remove_dir_all(&dir_perm);
        assert!(perm_res.is_err(), "expected unreadable file to fail repo_files: {perm_res:?}");
    }

    #[test]
    fn i18n_report_requires_exact_single_passing_test() {
        const TEST: &str = "i18n::tests::untranslated_report";
        let valid = "running 1 test\ntest i18n::tests::untranslated_report ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n";
        assert!(verify_exact_single_test_passed(valid, TEST).is_ok());

        // Even with --nocapture output before `ok`, `test <name> ...` line and `1 passed; 0 failed; 0 ignored;` still pass.
        let nocapture = "running 1 test\nreport line 1\ntest i18n::tests::untranslated_report ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n";
        assert!(verify_exact_single_test_passed(nocapture, TEST).is_ok());

        for bad in [
            "running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n",
            "running 11 tests\ntest i18n::tests::untranslated_report ... ok\n\ntest result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n",
            "running 1 test\ntest i18n::tests::untranslated_report ... ignored\n\ntest result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out\n",
            "running 1 test\ntest other::test ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n",
            "running 1 test\ntest i18n::tests::untranslated_report ... FAILED\n\ntest result: FAILED. 0 passed; 1 failed; 0 ignored;\n",
        ] {
            let res = verify_exact_single_test_passed(bad, TEST);
            assert!(res.is_err(), "expected failure for output: {bad}");
        }
    }
}
