//! Headless command-line runner and stdio MCP server entry point for CurveBook.
#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

mod cli_catalog;
mod cmd_bench_perf;
mod cmd_commands;
mod cmd_convert;
mod cmd_info;
mod cmd_run_batch;
mod error;
mod sandbox;

use std::path::PathBuf;
use std::process::ExitCode;

use error::{CliError, EXIT_OK};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&args) {
        Ok(()) => ExitCode::from(EXIT_OK),
        Err(err) => {
            if wants_json_output(&args) {
                println!("{}", err.to_json_string());
            } else {
                eprintln!("curvebook-cli: {err}");
            }
            ExitCode::from(err.exit_code())
        }
    }
}

fn wants_json_output(args: &[String]) -> bool {
    args.iter().take_while(|arg| *arg != "--").any(|arg| arg == "--json")
}

fn wants_subcommand_help(args: &[String]) -> bool {
    args.len() == 1 && matches!(args.first().map(String::as_str), Some("-h" | "--help"))
}

fn dispatch(args: &[String]) -> Result<(), CliError> {
    cmd_convert::run_guarded_cli("cli.dispatch", || {
        let tail = args.get(1..).unwrap_or(&[]);
        match args.first().map(String::as_str) {
            None | Some("-h" | "--help") => {
                print!("{}", cli_catalog::format_usage());
                Ok(())
            }
            Some("help") => {
                if let Some(sub) = args.get(1)
                    && let Some(help) = cli_catalog::format_subcommand_help(sub)
                {
                    print!("{help}");
                    return Ok(());
                }
                print!("{}", cli_catalog::format_usage());
                Ok(())
            }
            Some("-V" | "--version" | "version") => {
                println!("curvebook-cli {}", env!("CARGO_PKG_VERSION"));
                Ok(())
            }
            Some(sub @ ("info" | "run" | "batch" | "convert" | "export" | "bench" | "perf" | "mcp")) if wants_subcommand_help(tail) => {
                if let Some(help) = cli_catalog::format_subcommand_help(sub) {
                    print!("{help}");
                } else {
                    print!("{}", cli_catalog::format_usage());
                }
                Ok(())
            }
            Some("info") => cmd_info::run_info(tail),
            Some("commands") => cmd_commands::run_commands(tail),
            Some("run") => cmd_run_batch::run_script(tail),
            Some("batch") => cmd_run_batch::run_batch(tail),
            Some("convert") => cmd_convert::run_convert(tail),
            Some("export") => cmd_convert::run_export(tail),
            Some("bench") => cmd_bench_perf::run_bench(tail),
            Some("perf") => cmd_bench_perf::run_perf(tail),
            Some("mcp") => run_mcp(tail),
            Some(other) => {
                if let Some(spec) = curvebook_engine::cmd::registry().get(other) {
                    return cmd_commands::run_direct_command(spec, tail);
                }
                let usage = cli_catalog::format_usage();
                Err(CliError::Usage(format!("unknown command `{other}`\n\n{usage}")))
            }
        }
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct McpCliConfig {
    control_port: Option<u16>,
    control_token: Option<String>,
    workdir: Option<PathBuf>,
}

fn run_mcp(args: &[String]) -> Result<(), CliError> {
    let cfg = parse_mcp_args_with_env(args, &|key| std::env::var_os(key))?;
    let _ = curvebook_mcp::SERVER_NAME;
    if let Some(ref workdir) = cfg.workdir {
        let _ = sandbox::WorkdirGuard::new(workdir)?;
    }
    let mut state = match cfg.control_port {
        Some(port) => {
            curvebook_mcp::McpServerState::with_control_port(port).map_err(|err| CliError::Runtime(format!("MCP stdio server failed: {err}")))?
        }
        None => curvebook_mcp::McpServerState::new(),
    };
    if let Some(ref token) = cfg.control_token {
        state.set_control_token(Some(token)).map_err(|err| CliError::Usage(format!("invalid --control-token: {err}")))?;
    }
    if let Some(ref workdir) = cfg.workdir {
        state.set_workdir(Some(workdir)).map_err(|err| CliError::Runtime(format!("MCP workdir setup failed: {err}")))?;
    }
    curvebook_mcp::run_stdio_with_state(&mut state).map_err(|err| CliError::Runtime(format!("MCP stdio server failed: {err}")))
}

#[cfg_attr(not(test), allow(dead_code))]
fn parse_mcp_args(args: &[String]) -> Result<McpCliConfig, CliError> {
    parse_mcp_args_with_env(args, &|_| None)
}

fn parse_mcp_args_with_env(args: &[String], env_lookup: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> Result<McpCliConfig, CliError> {
    const MCP_USAGE: &str = "usage: curvebook-cli mcp [--control-port <1..=65535>] [--workdir <dir>] [--control-token <token>]";
    let mut cfg = McpCliConfig::default();
    let mut idx = 0usize;
    while idx < args.len() {
        let Some(arg) = args.get(idx).map(String::as_str) else {
            break;
        };
        if let Some(raw) = arg.strip_prefix("--control-port=") {
            cfg.control_port = Some(parse_control_port_value(raw)?);
        } else if arg == "--control-port" {
            idx += 1;
            let raw = args.get(idx).ok_or_else(|| CliError::Usage(String::from(MCP_USAGE)))?;
            cfg.control_port = Some(parse_control_port_value(raw)?);
        } else if let Some(raw) = arg.strip_prefix("--control-token=") {
            let auth = curvebook_platform::ControlAuth::from_token(raw).map_err(|err| CliError::Usage(format!("invalid --control-token: {err}")))?;
            cfg.control_token = Some(auth.as_str().to_owned());
        } else if arg == "--control-token" {
            idx += 1;
            let raw = args.get(idx).ok_or_else(|| CliError::Usage(String::from(MCP_USAGE)))?;
            let auth = curvebook_platform::ControlAuth::from_token(raw).map_err(|err| CliError::Usage(format!("invalid --control-token: {err}")))?;
            cfg.control_token = Some(auth.as_str().to_owned());
        } else if let Some(raw) = arg.strip_prefix("--workdir=") {
            if raw.trim().is_empty() {
                return Err(CliError::Usage(String::from("--workdir= requires a non-empty directory path")));
            }
            cfg.workdir = Some(PathBuf::from(raw));
        } else if arg == "--workdir" {
            idx += 1;
            let raw = args.get(idx).ok_or_else(|| CliError::Usage(String::from("--workdir requires a directory path")))?;
            if raw.trim().is_empty() {
                return Err(CliError::Usage(String::from("--workdir requires a directory path")));
            }
            cfg.workdir = Some(PathBuf::from(raw));
        } else {
            return Err(CliError::Usage(String::from(MCP_USAGE)));
        }
        idx += 1;
    }
    if cfg.control_port.is_none()
        && let Some(raw_os) = env_lookup("CURVEBOOK_CONTROL_PORT")
    {
        let raw = raw_os
            .to_str()
            .ok_or_else(|| CliError::Usage(String::from("invalid CURVEBOOK_CONTROL_PORT: expected valid UTF-8 integer in 1..=65535")))?;
        if !raw.trim().is_empty() {
            cfg.control_port = Some(parse_control_port_value(raw).map_err(|err| CliError::Usage(format!("invalid CURVEBOOK_CONTROL_PORT: {err}")))?);
        }
    }
    if cfg.control_port.is_some() && cfg.workdir.is_some() {
        return Err(CliError::Usage(String::from(
            "cannot combine --control-port (or CURVEBOOK_CONTROL_PORT) with --workdir: `--workdir` cannot confine file operations on a live-attached desktop host",
        )));
    }
    Ok(cfg)
}

fn parse_control_port_value(raw: &str) -> Result<u16, CliError> {
    let trimmed = raw.trim();
    let port: u16 = trimmed.parse().map_err(|_| CliError::Usage(format!("invalid --control-port `{raw}` (expected integer in 1..=65535)")))?;
    if port == 0 {
        return Err(CliError::Usage(String::from("--control-port must be >= 1 (1..=65535)")));
    }
    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::{dispatch, parse_mcp_args, wants_json_output};
    use crate::error::{CliError, EXIT_INTERRUPTED, EXIT_RUNTIME, EXIT_USAGE_OR_POLICY};
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
        let err = dispatch(&[String::from("info"), String::from("/definitely-missing-curvebook-archive-9f8e.curvebook")]).err();
        assert!(matches!(err, Some(CliError::Runtime(_))));
        assert_eq!(err.map(|e| e.exit_code()), Some(EXIT_RUNTIME));
    }

    #[test]
    fn cli_commands_and_direct_dispatch_and_interrupted_exit_code() {
        assert!(dispatch(&[String::from("commands"), String::from("--group"), String::from("project"), String::from("--json")]).is_ok());
        assert!(dispatch(&[String::from("project.inspect"), String::from("--json")]).is_ok());
        assert!(wants_json_output(&[String::from("info"), String::from("x"), String::from("--json")]));
        assert!(!wants_json_output(&[String::from("info"), String::from("--"), String::from("--json")]));
        assert_eq!(CliError::Interrupted(String::from("SIGINT")).exit_code(), EXIT_INTERRUPTED);
    }

    #[test]
    fn mcp_control_port_validation_rejects_zero_overflow_and_unknown_flags() {
        assert_eq!(parse_mcp_args(&[]).unwrap(), super::McpCliConfig { control_port: None, workdir: None, control_token: None });
        assert_eq!(
            parse_mcp_args(&[String::from("--control-port"), String::from("49152")]).unwrap(),
            super::McpCliConfig { control_port: Some(49152), workdir: None, control_token: None }
        );
        assert_eq!(
            parse_mcp_args(&[String::from("--control-port=1")]).unwrap(),
            super::McpCliConfig { control_port: Some(1), workdir: None, control_token: None }
        );
        assert_eq!(
            parse_mcp_args(&[String::from("--control-port=65535")]).unwrap(),
            super::McpCliConfig { control_port: Some(65535), workdir: None, control_token: None }
        );
        assert_eq!(
            parse_mcp_args(&[String::from("--workdir"), String::from("/tmp/ws")]).unwrap(),
            super::McpCliConfig { control_port: None, workdir: Some(std::path::PathBuf::from("/tmp/ws")), control_token: None }
        );
        let combined_flags_err = parse_mcp_args(&[String::from("--workdir=/tmp/ws"), String::from("--control-port=49154")]).unwrap_err();
        assert!(matches!(combined_flags_err, CliError::Usage(ref msg) if msg.contains("--workdir") && msg.contains("--control-port")));
        let combined_env_err = super::parse_mcp_args_with_env(&[String::from("--workdir=/tmp/ws")], &|key| {
            if key == "CURVEBOOK_CONTROL_PORT" { Some(std::ffi::OsString::from("49154")) } else { None }
        })
        .unwrap_err();
        assert!(matches!(combined_env_err, CliError::Usage(ref msg) if msg.contains("--workdir") && msg.contains("--control-port")));
        assert_eq!(
            parse_mcp_args(&[String::from("--control-token"), String::from("secret-tok")]).unwrap(),
            super::McpCliConfig { control_port: None, workdir: None, control_token: Some(String::from("secret-tok")) }
        );
        assert_eq!(
            parse_mcp_args(&[String::from("--control-port=49155"), String::from("--control-token=tok-2")]).unwrap(),
            super::McpCliConfig { control_port: Some(49155), workdir: None, control_token: Some(String::from("tok-2")) }
        );
        let usage = super::cli_catalog::format_usage();
        assert!(!usage.contains("--gui-only"));
        assert!(usage.contains("mcp [--control-port <port>] [--workdir <dir>]"));
        assert!(usage.contains("--control-token <token>"));
        assert!(usage.contains(super::cli_catalog::CLI_DOCS_URL));
        for cmd in super::cli_catalog::CLI_COMMANDS {
            let sub_help = super::cli_catalog::format_subcommand_help(cmd.id).unwrap();
            assert!(sub_help.contains(&format!("#cli-{}", cmd.id)));
            assert!(dispatch(&[String::from(cmd.id), String::from("--help")]).is_ok() || cmd.id == "direct-command");
        }
        assert!(dispatch(&[String::from("help"), String::from("mcp")]).is_ok());

        for bad in [
            vec!["--control-port", "0"],
            vec!["--control-port=0"],
            vec!["--control-port", "65536"],
            vec!["--control-port", "-1"],
            vec!["--control-port", "abc"],
            vec!["--control-port"],
            vec!["--workdir"],
            vec!["--workdir="],
            vec!["--workdir", "   "],
            vec!["--control-token"],
            vec!["--control-token="],
            vec!["--control-token", "   "],
            vec!["--control-token", "has space"],
            vec!["--gui-only"],
            vec!["--gui-only=true"],
            vec!["--unknown"],
            vec!["extra-arg"],
        ] {
            let owned: Vec<String> = bad.into_iter().map(String::from).collect();
            let err = parse_mcp_args(&owned).unwrap_err();
            assert!(matches!(err, CliError::Usage(_)), "expected Usage for {owned:?}, got {err:?}");
            assert_eq!(err.exit_code(), EXIT_USAGE_OR_POLICY);
        }
    }

    #[test]
    fn r1_s12_09_mcp_args_fall_back_to_env_control_port_with_cli_precedence() {
        use std::ffi::OsString;

        let from_env = super::parse_mcp_args_with_env(&[], &|k| {
            if k == "CURVEBOOK_CONTROL_PORT" { Some(OsString::from("49201")) } else { None }
        })
        .unwrap();
        assert_eq!(from_env.control_port, Some(49201));

        let cli_wins = super::parse_mcp_args_with_env(&[String::from("--control-port=49202")], &|k| {
            if k == "CURVEBOOK_CONTROL_PORT" { Some(OsString::from("49201")) } else { None }
        })
        .unwrap();
        assert_eq!(cli_wins.control_port, Some(49202));

        let empty_env = super::parse_mcp_args_with_env(&[], &|k| {
            if k == "CURVEBOOK_CONTROL_PORT" { Some(OsString::from("   ")) } else { None }
        })
        .unwrap();
        assert_eq!(empty_env.control_port, None);

        for bad in ["0", "abc", "70000"] {
            let err = super::parse_mcp_args_with_env(&[], &|k| {
                if k == "CURVEBOOK_CONTROL_PORT" { Some(OsString::from(bad)) } else { None }
            })
            .unwrap_err();
            assert!(matches!(err, CliError::Usage(_)), "expected Usage for `{bad}`, got {err:?}");
        }
    }

    #[test]
    fn workdir_guard_blocks_parent_traversal_and_outside_paths() {
        let base = std::env::temp_dir().join(format!("cb_guard_unit_{}", std::process::id()));
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
