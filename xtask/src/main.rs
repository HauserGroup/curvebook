//! Workspace quality gates, packaging helpers, and architectural checks for CurveBook.
#![forbid(unsafe_code)]

mod assets;
mod brands;
mod bundle;
mod ci;
mod ci_quick;
mod citations;
mod cleanroom;
mod commands;
mod contributors;
mod corpus;
mod coverage;
mod cursors;
mod docs_site;
mod ico;
mod json;
mod layers;
mod licenses;
mod msix;
mod pending;
mod release_scan;
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
  ci [stage]            run default gates or build/policy/lint/tests/doctests/features/performance/wasm
  ci quick [--base <ref>] policy gates + clippy/tests for packages changed since main and their dependents
  r10-gate              run R10 parity, Unicode, screenshot baseline, web parity & latency gates
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
  citations             verify D1 literature citations across math, stats, fit, and transform
  cursors               verify Cursor variants match crates/tools/src/cursors.toml
  i18n-report           report untranslated locale keys in curvebook-ui-egui
  ui-commands [--check] sync (or verify) crates/mcp/src/ui_catalog.rs from crates/ui-egui
  commands [--check]    regenerate (or verify) docs/commands.md from curvebook-engine
  schemas [--check]     regenerate (or verify) docs/schemas/*.schema.json from curvebook-format
  docs-site [--check] [--output <dir>] [--release <ver>]
                        build (or verify) the static GitHub Pages documentation site
  wasm                  check + clippy (-D warnings) wasm32 targets, trunk web-release build, and web bundle readiness
  web-bundle            verify web bundle readiness and R10.22 zip size budget (CURVEBOOK_REQUIRE_WEB_BUNDLE=1 requires zip)
  corpus [--fetch]      run offline corpus & NIST StRD suites (optionally fetch corpus/nist-strd)
  coverage [-p <crate>] write a local cargo-llvm-cov lcov + HTML report (diagnostic; no threshold)
  bundle                build release binaries and assemble dist/CurveBook.app
  msix --arch <arch> --package-version <A.B.C.0>  package Windows binaries for Microsoft Store
  release-scan <dir>    unpack release assets and run SHA-256, brand, cleanroom & asset gates
  ico <out> <png...>    pack PNG frames (<=256px) into a Windows .ico container
  version [set <ver>]   print or update [workspace.package] version
  stats [--exact]       print line and test counts across workspace crates
  contributors          refresh contributors/contributors.json from git history and people.toml
  web-fonts             stage optional CURVEBOOK_FONTS_DIR faces into dist/web/fonts
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
        "ci" => ci::run(&root, rest),
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
        "citations" => citations::run(&root, rest),
        "cursors" => cursors::run(&root),
        "i18n-report" => cmd_i18n_report(&root),
        "ui-commands" => ui_commands::run(&root, rest.iter().any(|a| a == "--check")),
        "commands" => commands::run_cli(&root, rest),
        "schemas" => schemas::run_cli(&root, rest),
        "docs-site" => docs_site::run_cli(&root, rest),
        "wasm" => cmd_wasm(&root),
        "web-bundle" => web_fonts::run_bundle_check(&root, rest),
        "corpus" => corpus::run(&root, rest),
        "coverage" => coverage::run(&root, rest),
        "bundle" => bundle::run(&root),
        "msix" => msix::run(&root, rest),
        "release-scan" => release_scan::run(&root, rest),
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
        let mut args =
            vec![String::from("nextest"), String::from("run"), String::from("--locked"), String::from("--workspace"), String::from("--all-targets")];
        args.extend_from_slice(extra);
        run_cargo(root, &args)
    } else {
        let mut args = vec![String::from("test"), String::from("--locked"), String::from("--workspace"), String::from("--all-targets")];
        args.extend_from_slice(extra);
        run_cargo(root, &args)
    }
}

fn cmd_i18n_report(root: &Path) -> Result<(), String> {
    const TEST_PATH: &str = "i18n::tests::untranslated_report";
    let args = ["test", "-p", "curvebook-ui-egui", "--lib", TEST_PATH, "--", "--exact", "--nocapture"];
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
    run_cargo_strs(
        root,
        &["check", "--locked", "--target", "wasm32-unknown-unknown", "-p", "curvebook-web", "-p", "curvebook-engine", "--keep-going"],
    )?;
    run_cargo_strs(
        root,
        &["clippy", "--locked", "--target", "wasm32-unknown-unknown", "-p", "curvebook-web", "-p", "curvebook-ui-egui", "--", "-D", "warnings"],
    )?;
    run_trunk_web_release_if_available(root)?;
    web_fonts::check_web_bundle_readiness(root)
}

fn run_trunk_web_release_if_available(root: &Path) -> Result<(), String> {
    let require_trunk = workflows::parse_ci_env_value(std::env::var_os("CURVEBOOK_REQUIRE_TRUNK").as_deref())
        || web_fonts::parse_require_web_bundle_env(std::env::var_os("CURVEBOOK_REQUIRE_WEB_BUNDLE").as_deref());
    if !has_trunk() {
        if require_trunk {
            return Err(String::from(
                "`trunk` is not installed in PATH (required when CURVEBOOK_REQUIRE_TRUNK=1 or CURVEBOOK_REQUIRE_WEB_BUNDLE=1 is set)",
            ));
        }
        println!("wasm: `trunk` not found in PATH; skipping `trunk build --cargo-profile web-release` (set CURVEBOOK_REQUIRE_TRUNK=1 to require it)");
        return Ok(());
    }

    let target_dir = std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from);
    let dist_dir = target_dir.join("wasm-trunk-dist");
    std::fs::create_dir_all(&dist_dir).map_err(|e| format!("mkdir {}: {e}", dist_dir.display()))?;
    let dist_arg = dist_dir.to_string_lossy().into_owned();
    let web_dir = root.join("apps/curvebook-web");
    println!("+ (cd apps/curvebook-web && trunk build --dist {dist_arg} --cargo-profile web-release)");
    let status = Command::new("trunk")
        .current_dir(&web_dir)
        .args(["build", "--dist", &dist_arg, "--cargo-profile", "web-release"])
        .status()
        .map_err(|e| format!("trunk build: {e}"))?;
    if !status.success() {
        return Err(format!("`trunk build --cargo-profile web-release` failed with {status}"));
    }
    if !dist_dir.join("index.html").is_file() {
        return Err(format!("trunk build succeeded, but `{}/index.html` is missing", dist_dir.display()));
    }
    let has_wasm = std::fs::read_dir(&dist_dir)
        .map_err(|e| format!("read {}: {e}", dist_dir.display()))?
        .flatten()
        .any(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("wasm"));
    if !has_wasm {
        return Err(format!("trunk build succeeded, but `{}` contains no `.wasm` binary", dist_dir.display()));
    }
    Ok(())
}

fn has_trunk() -> bool {
    Command::new("trunk").arg("--version").output().is_ok_and(|o| o.status.success())
}

fn cmd_r10_gate(root: &Path) -> Result<(), String> {
    run_cargo_strs(root, &["test", "-p", "curvebook-ui-egui", "--test", "r10_core_and_spike"])?;
    run_cargo_strs(root, &["test", "-p", "curvebook-ui-egui", "--test", "r10_phase7_gates"])?;
    run_cargo_strs(root, &["test", "-p", "curvebook-web", "--test", "r10_7_web_shared_layout"])?;
    screenshots::run(root, &[String::from("--check")])?;
    web_fonts::check_web_bundle_readiness(root)?;
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::{repo_files, verify_exact_single_test_passed};

    fn temp_git_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cb-xtask-main-{}-{label}", std::process::id()));
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
