//! `cargo xtask coverage`: local source-coverage reports through `cargo-llvm-cov` (`R0.16`).
//!
//! Coverage is a diagnostic for finding untested edge-case and error paths. It is never a
//! substitute for `D1` oracle checks, has no percentage gate, and its numbers are never copied
//! into hand-written docs. Reports stay on disk (lcov + HTML); nothing is uploaded anywhere.

use std::path::{Path, PathBuf};
use std::time::Duration;

const USAGE: &str = "usage: cargo xtask coverage [-p <crate>]... [-- <test-runner args>]";

/// Packages left out of both the instrumented test run and the report.
const EXCLUDED_PACKAGES: &[&str] = &["xtask", "curvebook-testkit"];

/// Source files dropped from every report (build automation and dev-only fixtures).
const IGNORE_FILENAME_REGEX: &str = r"(^|[/\\])(xtask|crates[/\\]testkit)[/\\]";

const INSTALL_HINT: &str = "install it with `cargo install --locked cargo-llvm-cov`";
const LLVM_TOOLS_HINT: &str = "run `rustup component add llvm-tools-preview`, or, for a toolchain without that component \
     (for example Homebrew Rust), set `LLVM_COV` and `LLVM_PROFDATA` to the `llvm-cov` / `llvm-profdata` of an LLVM \
     whose major version matches `rustc -vV`";

#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    packages: Vec<String>,
    runner_args: Vec<String>,
}

/// Execute `cargo xtask coverage`.
pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    if args.iter().take_while(|a| a.as_str() != "--").any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return Ok(());
    }
    let opts = parse_args(args)?;
    ensure_tools(root)?;

    let out = Outputs::in_dir(&coverage_output_dir(root)?);
    let nextest = crate::has_cargo_nextest();

    for step in plan(&opts, nextest, &out) {
        // `llvm-cov clean` may remove the report directory, and `report` does not create it.
        std::fs::create_dir_all(&out.dir).map_err(|e| format!("{}: {e}", out.dir.display()))?;
        crate::run_cargo(root, &step)?;
    }

    let summary = std::fs::read_to_string(&out.summary_json).map_err(|e| format!("{}: {e}", out.summary_json.display()))?;
    let summary_doc: serde_json::Value = serde_json::from_str(&summary).map_err(|e| format!("{}: {e}", out.summary_json.display()))?;
    for line in summary_lines(&summary_doc)? {
        println!("coverage: {line}");
    }
    println!("coverage: lcov report  {}", out.lcov.display());
    println!("coverage: HTML report  {}", out.html_index().display());
    println!("coverage: diagnostic only (no threshold); use it to find untested edge-case and error paths");
    Ok(())
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut opts = Options::default();
    let mut iter = args.iter();
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

/// Report files written under `<cargo target dir>/llvm-cov`.
#[derive(Debug)]
struct Outputs {
    /// Passed to `report --html --output-dir`; `cargo-llvm-cov` writes the site into `<dir>/html`.
    dir: PathBuf,
    lcov: PathBuf,
    summary_json: PathBuf,
}

impl Outputs {
    fn in_dir(dir: &Path) -> Self {
        Self { dir: dir.to_path_buf(), lcov: dir.join("lcov.info"), summary_json: dir.join("summary.json") }
    }

    fn html_index(&self) -> PathBuf {
        self.dir.join("html").join("index.html")
    }
}

/// Build the ordered `cargo` invocations: clean stale profiles, run the instrumented tests once,
/// then render lcov, HTML, and a JSON summary from the same profile data.
fn plan(opts: &Options, nextest: bool, out: &Outputs) -> Vec<Vec<String>> {
    let s = |v: &str| v.to_owned();
    let path = |p: &Path| p.to_string_lossy().into_owned();

    let mut test_run = vec![s("llvm-cov")];
    if nextest {
        test_run.push(s("nextest"));
    } else {
        test_run.push(s("--all-targets"));
    }
    test_run.push(s("--locked"));
    test_run.push(s("--no-report"));
    if opts.packages.is_empty() {
        test_run.push(s("--workspace"));
        for excluded in EXCLUDED_PACKAGES {
            test_run.extend([s("--exclude"), s(excluded)]);
        }
    } else {
        for package in &opts.packages {
            test_run.extend([s("-p"), package.clone()]);
        }
    }
    if !opts.runner_args.is_empty() {
        test_run.push(s("--"));
        test_run.extend(opts.runner_args.iter().cloned());
    }

    let report = |extra: Vec<String>| {
        let mut cmd = vec![s("llvm-cov"), s("report"), s("--ignore-filename-regex"), s(IGNORE_FILENAME_REGEX)];
        cmd.extend(extra);
        cmd
    };

    vec![
        vec![s("llvm-cov"), s("clean"), s("--workspace")],
        test_run,
        report(vec![s("--lcov"), s("--output-path"), path(&out.lcov)]),
        report(vec![s("--html"), s("--output-dir"), path(&out.dir)]),
        report(vec![s("--json"), s("--summary-only"), s("--output-path"), path(&out.summary_json)]),
    ]
}

/// Workspace totals from an `llvm-cov export -summary-only` JSON document
/// (`data[0].totals.{lines,functions,regions}.{covered,count}`).
fn summary_lines(doc: &serde_json::Value) -> Result<Vec<String>, String> {
    let totals = doc
        .get("data")
        .and_then(serde_json::Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|d| d.get("totals"))
        .ok_or_else(|| String::from("coverage summary has no `data[0].totals`"))?;
    let number = |metric: &serde_json::Value, key: &str| metric.get(key).and_then(serde_json::Value::as_u64);
    let mut lines = Vec::new();
    for name in ["lines", "functions", "regions"] {
        let metric = totals.get(name).ok_or_else(|| format!("coverage summary has no `totals.{name}`"))?;
        let (Some(covered), Some(count)) = (number(metric, "covered"), number(metric, "count")) else {
            return Err(format!("coverage summary `totals.{name}` lacks integer `covered`/`count`"));
        };
        let pct = if count == 0 { 0.0 } else { 100.0 * (covered as f64) / (count as f64) };
        lines.push(format!("{name:<9} {covered:>8} / {count:<8} covered ({pct:.1}%)"));
    }
    Ok(lines)
}

fn ensure_tools(root: &Path) -> Result<(), String> {
    let has_llvm_cov = crate::dev_tool_command("cargo")
        .current_dir(root)
        .args(["llvm-cov", "--version"])
        .wall_timeout(Duration::from_secs(30))
        .idle_timeout(Duration::from_secs(10))
        .run()
        .is_ok_and(|o| o.success);
    if !has_llvm_cov {
        return Err(format!("`cargo-llvm-cov` is not installed; {INSTALL_HINT}"));
    }
    if llvm_tools_available(root) {
        Ok(())
    } else {
        Err(format!("`llvm-cov` / `llvm-profdata` not found for the active toolchain; {LLVM_TOOLS_HINT}"))
    }
}

/// `LLVM_COV` + `LLVM_PROFDATA` overrides, or the rustup `llvm-tools-preview` binaries in the
/// active sysroot (`<sysroot>/lib/rustlib/<host>/bin`), which is where `cargo-llvm-cov` looks.
fn llvm_tools_available(root: &Path) -> bool {
    let env_tool = |var: &str| std::env::var_os(var).is_some_and(|p| Path::new(&p).is_file());
    if env_tool("LLVM_COV") && env_tool("LLVM_PROFDATA") {
        return true;
    }
    let Some(sysroot) = rustc_output(root, &["--print", "sysroot"]) else {
        return false;
    };
    let Some(host) = rustc_output(root, &["-vV"]).and_then(|text| text.lines().find_map(|l| l.strip_prefix("host: ").map(|h| h.trim().to_owned())))
    else {
        return false;
    };
    let bin = Path::new(sysroot.trim()).join("lib").join("rustlib").join(host).join("bin");
    let exe = |name: &str| bin.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)).is_file();
    exe("llvm-cov") && exe("llvm-profdata")
}

fn rustc_output(root: &Path, args: &[&str]) -> Option<String> {
    let out = crate::dev_tool_command("rustc")
        .current_dir(root)
        .args(args.iter().copied())
        .wall_timeout(Duration::from_secs(30))
        .idle_timeout(Duration::from_secs(10))
        .run()
        .ok()?;
    out.success.then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `<cargo target dir>/llvm-cov`, honouring `CARGO_TARGET_DIR` so parallel agents stay isolated.
/// `cargo-llvm-cov` keeps its instrumented build in a sibling `llvm-cov-target` directory.
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
    fn parse_accepts_packages_and_runner_args() {
        let opts = parse_args(&strs(&["-p", "curvebook-stats", "--package=curvebook-fit", "-p", "curvebook-stats", "--", "anova", "-p"]));
        assert_eq!(opts, Ok(Options { packages: strs(&["curvebook-stats", "curvebook-fit"]), runner_args: strs(&["anova", "-p"]) }));
        assert_eq!(parse_args(&[]), Ok(Options::default()));
    }

    #[test]
    fn parse_rejects_bad_input() {
        assert!(parse_args(&strs(&["-p"])).is_err());
        assert!(parse_args(&strs(&["--package="])).is_err());
        assert!(parse_args(&strs(&["--html"])).is_err());
        for excluded in EXCLUDED_PACKAGES {
            assert!(parse_args(&strs(&["-p", excluded])).is_err(), "{excluded} must be rejected");
        }
    }

    #[test]
    fn workspace_plan_excludes_tooling_crates_and_reports_from_one_run() {
        let steps = plan(&Options::default(), false, &Outputs::in_dir(Path::new("/t/llvm-cov")));
        assert_eq!(steps.len(), 5);
        assert_eq!(steps.first(), Some(&strs(&["llvm-cov", "clean", "--workspace"])));
        assert_eq!(
            steps.get(1),
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
        );
        assert!(IGNORE_FILENAME_REGEX.contains(r"[/\\]"));
        for report in steps.iter().skip(2) {
            assert_eq!(report.get(..4), Some(strs(&["llvm-cov", "report", "--ignore-filename-regex", IGNORE_FILENAME_REGEX]).as_slice()));
        }
        assert!(steps.get(2).is_some_and(|s| s.ends_with(&strs(&["--lcov", "--output-path", "/t/llvm-cov/lcov.info"]))));
        assert!(steps.get(3).is_some_and(|s| s.ends_with(&strs(&["--html", "--output-dir", "/t/llvm-cov"]))));
        assert!(steps.get(4).is_some_and(|s| s.ends_with(&strs(&["--json", "--summary-only", "--output-path", "/t/llvm-cov/summary.json"]))));
    }

    #[test]
    fn package_plan_uses_nextest_and_forwards_runner_args() {
        let opts = Options { packages: strs(&["curvebook-stats"]), runner_args: strs(&["anova"]) };
        let steps = plan(&opts, true, &Outputs::in_dir(Path::new("o")));
        assert_eq!(steps.get(1), Some(&strs(&["llvm-cov", "nextest", "--locked", "--no-report", "-p", "curvebook-stats", "--", "anova"])));
    }

    #[test]
    fn summary_reads_workspace_totals_and_rejects_malformed_documents() {
        let doc: serde_json::Value = serde_json::from_str(
            r#"{"data":[{"totals":{"lines":{"count":200,"covered":150,"percent":75.0},
                "functions":{"count":0,"covered":0,"percent":0},
                "regions":{"count":8,"covered":1,"percent":12.5}}}],"type":"llvm.coverage.json.export"}"#,
        )
        .unwrap();
        let lines = summary_lines(&doc);
        assert_eq!(
            lines,
            Ok(strs(&[
                "lines          150 / 200      covered (75.0%)",
                "functions        0 / 0        covered (0.0%)",
                "regions          1 / 8        covered (12.5%)",
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
