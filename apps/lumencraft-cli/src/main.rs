//! Headless command-line runner and stdio MCP server entry point for LumenCraft.
#![forbid(unsafe_code)]

mod cmd_bench_perf;
mod cmd_convert;
mod cmd_info;
mod cmd_run_batch;
mod error;
mod sandbox;

use std::process::ExitCode;

use error::{CliError, EXIT_OK};

const USAGE: &str = "\
usage: lumencraft-cli <command> [args]

commands:
  info <file> [--json]
                    inspect tables, info sheets, analyses, graphs and layouts in an archive
  run <script> [--workdir <dir>] [--json]
                    execute a JSON-lines command stream headlessly
  batch <manifest> [--workdir <dir>] [--json]
                    execute a `lumencraft.batch.v1` JSON manifest headlessly
  convert <in> <out> [--sheet <id>] [--dpi <u16>] [--json]
                    convert archives or export tables and figures (lumencraft, svg, pdf, png, tiff, csv, tsv, json)
  bench <file> [--json]
                    measure parse, recalculation, compilation and export timings
  perf [--json]
                    run synthetic G12 performance checks
  mcp               run the Model Context Protocol (MCP) server over stdio

options:
  -V, --version     print version and exit
  -h, --help        print this help message and exit
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&args) {
        Ok(()) => ExitCode::from(EXIT_OK),
        Err(err) => {
            eprintln!("lumencraft-cli: {err}");
            ExitCode::from(err.exit_code())
        }
    }
}

fn dispatch(args: &[String]) -> Result<(), CliError> {
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help" | "help") => {
            print!("{USAGE}");
            Ok(())
        }
        Some("-V" | "--version" | "version") => {
            println!("lumencraft-cli {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("info") => cmd_info::run_info(&args[1..]),
        Some("run") => cmd_run_batch::run_script(&args[1..]),
        Some("batch") => cmd_run_batch::run_batch(&args[1..]),
        Some("convert") => cmd_convert::run_convert(&args[1..]),
        Some("bench") => cmd_bench_perf::run_bench(&args[1..]),
        Some("perf") => cmd_bench_perf::run_perf(&args[1..]),
        Some("mcp") => run_mcp(&args[1..]),
        Some(other) => Err(CliError::Usage(format!("unknown command `{other}`\n\n{USAGE}"))),
    }
}

fn run_mcp(args: &[String]) -> Result<(), CliError> {
    if !args.is_empty() {
        return Err(CliError::Usage(String::from("usage: lumencraft-cli mcp")));
    }
    let _ = lumencraft_mcp::SERVER_NAME;
    lumencraft_mcp::run_stdio().map_err(|err| CliError::Runtime(format!("MCP stdio server failed: {err}")))
}

#[cfg(test)]
mod tests {
    use super::dispatch;
    use crate::error::{CliError, EXIT_RUNTIME, EXIT_USAGE_OR_POLICY};
    use crate::sandbox::WorkdirGuard;

    #[test]
    fn cli_version_and_help_succeed() {
        assert!(dispatch(&[String::from("--version")]).is_ok());
        assert!(dispatch(&[String::from("--help")]).is_ok());
        assert!(dispatch(&[]).is_ok());
    }

    #[test]
    fn cli_rejects_unknown_subcommand_with_exit_2() {
        let err = dispatch(&[String::from("nonexistent")]).err();
        assert!(matches!(err, Some(CliError::Usage(_))));
        assert_eq!(err.map(|e| e.exit_code()), Some(EXIT_USAGE_OR_POLICY));
    }

    #[test]
    fn cli_runtime_error_uses_exit_1() {
        let err = dispatch(&[String::from("info"), String::from("/definitely-missing-lumencraft-archive-9f8e.lumencraft")]).err();
        assert!(matches!(err, Some(CliError::Runtime(_))));
        assert_eq!(err.map(|e| e.exit_code()), Some(EXIT_RUNTIME));
    }

    #[test]
    fn workdir_guard_blocks_parent_traversal_and_outside_paths() {
        let base = std::env::temp_dir().join(format!("lc_guard_unit_{}", std::process::id()));
        assert!(std::fs::create_dir_all(base.join("sub")).is_ok());
        let Ok(guard) = WorkdirGuard::new(&base) else {
            let _ = std::fs::remove_dir_all(&base);
            return;
        };

        assert!(guard.resolve("sub/output.svg").is_ok());
        assert!(matches!(guard.resolve("../outside.svg"), Err(CliError::PathEscape(_))));
        assert!(matches!(guard.resolve("sub/../../outside.svg"), Err(CliError::PathEscape(_))));
        assert!(matches!(guard.resolve("/etc/passwd"), Err(CliError::PathEscape(_))));

        let _ = std::fs::remove_dir_all(&base);
    }
}
