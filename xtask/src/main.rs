//! Workspace quality gates, packaging helpers, and architectural checks for LumenCraft.
#![forbid(unsafe_code)]

mod assets;
mod brands;
mod bundle;
mod cleanroom;
mod contributors;
mod cursors;
mod ico;
mod json;
mod layers;
mod stats;
mod ui_commands;
mod version;
mod web_fonts;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const HELP: &str = "\
usage: cargo xtask <command> [args...]

commands:
  ci                    run full local/CI gate (fmt, clippy, tests, doctests, feature checks, policy gates, wasm)
  test [args...]        run workspace test suite (uses cargo-nextest when installed, else cargo test)
  layers                verify L0..L7 dependency rules and UI-crate isolation
  assets                verify all non-code assets are attributed in ASSETS.md
  brands                verify user-facing strings and packaging contain no competitor trademarks
  cleanroom             verify no vendor copyright headers or vendor-created fixtures exist
  cursors               report undocumented Cursor enum variants (warning-only)
  i18n-report           report untranslated locale keys in lumencraft-ui-egui
  ui-commands [--check] sync (or verify) crates/mcp/src/ui_catalog.rs from crates/ui-egui
  wasm                  check wasm32-unknown-unknown compilation across web-targeted crates
  corpus                run document/import corpus smoke checks
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
        "test" => cmd_test(&root, rest),
        "layers" => cmd_layers(&root),
        "assets" => assets::run(&root),
        "brands" => brands::run(&root),
        "cleanroom" => cleanroom::run(&root),
        "cursors" => cursors::run(&root),
        "i18n-report" => cmd_i18n_report(&root),
        "ui-commands" => ui_commands::run(&root, rest.iter().any(|a| a == "--check")),
        "wasm" => cmd_wasm(&root),
        "corpus" => cmd_corpus(&root),
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
    let out = Command::new("git")
        .current_dir(root)
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
        .output()
        .map_err(|e| format!("git ls-files: {e}"))?;
    if !out.status.success() {
        return Err(format!("git ls-files failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let text = String::from_utf8(out.stdout).map_err(|e| format!("git ls-files utf-8: {e}"))?;
    let mut files: Vec<String> = text.lines().filter(|l| !l.is_empty()).filter(|l| root.join(l).is_file()).map(str::to_owned).collect();
    files.sort();
    files.dedup();
    Ok(files)
}

pub(crate) fn cargo_metadata(root: &Path) -> Result<json::Json, String> {
    let out = Command::new("cargo")
        .current_dir(root)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let text = String::from_utf8(out.stdout).map_err(|e| format!("cargo metadata utf-8: {e}"))?;
    json::parse(&text)
}

fn cmd_layers(root: &Path) -> Result<(), String> {
    let meta = cargo_metadata(root)?;
    let graph = layers::from_metadata(&meta)?;
    let violations = layers::check(&graph);
    if violations.is_empty() {
        println!("layers: all {} workspace packages satisfy L0..L7 dependency rules", graph.len());
        return Ok(());
    }
    let lines: Vec<String> = violations.iter().map(ToString::to_string).collect();
    Err(format!("{} layer violation(s):\n  {}", violations.len(), lines.join("\n  ")))
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
    run_cargo_strs(root, &["test", "-p", "lumencraft-ui-egui", "--lib", "untranslated_report", "--", "--nocapture"])
}

fn cmd_wasm(root: &Path) -> Result<(), String> {
    run_cargo_strs(root, &["check", "--target", "wasm32-unknown-unknown", "-p", "lumencraft-web", "-p", "lumencraft-engine", "--keep-going"])
}

fn cmd_corpus(root: &Path) -> Result<(), String> {
    run_cargo_strs(root, &["test", "-p", "lumencraft-format", "-p", "lumencraft-testkit"])
}

fn cmd_ci(root: &Path) -> Result<(), String> {
    run_cargo_strs(root, &["fmt", "--all", "--", "--check"])?;
    run_cargo_strs(root, &["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])?;
    cmd_test(root, &[])?;
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
    cmd_layers(root)?;

    let in_ci = std::env::var_os("CI").is_some_and(|v| !v.is_empty());
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
