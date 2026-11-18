//! The same quality gates, runnable together locally or as independent hosted CI stages.

use std::path::Path;
use std::time::Instant;

use crate::{
    assets, brands, cleanroom, cmd_i18n_report, cmd_layers, cmd_test, cmd_wasm, commands, cursors, has_cargo_nextest, licenses, pending, run_cargo,
    run_cargo_strs, schemas, screenshots, secrets, status, telemetry, ui_commands, wasm_target_installed, web_fonts, workflows,
};

// Keep existing UI frame/latency (`r10_*`, including `r10_phase7_gates`), human-path
// input-to-paint budgets (`human_wf_*`), grid, and curve-fit throughput gates off the
// parallel correctness runner. The complement below still covers every other test.
const PERFORMANCE_FILTER: &str = "binary(r10_*) | binary(human_wf_*) | binary(*perf*)";
const STAGES: &[&str] = &["policy", "lint", "tests", "doctests", "features", "wasm", "performance"];

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    match args {
        [] => run_all(root),
        [stage] if stage == "build" || STAGES.contains(&stage.as_str()) => run_stage(root, stage),
        _ => Err(format!("usage: cargo xtask ci [build|{}]", STAGES.join("|"))),
    }
}

fn run_all(root: &Path) -> Result<(), String> {
    run_stage(root, "policy")?;
    run_stage(root, "lint")?;
    if has_cargo_nextest() {
        run_stage(root, "tests")?;
        run_stage(root, "performance")?;
    } else {
        // cargo test includes the entire R10 binary; do not execute those tests twice.
        cmd_test(root, &[])?;
    }
    run_stage(root, "doctests")?;
    run_stage(root, "features")?;
    if workflows::is_ci_env() || wasm_target_installed() {
        run_stage(root, "wasm")?;
    } else {
        println!("wasm: target not installed locally; enforced when CI=true");
    }
    println!("ci: all checks passed");
    Ok(())
}

fn run_stage(root: &Path, stage: &str) -> Result<(), String> {
    let start = Instant::now();
    println!("ci: starting {stage}");
    match stage {
        "build" => build_archive(root)?,
        "policy" => policy(root)?,
        "lint" => run_cargo_strs(root, &["clippy", "--locked", "--workspace", "--all-targets", "--", "-D", "warnings"])?,
        "tests" => native_tests(root, false)?,
        "performance" => native_tests(root, true)?,
        "doctests" => {
            run_cargo_strs(root, &["test", "--locked", "--workspace", "--doc"])?;
            rustdoc(root)?;
        }
        "features" => feature_checks(root)?,
        "wasm" => cmd_wasm(root)?,
        _ => return Err(format!("unknown CI stage `{stage}`")),
    }
    println!("ci: {stage} passed in {:.2}s", start.elapsed().as_secs_f64());
    Ok(())
}

/// `RUSTDOCFLAGS` for [`rustdoc`]: every default-on rustdoc lint (broken or private intra-doc
/// links, invalid HTML tags or code-block attributes, bare URLs) fails the build. `missing_docs`
/// is a separate rustc lint and stays off here.
const RUSTDOC_FLAGS: &str = "-D warnings";

/// Build the workspace API docs and fail on any rustdoc warning, so doc links cannot rot silently.
fn rustdoc(root: &Path) -> Result<(), String> {
    let args = ["doc", "--locked", "--workspace", "--no-deps"];
    println!("+ RUSTDOCFLAGS=\"{RUSTDOC_FLAGS}\" cargo {}", args.join(" "));
    let status = std::process::Command::new("cargo")
        .current_dir(root)
        .env("RUSTDOCFLAGS", RUSTDOC_FLAGS)
        .args(args)
        .status()
        .map_err(|e| format!("cargo {}: {e}", args.join(" ")))?;
    if !status.success() {
        return Err(format!("`cargo {}` failed with {status}; fix the rustdoc warnings above", args.join(" ")));
    }
    Ok(())
}

fn require_nextest() -> Result<(), String> {
    if has_cargo_nextest() {
        Ok(())
    } else {
        Err(String::from("this CI stage requires cargo-nextest; install the pinned CI version or use cargo xtask ci for the cargo-test fallback"))
    }
}

fn build_archive(root: &Path) -> Result<(), String> {
    require_nextest()?;
    let args = ["nextest", "archive", "--locked", "--workspace", "--all-targets", "--archive-file", "target/ci-tests.tar.zst"];
    run_cargo_strs(root, &args)
}

fn test_args(root: &Path, performance: bool, archive: Option<&str>) -> Vec<String> {
    let mut args = vec!["nextest", "run", "--profile", "ci", "--no-fail-fast"].into_iter().map(String::from).collect::<Vec<_>>();
    if let Some(path) = archive {
        args.extend([String::from("--archive-file"), path.to_owned(), String::from("--workspace-remap"), root.to_string_lossy().into_owned()]);
        // Preserve compile-time CARGO_BIN_EXE_* paths across hosted jobs too.
        args.extend([String::from("--extract-to"), root.to_string_lossy().into_owned(), String::from("--extract-overwrite")]);
    } else {
        args.extend([String::from("--locked"), String::from("--workspace"), String::from("--all-targets")]);
    }
    let filter = if performance { PERFORMANCE_FILTER.to_owned() } else { format!("not ({PERFORMANCE_FILTER})") };
    args.extend([String::from("-E"), filter]);
    if performance {
        args.extend([String::from("--test-threads"), String::from("1")]);
    }
    args
}

fn native_tests(root: &Path, performance: bool) -> Result<(), String> {
    require_nextest()?;
    let archive = std::env::var("CURVEBOOK_CI_TEST_ARCHIVE").map(Some).or_else(|err| match err {
        std::env::VarError::NotPresent => Ok(None),
        std::env::VarError::NotUnicode(_) => Err(String::from("CURVEBOOK_CI_TEST_ARCHIVE must be UTF-8")),
    })?;
    run_cargo(root, &test_args(root, performance, archive.as_deref()))
}

fn feature_checks(root: &Path) -> Result<(), String> {
    run_cargo_strs(root, &["test", "--locked", "-p", "curvebook-text", "--no-default-features"])?;
    if has_cargo_nextest() {
        // Preserve every no-font UI test, while also running its existing budgets serially.
        for performance in [false, true] {
            let mut args = test_args(root, performance, None);
            args.retain(|arg| arg != "--workspace");
            args.extend([String::from("-p"), String::from("curvebook-ui-egui"), String::from("--no-default-features")]);
            run_cargo(root, &args)?;
        }
        run_cargo_strs(root, &["test", "--locked", "-p", "curvebook-ui-egui", "--no-default-features", "--doc"])?;
    } else {
        run_cargo_strs(root, &["test", "--locked", "-p", "curvebook-ui-egui", "--no-default-features"])?;
    }
    run_cargo_strs(root, &["check", "--locked", "-p", "curvebook-mcp", "--no-default-features"])
}

fn policy(root: &Path) -> Result<(), String> {
    run_cargo_strs(root, &["fmt", "--all", "--", "--check"])?;
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
    // Formerly reached through `r10-gate` inside the monolithic `cmd_ci`; the R10 test
    // binaries themselves now run in the serial `performance` stage.
    web_fonts::check_web_bundle_readiness(root)?;
    pending::run_count(root)?;
    status::run(root)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{PERFORMANCE_FILTER, STAGES, run, test_args};

    #[test]
    fn archived_tests_do_not_rebuild_or_override_the_archived_feature_selection() {
        let args = test_args(Path::new("."), false, Some("tests.tar.zst"));
        assert!(args.windows(2).any(|pair| pair == ["--archive-file", "tests.tar.zst"]));
        assert!(!args.iter().any(|arg| ["--workspace", "--all-targets", "--locked"].contains(&arg.as_str())));
    }

    #[test]
    fn test_stages_use_complementary_filters_and_only_budget_tests_are_serial() {
        let regular = test_args(Path::new("."), false, None);
        let performance = test_args(Path::new("."), true, None);
        assert!(regular.windows(2).any(|pair| pair == ["-E", &format!("not ({PERFORMANCE_FILTER})")]));
        assert!(performance.windows(2).any(|pair| pair == ["-E", PERFORMANCE_FILTER]));
        assert!(!regular.iter().any(|arg| arg == "--test-threads"));
        assert!(performance.windows(2).any(|pair| pair == ["--test-threads", "1"]));
    }

    #[test]
    fn performance_filter_serializes_r10_and_human_path_latency_binaries() {
        for binary in ["binary(r10_*)", "binary(human_wf_*)", "binary(*perf*)"] {
            assert!(PERFORMANCE_FILTER.split(" | ").any(|term| term == binary), "PERFORMANCE_FILTER must select {binary}");
        }
    }

    #[test]
    fn workflow_runs_every_stage_and_unknown_stages_fail_before_spawning() {
        let workflow = include_str!("../../.github/workflows/ci.yml");
        let expected = format!("stage: [{}]", STAGES.join(", "));
        assert!(workflow.contains(&expected), "hosted CI must include every quality stage");
        assert!(run(Path::new("."), &[String::from("nonexistent")]).is_err());
    }
}
