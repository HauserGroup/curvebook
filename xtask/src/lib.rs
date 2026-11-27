//! Workspace quality gates, packaging helpers, and architectural checks for CurveBook.
#![forbid(unsafe_code)]
#![allow(clippy::indexing_slicing)] // Build/policy CLI scanners with heavy tabular/parser slicing (FA-SEC23-03); string_slice remains denied and all 14 I/O/boundary/app crates enforce workspace indexing_slicing = "deny".

mod assets;
pub mod brands;
mod bundle;
mod ci;
mod ci_quick;
mod citations;
mod cleanroom;
mod commands;
mod contributors;
mod corpus;
mod coverage;
mod design_tokens;
#[cfg(feature = "docs-site")]
mod docs_site;
mod ico;
pub mod interop_corpus;
mod layers;
mod licenses;
mod msix;
mod pages_site;
mod pending;
mod release_scan;
mod rust_lex;
mod scan_util;
mod schemas;
mod screenshots;
mod secrets;
mod stats;
mod status;
mod telemetry;
mod text_decode;
mod ui_commands;
mod version;
mod web_fonts;
mod workflows;
mod zip_read;

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

const HELP: &str = "\
usage: cargo xtask <command> [args...]

commands:
  ci [stage]            run default gates or build/policy/lint/tests/doctests/features/performance/wasm
  ci quick [--direct|--transitive] [--base <ref>]
                        policy gates + reverse-dep clippy + direct-crate tests (use --transitive for rdep tests)
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
  design-tokens [--verbose]
                        verify raw UI colours and sizes in crates/ui-egui/src stay within ratcheted ceilings
  i18n-report           report untranslated locale keys in curvebook-ui-egui
  ui-commands [--check] sync (or verify) crates/mcp/src/ui_catalog.rs from crates/ui-egui
  commands [--check]    regenerate (or verify) docs/commands.md from curvebook-engine
  schemas [--check]     regenerate (or verify) docs/schemas/*.schema.json from curvebook-format
  docs-site [--check] [--output <dir>] [--build-manual [<dir>]] [--manual-dir <dir>] [--release <ver>] [--sitemap]
                        build (or verify) the static GitHub Pages documentation site (requires `--features docs-site`)
  pages-stage [--manual-dir <dir>] (<dir> | --list | --check)
                        copy only the published Pages site files into <dir> (or list / verify them)
  wasm                  check + clippy (-D warnings) wasm32 targets, trunk web-release build, and web bundle readiness
  web-bundle            verify web bundle readiness and R10.22 zip size budget (CURVEBOOK_REQUIRE_WEB_BUNDLE=1 requires zip)
  corpus [--fetch]      run offline corpus & NIST StRD suites (optionally fetch corpus/nist-strd)
  interop-corpus [--strict] [--observe] [--corpus <dir>] [--ledger-dir <dir>] [--structural-only]
                        verify & observe external .pzfx/.prism interop corpus (aggregate stats only)
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

pub fn run_cli() -> ExitCode {
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
        "design-tokens" => design_tokens::run(&root, rest),
        "i18n-report" => cmd_i18n_report(&root),
        "ui-commands" => ui_commands::run(&root, parse_single_flag(rest, "--check", "usage: cargo xtask ui-commands [--check]")?),
        "commands" => commands::run_cli(&root, rest),
        "schemas" => schemas::run_cli(&root, rest),
        "docs-site" => cmd_docs_site(&root, rest),
        "pages-stage" => pages_site::run(&root, rest),
        "wasm" => cmd_wasm(&root),
        "web-bundle" => web_fonts::run_bundle_check(&root, rest),
        "corpus" => corpus::run(&root, rest),
        "interop-corpus" => interop_corpus::run(&root, rest),
        "coverage" => coverage::run(&root, rest),
        "bundle" => bundle::run(&root),
        "msix" => msix::run(&root, rest),
        "release-scan" => release_scan::run(&root, rest),
        "ico" => ico::run(rest),
        "version" => version::run(&root, rest),
        "stats" => stats::run(&root, parse_single_flag(rest, "--exact", "usage: cargo xtask stats [--exact]")?),
        "contributors" => contributors::run(&root),
        "web-fonts" => web_fonts::run(&root),
        other => Err(format!("unknown xtask command `{other}`\n\n{HELP}")),
    }
}

fn parse_single_flag(args: &[String], flag: &str, usage: &str) -> Result<bool, String> {
    match args {
        [] => Ok(false),
        [a] if a == flag => Ok(true),
        _ => Err(format!("unknown argument(s) `{}`; {usage}", args.join(" "))),
    }
}

#[cfg(feature = "docs-site")]
fn cmd_docs_site(root: &Path, rest: &[String]) -> Result<(), String> {
    docs_site::run_cli(root, rest)
}

#[cfg(not(feature = "docs-site"))]
fn cmd_docs_site(_root: &Path, _rest: &[String]) -> Result<(), String> {
    Err(String::from("`docs-site` requires `--features docs-site` (for example `cargo run -p xtask --features docs-site -- docs-site`)"))
}

pub(crate) fn resolve_target_dir_from_env(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .map_or_else(|| root.join("target"), |p| if p.is_absolute() { p } else { root.join(p) })
}

fn resolve_prebuilt_example_bin(dir: &Path, example: &str) -> Result<PathBuf, String> {
    if example.is_empty() || example.contains('/') || example.contains('\\') || example.contains("..") || example.contains('\0') {
        return Err(format!("invalid example binary name `{example}`"));
    }
    let bin = dir.join(format!("{example}{}", std::env::consts::EXE_SUFFIX));
    if !bin.is_file() {
        return Err(format!("CURVEBOOK_PREBUILT_EXAMPLES_DIR is set, but prebuilt example binary `{}` was not found", bin.display()));
    }
    Ok(bin)
}

pub(crate) fn run_workspace_example(root: &Path, package: &str, example: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    if let Some(dir) = std::env::var_os("CURVEBOOK_PREBUILT_EXAMPLES_DIR").filter(|v| !v.is_empty()).map(PathBuf::from) {
        let bin = resolve_prebuilt_example_bin(&dir, example)?;
        let bin_str = bin.to_str().ok_or_else(|| format!("non-UTF-8 prebuilt example path {}", bin.display()))?;
        let out = dev_tool_command(bin_str)
            .current_dir(root)
            .args(args.iter().copied())
            .wall_timeout(Duration::from_secs(900))
            .idle_timeout(Duration::from_secs(600))
            .run()
            .map_err(|e| format!("run prebuilt example {}: {e}", bin.display()))?;
        if !out.success {
            return Err(format!(
                "prebuilt example `{}` failed ({:?}):\n{}",
                bin.display(),
                out.status_code,
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        return Ok(out.stdout);
    }
    let target_dir = resolve_target_dir_from_env(root);
    let target_str = target_dir.to_str().ok_or_else(|| format!("non-UTF-8 target dir {}", target_dir.display()))?;
    let mut cmd_args = vec!["run", "--locked", "-p", package, "--example", example];
    if !args.is_empty() {
        cmd_args.push("--");
        cmd_args.extend_from_slice(args);
    }
    let out = dev_tool_command("cargo")
        .current_dir(root)
        .env("CARGO_TARGET_DIR", target_str)
        .args(cmd_args)
        .wall_timeout(Duration::from_secs(900))
        .idle_timeout(Duration::from_secs(600))
        .run()
        .map_err(|e| format!("cargo run -p {package} --example {example}: {e}"))?;
    if !out.success {
        return Err(format!(
            "`cargo run --locked -p {package} --example {example}` failed ({:?}):\n{}",
            out.status_code,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
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
    out.stdout
        .split(|&b| b == 0)
        .filter(|r| !r.is_empty())
        .map(|r| std::str::from_utf8(r).map(str::to_owned).map_err(|e| format!("git ls-files utf-8: {e}")))
        .collect()
}

fn run_cargo_metadata(root: &Path, no_deps: bool) -> Result<serde_json::Value, String> {
    let mut args = vec!["metadata"];
    if root.join("Cargo.lock").is_file() {
        args.push("--locked");
    }
    args.extend(["--format-version", "1"]);
    if no_deps {
        args.push("--no-deps");
    }
    let out = Command::new("cargo").current_dir(root).args(&args).output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata json: {e}"))
}

pub(crate) fn cargo_metadata(root: &Path) -> Result<serde_json::Value, String> {
    run_cargo_metadata(root, true)
}
pub(crate) fn cargo_metadata_resolved(root: &Path) -> Result<serde_json::Value, String> {
    run_cargo_metadata(root, false)
}

fn cmd_layers(root: &Path) -> Result<(), String> {
    let graph = layers::from_metadata(&cargo_metadata_resolved(root)?)?;
    let mut violations = layers::check(&graph);
    violations.extend(layers::check_manifest_patches(root));
    let mut hygiene = layers::check_source_hygiene(root);
    hygiene.extend(layers::check_lint_policy(root));
    hygiene.extend(layers::check_integration_test_binaries(root));
    if violations.is_empty() && hygiene.is_empty() {
        println!("layers: all {} workspace packages satisfy L0..L7 dependency rules and source hygiene", graph.len());
        return Ok(());
    }
    let lines: Vec<String> = violations.iter().map(ToString::to_string).chain(hygiene.iter().map(ToString::to_string)).collect();
    Err(scan_util::format_error_list("layer/hygiene violation(s)", &lines))
}

fn cmd_test(root: &Path, extra: &[String]) -> Result<(), String> {
    let use_nextest = has_cargo_nextest();
    let mut args: Vec<String> = (if use_nextest { &["nextest", "run"][..] } else { &["test"] })
        .iter()
        .chain(&["--locked", "--workspace", "--all-targets"])
        .map(|&s| s.to_owned())
        .collect();
    args.extend_from_slice(extra);
    run_cargo(root, &args)?;
    if extra.is_empty() {
        run_cargo(root, &ci::docs_site_test_args(use_nextest))?;
    }
    Ok(())
}

fn cmd_i18n_report(root: &Path) -> Result<(), String> {
    const TEST_PATH: &str = "i18n::tests::untranslated_report";
    let args = ["test", "--locked", "-p", "curvebook-ui-egui", "--lib", TEST_PATH, "--", "--exact", "--nocapture"];
    println!("+ cargo {}", args.join(" "));
    let out = Command::new("cargo").current_dir(root).args(args).output().map_err(|e| format!("cargo i18n-report: {e}"))?;
    let _ = std::io::stdout().write_all(&out.stdout);
    let _ = std::io::stderr().write_all(&out.stderr);
    if !out.status.success() {
        return Err(format!("cargo i18n-report failed with {}", out.status));
    }
    verify_exact_single_test_passed(&String::from_utf8_lossy(&out.stdout), TEST_PATH)
}

fn verify_exact_single_test_passed(stdout: &str, test_name: &str) -> Result<(), String> {
    let summary = stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("test result: ok."))
        .ok_or_else(|| format!("i18n-report: expected `test result: ok.` summary for `{test_name}`"))?;
    let parse_k = |sfx: &str| summary.split(';').find_map(|p| p.trim().strip_suffix(sfx)?.trim().parse::<usize>().ok());
    if (parse_k(" passed"), parse_k(" failed"), parse_k(" ignored")) != (Some(1), Some(0), Some(0)) {
        return Err(format!(
            "i18n-report: expected exact single passing test for `{test_name}` (1 passed; 0 failed; 0 ignored), got `{}`",
            summary.trim()
        ));
    }
    if !stdout.lines().any(|l| l.trim_start().starts_with(&format!("test {test_name} ..."))) {
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
    if !Command::new("trunk").arg("--version").output().is_ok_and(|o| o.status.success()) {
        if require_trunk {
            return Err(String::from(
                "`trunk` is not installed in PATH (required when CURVEBOOK_REQUIRE_TRUNK=1 or CURVEBOOK_REQUIRE_WEB_BUNDLE=1 is set)",
            ));
        }
        println!("wasm: `trunk` not found in PATH; skipping `trunk build --cargo-profile web-release` (set CURVEBOOK_REQUIRE_TRUNK=1 to require it)");
        return Ok(());
    }
    let dist_dir = resolve_target_dir_from_env(root).join("wasm-trunk-dist");
    std::fs::create_dir_all(&dist_dir).map_err(|e| format!("mkdir {}: {e}", dist_dir.display()))?;
    let dist_arg = dist_dir.to_string_lossy().into_owned();
    println!("+ (cd apps/curvebook-web && trunk build --dist {dist_arg} --cargo-profile web-release)");
    let status = Command::new("trunk")
        .current_dir(root.join("apps/curvebook-web"))
        .args(["build", "--dist", &dist_arg, "--cargo-profile", "web-release"])
        .status()
        .map_err(|e| format!("trunk build: {e}"))?;
    if !status.success() {
        return Err(format!("`trunk build --cargo-profile web-release` failed with {status}"));
    }
    if !dist_dir.join("index.html").is_file() {
        return Err(format!("trunk build succeeded, but `{}/index.html` is missing", dist_dir.display()));
    }
    if !std::fs::read_dir(&dist_dir)
        .map_err(|e| format!("read {}: {e}", dist_dir.display()))?
        .flatten()
        .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some("wasm"))
    {
        return Err(format!("trunk build succeeded, but `{}` contains no `.wasm` binary", dist_dir.display()));
    }
    Ok(())
}

fn cargo_home_bin_dir(cargo_home: Option<&OsStr>, home: Option<&OsStr>, userprofile: Option<&OsStr>) -> Option<PathBuf> {
    if let Some(ch) = cargo_home.filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(ch).join("bin"));
    }
    home.filter(|v| !v.is_empty()).or_else(|| userprofile.filter(|v| !v.is_empty())).map(|h| PathBuf::from(h).join(".cargo").join("bin"))
}

fn append_dir_to_path_if_missing(current_path: Option<&OsStr>, extra_dir: &Path) -> Option<OsString> {
    let mut dirs: Vec<PathBuf> = current_path.map(std::env::split_paths).into_iter().flatten().collect();
    if dirs.iter().any(|dir| dir == extra_dir) {
        return None;
    }
    dirs.push(extra_dir.to_path_buf());
    std::env::join_paths(dirs).ok()
}

fn cargo_path_with_fallback() -> Option<OsString> {
    let (ch, h, up) = (std::env::var_os("CARGO_HOME"), std::env::var_os("HOME"), std::env::var_os("USERPROFILE"));
    let bin_dir = cargo_home_bin_dir(ch.as_deref(), h.as_deref(), up.as_deref())?;
    bin_dir.is_dir().then(|| append_dir_to_path_if_missing(std::env::var_os("PATH").as_deref(), &bin_dir)).flatten()
}

fn cargo_command() -> Command {
    let mut cmd = Command::new("cargo");
    if let Some(path) = cargo_path_with_fallback() {
        cmd.env("PATH", path);
    }
    cmd
}

fn run_cargo_strs(root: &Path, args: &[&str]) -> Result<(), String> {
    run_cargo(root, &args.iter().map(|&s| s.to_owned()).collect::<Vec<_>>())
}

fn run_cargo(root: &Path, args: &[String]) -> Result<(), String> {
    println!("+ cargo {}", args.join(" "));
    let status = cargo_command().current_dir(root).args(args).status().map_err(|e| format!("cargo {}: {e}", args.join(" ")))?;
    if status.success() { Ok(()) } else { Err(format!("`cargo {}` failed with {status}", args.join(" "))) }
}

fn has_cargo_nextest() -> bool {
    cargo_command().args(["nextest", "--version"]).output().is_ok_and(|o| o.status.success())
}

pub(crate) fn dev_tool_command(program: &str) -> curvebook_platform::process::ProcessCommand {
    const FORWARD_ENV_KEYS: &str = "PATH PATHEXT CARGO CARGO_HOME CARGO_TARGET_DIR CARGO_NET_OFFLINE RUSTUP_HOME RUSTUP_TOOLCHAIN RUSTC RUSTC_WRAPPER RUSTFMT RUSTFLAGS RUSTDOCFLAGS LLVM_COV LLVM_PROFDATA CC CXX AR SDKROOT DEVELOPER_DIR MACOSX_DEPLOYMENT_TARGET CI GITHUB_ACTIONS HTTPS_PROXY HTTP_PROXY NO_PROXY SSL_CERT_FILE SSL_CERT_DIR";
    let mut cmd = curvebook_platform::process::ProcessCommand::new(program);
    for key in FORWARD_ENV_KEYS.split_ascii_whitespace() {
        if let Ok(val) = std::env::var(key)
            && !val.is_empty()
        {
            cmd = cmd.env(key, val);
        }
    }
    if program == "cargo"
        && let Some(path) = cargo_path_with_fallback().and_then(|p| p.into_string().ok())
    {
        cmd = cmd.env("PATH", path);
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan_util::test_support::TestDir;

    #[test]
    fn repo_files_handles_non_ascii_deleted_and_unreadable() {
        let dir = TestDir::git_repo("main-non-ascii");
        for rel in ["assets/logó.png", "assets/a\"b.png", "crates/format/tests/fixtures/données.pzfx"] {
            dir.write(rel, "fixture");
        }
        dir.run_git(&["add", "--", "assets", "crates"]);
        dir.write("packaging/linux/résumé.desktop", "[Desktop Entry]\n");
        assert_eq!(
            repo_files(dir.path()).unwrap(),
            vec!["assets/a\"b.png", "assets/logó.png", "crates/format/tests/fixtures/données.pzfx", "packaging/linux/résumé.desktop"]
        );

        let del = TestDir::git_repo("main-deleted");
        del.write("kept.txt", "ok");
        del.write("removed.txt", "gone");
        del.run_git(&["add", "--", "kept.txt", "removed.txt"]);
        del.run_git(&["commit", "-m", "initial"]);
        std::fs::remove_file(del.join("removed.txt")).unwrap();
        assert_eq!(repo_files(del.path()).unwrap(), vec!["kept.txt"]);

        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            let s = TestDir::git_repo("main-symlink");
            symlink("/nonexistent/target", s.join("broken.txt")).unwrap();
            s.run_git(&["add", "--", "broken.txt"]);
            assert!(repo_files(s.path()).is_err());

            let p = TestDir::git_repo("main-unreadable");
            let secret = p.write("unreadable.txt", "data");
            p.run_git(&["add", "--", "unreadable.txt"]);
            std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o000)).unwrap();
            let res = repo_files(p.path());
            let _ = std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644));
            assert!(res.is_err());
        }
    }

    #[test]
    fn i18n_report_parse_single_flag_dev_tool_and_cargo_path_fallback() {
        const TEST: &str = "i18n::tests::untranslated_report";
        for ok in [
            "running 1 test\ntest i18n::tests::untranslated_report ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n",
            "running 1 test\nreport\ntest i18n::tests::untranslated_report ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored;\n",
        ] {
            assert!(verify_exact_single_test_passed(ok, TEST).is_ok());
        }
        for bad in [
            "test result: ok. 0 passed; 0 failed; 0 ignored;\n",
            "test i18n::tests::untranslated_report ... ok\ntest result: ok. 11 passed; 0 failed; 0 ignored;\n",
            "test i18n::tests::untranslated_report ... ignored\ntest result: ok. 0 passed; 0 failed; 1 ignored;\n",
            "test other::test ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored;\n",
            "test i18n::tests::untranslated_report ... FAILED\ntest result: FAILED. 0 passed; 1 failed; 0 ignored;\n",
        ] {
            assert!(verify_exact_single_test_passed(bad, TEST).is_err(), "{bad}");
        }
        let out = dev_tool_command("rustfmt").arg("--version").run().expect("rustfmt on host PATH should execute");
        assert!(out.success && String::from_utf8_lossy(&out.stdout).contains("rustfmt"));
        assert_eq!(
            cargo_home_bin_dir(Some(OsStr::new("/custom/cargo")), Some(OsStr::new("/opt/dev")), None),
            Some(PathBuf::from("/custom/cargo/bin"))
        );
        assert_eq!(cargo_home_bin_dir(Some(OsStr::new("")), Some(OsStr::new("/opt/dev")), None), Some(PathBuf::from("/opt/dev/.cargo/bin")));
        assert_eq!(cargo_home_bin_dir(None, None, Some(OsStr::new("D:\\dev"))), Some(PathBuf::from("D:\\dev/.cargo/bin")));
        assert_eq!(cargo_home_bin_dir(None, None, None), None);
        let existing = std::env::join_paths([Path::new("/usr/local/bin"), Path::new("/usr/bin")]).unwrap();
        let updated = append_dir_to_path_if_missing(Some(existing.as_os_str()), Path::new("/opt/dev/.cargo/bin")).unwrap();
        assert_eq!(
            std::env::split_paths(&updated).collect::<Vec<_>>(),
            vec![PathBuf::from("/usr/local/bin"), PathBuf::from("/usr/bin"), PathBuf::from("/opt/dev/.cargo/bin")]
        );
        assert!(append_dir_to_path_if_missing(Some(updated.as_os_str()), Path::new("/opt/dev/.cargo/bin")).is_none());
        assert_eq!((parse_single_flag(&[], "--check", "u"), parse_single_flag(&["--check".into()], "--check", "u")), (Ok(false), Ok(true)));
        assert!(
            parse_single_flag(&["--chck".into()], "--check", "u").is_err()
                && parse_single_flag(&["--check".into(), "--check".into()], "--check", "u").is_err()
        );
        let ex_dir = TestDir::new("prebuilt-examples");
        let bin_name = format!("gen_commands{}", std::env::consts::EXE_SUFFIX);
        let created = ex_dir.write(&bin_name, "bin");
        assert_eq!(super::resolve_prebuilt_example_bin(ex_dir.path(), "gen_commands").unwrap(), created);
        for bad in ["", "../gen_commands", "sub/gen_commands", "gen_schemas"] {
            assert!(super::resolve_prebuilt_example_bin(ex_dir.path(), bad).is_err(), "{bad}");
        }
    }
}
