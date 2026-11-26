//! Corpus verification and optional on-demand NIST StRD `.dat` fetcher (`docs/development.md`
//! §Verified NIST StRD corpus).
//!
//! Delegates NIST StRD verification and network download to `curvebook-testkit`'s
//! `fetch_corpus` example so `xtask` carries zero direct `curvebook-testkit` dependency.

use std::path::Path;
use std::time::Duration;

const CORPUS_CARGO_WALL_TIMEOUT: Duration = Duration::from_secs(600);
const CORPUS_CARGO_IDLE_TIMEOUT: Duration = Duration::from_secs(180);

const CORPUS_HELP: &str = "\
usage: cargo xtask corpus [--fetch | --fetch-only]

options:
  --fetch       download canonical public-domain NIST StRD .dat files into corpus/nist-strd/ and run verification suites
  --fetch-only  download canonical public-domain NIST StRD .dat files and write sha256sums.txt without running tests
  -h, --help    print this help message
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorpusOptions {
    pub fetch: bool,
    pub fetch_only: bool,
    pub help: bool,
}

pub fn parse_args(args: &[String]) -> Result<CorpusOptions, String> {
    let mut fetch = false;
    let mut fetch_only = false;
    let mut help = false;
    for arg in args {
        match arg.as_str() {
            "--fetch" => fetch = true,
            "--fetch-only" => fetch_only = true,
            "-h" | "--help" => help = true,
            other => return Err(format!("unknown `cargo xtask corpus` flag `{other}`\n\n{CORPUS_HELP}")),
        }
    }
    Ok(CorpusOptions { fetch, fetch_only, help })
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let opts = parse_args(args)?;
    if opts.help {
        print!("{CORPUS_HELP}");
        return Ok(());
    }

    let root_str = root.to_str().ok_or_else(|| format!("non-UTF-8 workspace root {}", root.display()))?;
    let mut example_args = vec!["--root", root_str];
    if opts.fetch || opts.fetch_only {
        example_args.push("--fetch");
    }
    let out = crate::run_workspace_example(root, "curvebook-testkit", "fetch_corpus", &example_args)?;
    if !out.is_empty() {
        print!("{}", String::from_utf8_lossy(&out));
    }
    if opts.fetch_only {
        return Ok(());
    }

    let nist_root = root.join("corpus").join("nist-strd");
    let require_corpus = nist_root.exists() || opts.fetch;
    run_cargo_corpus(root, &["test", "--locked", "-p", "curvebook-format", "-p", "curvebook-testkit"], require_corpus)?;
    run_cargo_corpus(root, &["test", "--locked", "-p", "curvebook-math", "--test", "nist_strd"], require_corpus)?;
    run_cargo_corpus(root, &["test", "--locked", "-p", "curvebook-stats", "--test", "nist_strd"], require_corpus)?;
    run_cargo_corpus(root, &["test", "--locked", "-p", "curvebook-fit", "--test", "nist_strd"], require_corpus)?;
    Ok(())
}

fn run_cargo_corpus(root: &Path, args: &[&str], require_corpus: bool) -> Result<(), String> {
    println!("+ cargo {}", args.join(" "));
    let mut cmd = crate::dev_tool_command("cargo")
        .current_dir(root)
        .args(args.iter().copied())
        .wall_timeout(CORPUS_CARGO_WALL_TIMEOUT)
        .idle_timeout(CORPUS_CARGO_IDLE_TIMEOUT);
    if require_corpus {
        cmd = cmd.env("CURVEBOOK_REQUIRE_CORPUS", "1");
    }
    let out = cmd.run().map_err(|e| format!("cargo {}: {e}", args.join(" ")))?;
    if !out.stdout.is_empty() {
        print!("{}", String::from_utf8_lossy(&out.stdout));
    }
    if !out.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
    }
    if !out.success {
        return Err(format!("`cargo {}` failed with exit code {:?}", args.join(" "), out.status_code));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{CorpusOptions, parse_args};

    #[test]
    fn parse_args_handles_default_fetch_help_and_unknown_flags() {
        assert_eq!(parse_args(&[]).unwrap(), CorpusOptions { fetch: false, fetch_only: false, help: false });
        assert_eq!(parse_args(&[String::from("--fetch")]).unwrap(), CorpusOptions { fetch: true, fetch_only: false, help: false });
        assert_eq!(parse_args(&[String::from("--fetch-only")]).unwrap(), CorpusOptions { fetch: false, fetch_only: true, help: false });
        assert_eq!(parse_args(&[String::from("--help")]).unwrap(), CorpusOptions { fetch: false, fetch_only: false, help: true });
        assert!(parse_args(&[String::from("--bogus")]).is_err());
    }
}
