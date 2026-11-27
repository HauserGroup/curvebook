//! `cargo xtask coverage`: local source-coverage reports through `cargo-llvm-cov` (`R0.16`).

use std::path::{Path, PathBuf};
use std::time::Duration;

const USAGE: &str = "usage: cargo xtask coverage [-p <crate>]... [-- <test-runner args>]";
const EXCLUDED_PACKAGES: &[&str] = &["xtask", "curvebook-testkit"];
const IGNORE_FILENAME_REGEX: &str = r"(^|[/\\])(xtask|crates[/\\]testkit)[/\\]";
const INSTALL_HINT: &str = "install it with `cargo install --locked cargo-llvm-cov`";
const LLVM_TOOLS_HINT: &str = "run `rustup component add llvm-tools-preview`, or, for a toolchain without that component (for example Homebrew Rust), set `LLVM_COV` and `LLVM_PROFDATA` to the `llvm-cov` / `llvm-profdata` of an LLVM whose major version matches `rustc -vV`";

#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    packages: Vec<String>,
    runner_args: Vec<String>,
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    if args.iter().take_while(|a| a.as_str() != "--").any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return Ok(());
    }
    let opts = parse_args(args)?;
    ensure_tools(root)?;
    let out = Outputs::in_dir(&coverage_output_dir(root)?);
    for step in plan(&opts, crate::has_cargo_nextest(), &out) {
        std::fs::create_dir_all(&out.dir).map_err(|e| format!("{}: {e}", out.dir.display()))?;
        crate::run_cargo(root, &step)?;
    }
    let summary = std::fs::read_to_string(&out.summary_json).map_err(|e| format!("{}: {e}", out.summary_json.display()))?;
    let summary_doc: serde_json::Value = serde_json::from_str(&summary).map_err(|e| format!("{}: {e}", out.summary_json.display()))?;
    for line in summary_lines(&summary_doc)? {
        println!("coverage: {line}");
    }
    println!(
        "coverage: lcov report  {}\ncoverage: HTML report  {}\ncoverage: diagnostic only (no threshold); use it to find untested edge-case and error paths",
        out.lcov.display(),
        out.dir.join("html/index.html").display()
    );
    Ok(())
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let (mut opts, mut iter) = (Options::default(), args.iter());
    while let Some(arg) = iter.next() {
        let package = if arg == "--" {
            opts.runner_args.extend(iter.by_ref().cloned());
            break;
        } else if arg == "-p" || arg == "--package" {
            iter.next().cloned().ok_or_else(|| format!("`{arg}` needs a crate name\n{USAGE}"))?
        } else if let Some(name) = arg.strip_prefix("--package=") {
            name.to_owned()
        } else {
            return Err(format!("unexpected argument `{arg}`\n{USAGE}"));
        };
        if package.is_empty() {
            return Err(format!("empty crate name\n{USAGE}"));
        }
        if EXCLUDED_PACKAGES.contains(&package.as_str()) {
            return Err(format!("`{package}` is excluded from coverage reports (build automation / dev-only fixtures)"));
        }
        if !opts.packages.contains(&package) {
            opts.packages.push(package);
        }
    }
    Ok(opts)
}

struct Outputs {
    dir: PathBuf,
    lcov: PathBuf,
    summary_json: PathBuf,
}

impl Outputs {
    fn in_dir(dir: &Path) -> Self {
        Self { dir: dir.to_path_buf(), lcov: dir.join("lcov.info"), summary_json: dir.join("summary.json") }
    }
}

fn plan(opts: &Options, nextest: bool, out: &Outputs) -> Vec<Vec<String>> {
    let s = |v: &str| v.to_owned();
    let path = |p: &Path| p.to_string_lossy().into_owned();
    let mut test_run = vec![s("llvm-cov"), s(if nextest { "nextest" } else { "--all-targets" }), s("--locked"), s("--no-report")];
    if opts.packages.is_empty() {
        test_run.push(s("--workspace"));
        EXCLUDED_PACKAGES.iter().for_each(|e| test_run.extend([s("--exclude"), s(e)]));
        if nextest && opts.runner_args.is_empty() {
            // Skip wall-clock performance micro-benchmarks whose millisecond budgets cannot hold
            // under LLVM coverage instrumentation (`-C instrument-coverage`).
            test_run.extend([
                s("-E"),
                format!(
                    "not ({} | test(=grid_view::tests::rv50_one_million_row_range_selection_and_brushing_completes_under_5ms_without_giant_alloc))",
                    crate::ci::PERFORMANCE_FILTER
                ),
            ]);
        }
    } else {
        opts.packages.iter().for_each(|p| test_run.extend([s("-p"), p.clone()]));
    }
    if !opts.runner_args.is_empty() {
        test_run.push(s("--"));
        test_run.extend(opts.runner_args.iter().cloned());
    }
    let rep = |extra: &[String]| {
        let mut cmd = vec![s("llvm-cov"), s("report"), s("--ignore-filename-regex"), s(IGNORE_FILENAME_REGEX)];
        cmd.extend_from_slice(extra);
        cmd
    };
    vec![
        vec![s("llvm-cov"), s("clean"), s("--workspace")],
        test_run,
        rep(&[s("--lcov"), s("--output-path"), path(&out.lcov)]),
        rep(&[s("--html"), s("--output-dir"), path(&out.dir)]),
        rep(&[s("--json"), s("--summary-only"), s("--output-path"), path(&out.summary_json)]),
    ]
}

fn summary_lines(doc: &serde_json::Value) -> Result<Vec<String>, String> {
    let totals = doc
        .get("data")
        .and_then(serde_json::Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|d| d.get("totals"))
        .ok_or_else(|| String::from("coverage summary has no `data[0].totals`"))?;
    let mut lines = Vec::new();
    for name in ["lines", "functions", "regions"] {
        let m = totals.get(name).ok_or_else(|| format!("coverage summary has no `totals.{name}`"))?;
        let (Some(cov), Some(cnt)) = (m.get("covered").and_then(serde_json::Value::as_u64), m.get("count").and_then(serde_json::Value::as_u64))
        else {
            return Err(format!("coverage summary `totals.{name}` lacks integer `covered`/`count`"));
        };
        let pct = if cnt == 0 { 0.0 } else { 100.0 * (cov as f64) / (cnt as f64) };
        lines.push(format!("{name:<9} {cov:>8} / {cnt:<8} covered ({pct:.1}%)"));
    }
    Ok(lines)
}

fn ensure_tools(root: &Path) -> Result<(), String> {
    let has_cov = crate::dev_tool_command("cargo")
        .current_dir(root)
        .args(["llvm-cov", "--version"])
        .wall_timeout(Duration::from_secs(30))
        .idle_timeout(Duration::from_secs(10))
        .run()
        .is_ok_and(|o| o.success);
    if !has_cov {
        return Err(format!("`cargo-llvm-cov` is not installed; {INSTALL_HINT}"));
    }
    if llvm_tools_available(root) {
        Ok(())
    } else {
        Err(format!("`llvm-cov` / `llvm-profdata` not found for the active toolchain; {LLVM_TOOLS_HINT}"))
    }
}

fn llvm_tools_available(root: &Path) -> bool {
    let env_tool = |var: &str| std::env::var_os(var).is_some_and(|p| Path::new(&p).is_file());
    if env_tool("LLVM_COV") && env_tool("LLVM_PROFDATA") {
        return true;
    }
    let rustc_out = |args: &[&str]| {
        crate::dev_tool_command("rustc")
            .current_dir(root)
            .args(args.iter().copied())
            .wall_timeout(Duration::from_secs(30))
            .idle_timeout(Duration::from_secs(10))
            .run()
            .ok()
            .filter(|o| o.success)
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let Some(sysroot) = rustc_out(&["--print", "sysroot"]) else { return false };
    let Some(host) = rustc_out(&["-vV"]).and_then(|t| t.lines().find_map(|l| l.strip_prefix("host: ").map(|h| h.trim().to_owned()))) else {
        return false;
    };
    let bin = Path::new(sysroot.trim()).join("lib/rustlib").join(host).join("bin");
    ["llvm-cov", "llvm-profdata"].into_iter().all(|n| bin.join(format!("{n}{}", std::env::consts::EXE_SUFFIX)).is_file())
}

fn coverage_output_dir(root: &Path) -> Result<PathBuf, String> {
    let meta = crate::cargo_metadata(root)?;
    let target =
        meta.get("target_directory").and_then(serde_json::Value::as_str).ok_or_else(|| String::from("cargo metadata has no `target_directory`"))?;
    Ok(PathBuf::from(target).join("llvm-cov"))
}

#[cfg(test)]
mod tests {
    use super::{EXCLUDED_PACKAGES, IGNORE_FILENAME_REGEX, Options, Outputs, parse_args, plan, summary_lines};
    use std::path::Path;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|&s| s.to_owned()).collect()
    }

    #[test]
    fn parses_args_builds_plan_and_formats_summary() {
        assert_eq!(
            parse_args(&strs(&["-p", "curvebook-stats", "--package=curvebook-fit", "-p", "curvebook-stats", "--", "anova", "-p"])),
            Ok(Options { packages: strs(&["curvebook-stats", "curvebook-fit"]), runner_args: strs(&["anova", "-p"]) })
        );
        assert_eq!(parse_args(&[]), Ok(Options::default()));
        for bad in [strs(&["-p"]), strs(&["--package="]), strs(&["--html"])] {
            assert!(parse_args(&bad).is_err());
        }
        for excluded in EXCLUDED_PACKAGES {
            assert!(parse_args(&strs(&["-p", excluded])).is_err());
        }
        let steps = plan(&Options::default(), false, &Outputs::in_dir(Path::new("/t/llvm-cov")));
        assert_eq!(
            (steps.len(), steps.first(), steps.get(1)),
            (
                5,
                Some(&strs(&["llvm-cov", "clean", "--workspace"])),
                Some(&strs(&[
                    "llvm-cov",
                    "--all-targets",
                    "--locked",
                    "--no-report",
                    "--workspace",
                    "--exclude",
                    "xtask",
                    "--exclude",
                    "curvebook-testkit"
                ]))
            )
        );
        assert!(IGNORE_FILENAME_REGEX.contains(r"[/\\]"));
        for r in steps.iter().skip(2) {
            assert_eq!(r.get(..4), Some(strs(&["llvm-cov", "report", "--ignore-filename-regex", IGNORE_FILENAME_REGEX]).as_slice()));
        }
        assert!(
            steps[2].ends_with(&strs(&["--lcov", "--output-path", "/t/llvm-cov/lcov.info"]))
                && steps[3].ends_with(&strs(&["--html", "--output-dir", "/t/llvm-cov"]))
                && steps[4].ends_with(&strs(&["--json", "--summary-only", "--output-path", "/t/llvm-cov/summary.json"]))
        );
        let p_steps = plan(&Options { packages: strs(&["curvebook-stats"]), runner_args: strs(&["anova"]) }, true, &Outputs::in_dir(Path::new("o")));
        assert_eq!(p_steps.get(1), Some(&strs(&["llvm-cov", "nextest", "--locked", "--no-report", "-p", "curvebook-stats", "--", "anova"])));
        let ws_nextest = plan(&Options::default(), true, &Outputs::in_dir(Path::new("o")));
        assert!(ws_nextest[1].windows(2).any(|w| w[0] == "-E" && w[1].contains(crate::ci::PERFORMANCE_FILTER)));

        let doc: serde_json::Value = serde_json::from_str(
            r#"{"data":[{"totals":{"lines":{"count":200,"covered":150},"functions":{"count":0,"covered":0},"regions":{"count":8,"covered":1}}}]}"#,
        )
        .unwrap();
        assert_eq!(
            summary_lines(&doc),
            Ok(strs(&[
                "lines          150 / 200      covered (75.0%)",
                "functions        0 / 0        covered (0.0%)",
                "regions          1 / 8        covered (12.5%)"
            ]))
        );
        for bad in [
            r#"{}"#,
            r#"{"data":[]}"#,
            r#"{"data":[{"totals":{"lines":{"count":1}}}]}"#,
            r#"{"data":[{"totals":{"lines":{"count":1.5,"covered":1},"functions":{"count":0,"covered":0},"regions":{"count":0,"covered":0}}}]}"#,
            r#"{"data":[{"totals":{"lines":{"count":1,"covered":-1},"functions":{"count":0,"covered":0},"regions":{"count":0,"covered":0}}}]}"#,
        ] {
            assert!(serde_json::from_str::<serde_json::Value>(bad).map_err(|e| e.to_string()).and_then(|d| summary_lines(&d)).is_err(), "{bad}");
        }
    }
}
