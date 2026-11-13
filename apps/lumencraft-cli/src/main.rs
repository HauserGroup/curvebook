//! Headless command-line runner and stdio MCP server entry point for LumenCraft.
#![forbid(unsafe_code)]

use std::process::ExitCode;

const USAGE: &str = "\
usage: lumencraft-cli <command> [args]

commands:
  info <file>       inspect tables, analyses, graphs and layouts in a project archive
  run <script>      execute a batch of engine commands headlessly
  convert <in> <out>
                    export tables or figures (CSV, TSV, SVG, PDF, PNG)
  bench <file>      measure parse, recalculation and figure render timings
  perf              verify synthetic performance budgets
  mcp               run the Model Context Protocol (MCP) server over stdio

options:
  -V, --version     print version and exit
  -h, --help        print this help message and exit
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("lumencraft-cli: {err}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help" | "help") => {
            print!("{USAGE}");
            Ok(())
        }
        Some("-V" | "--version" | "version") => {
            println!("lumencraft-cli {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(cmd @ ("info" | "run" | "convert" | "bench" | "perf" | "mcp")) => {
            let _ = lumencraft_mcp::SERVER_NAME;
            Err(format!("subcommand `{cmd}` is not implemented yet (workspace infrastructure skeleton)"))
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    }
}

#[cfg(test)]
mod tests {
    use super::dispatch;

    #[test]
    fn cli_version_and_help_succeed() {
        assert!(dispatch(&[String::from("--version")]).is_ok());
        assert!(dispatch(&[String::from("--help")]).is_ok());
        assert!(dispatch(&[]).is_ok());
    }

    #[test]
    fn cli_rejects_unknown_subcommand() {
        assert!(dispatch(&[String::from("nonexistent")]).is_err());
    }
}
