//! Headless command-line runner and stdio MCP server entry point for LumenCraft.
#![forbid(unsafe_code)]

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

const USAGE: &str = "\
usage: lumencraft-cli <command> [args]

commands:
  info <file> [--json]
                    inspect tables, info sheets, analyses, graphs and layouts in an archive
  commands [--group <group>] [--json]
                    list registered engine commands with schemas, effects and shortcuts
  <command-id> [--in <file>] [--out <file>] [--workdir <dir>] [--json] [--<param> <val> ...]
                    execute a registered engine command directly with schema-typed flags
  run <script> [--workdir <dir>] [--json]
                    execute a JSON-lines command stream headlessly
  batch <manifest> [--workdir <dir>] [--json]
                    execute a `lumencraft.batch.v1` JSON manifest headlessly
  convert <in> <out> [--format <fmt>] [--sheet <id>] [--dpi <u16>] [--pdf-version <1.4|1.7>] [--ttf-font <path>] [--json]
                    convert archives/unpacked `.lumencraft.d` dirs or export tables and figures (lumencraft, lumencraft.d, svg, pdf, emf, png, tiff, csv, tsv, json)
  export <in> (<out> | --out <out>) [--format <fmt>] [--sheet <id>] [--dpi <u16>] [--pdf-version <1.4|1.7>] [--ttf-font <path>] [--json]
                    export a table, graph, layout, or project to svg, pdf, emf, png, tiff, csv, tsv, json, or .lumencraft/.lumencraft.d
  bench <file> [--json]
                    measure parse, recalculation, compilation and export timings
  perf [--json]
                    run synthetic G12 performance checks
  mcp [--control-port <port>] [--gui-only] [--workdir <dir>]
                    run the Model Context Protocol (MCP) server over stdio

options:
  -V, --version     print version and exit
  -h, --help        print this help message and exit
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&args) {
        Ok(()) => ExitCode::from(EXIT_OK),
        Err(err) => {
            if wants_json_output(&args) {
                println!("{}", err.to_json_string());
            } else {
                eprintln!("lumencraft-cli: {err}");
            }
            ExitCode::from(err.exit_code())
        }
    }
}

fn wants_json_output(args: &[String]) -> bool {
    args.iter().take_while(|arg| *arg != "--").any(|arg| arg == "--json")
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
        Some("commands") => cmd_commands::run_commands(&args[1..]),
        Some("run") => cmd_run_batch::run_script(&args[1..]),
        Some("batch") => cmd_run_batch::run_batch(&args[1..]),
        Some("convert") => cmd_convert::run_convert(&args[1..]),
        Some("export") => cmd_convert::run_export(&args[1..]),
        Some("bench") => cmd_bench_perf::run_bench(&args[1..]),
        Some("perf") => cmd_bench_perf::run_perf(&args[1..]),
        Some("mcp") => run_mcp(&args[1..]),
        Some(other) => {
            if let Some(spec) = lumencraft_engine::cmd::registry().get(other) {
                return cmd_commands::run_direct_command(spec, &args[1..]);
            }
            Err(CliError::Usage(format!("unknown command `{other}`\n\n{USAGE}")))
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct McpCliConfig {
    control_port: Option<u16>,
    gui_only: bool,
    workdir: Option<PathBuf>,
}

/// In-process desktop UI host backing `lumencraft-cli mcp --gui-only`.
struct InProcessUiGuiHost {
    app: lumencraft_ui_egui::UiApp,
}

impl InProcessUiGuiHost {
    fn new(gui_only: bool) -> Self {
        let mut app = lumencraft_ui_egui::UiApp::new();
        app.set_gui_only(gui_only);
        Self { app }
    }
}

fn normalize_in_process_gui_params(method: &str, mut params: serde_json::Value) -> serde_json::Value {
    if let Some(obj) = params.as_object_mut() {
        match method {
            "ui.screenshot" => {
                if matches!(obj.get("target").and_then(serde_json::Value::as_str), Some("workspace" | "viewport")) {
                    obj.insert(String::from("target"), serde_json::Value::String(String::from("full_window")));
                }
            }
            "ui.pointer_gesture" => {
                if matches!(obj.get("action").and_then(serde_json::Value::as_str), Some("wheel")) {
                    obj.insert(String::from("action"), serde_json::Value::String(String::from("scroll")));
                }
            }
            "ui.invoke_menu" => {
                if let Some(arr) = obj.get("path").and_then(serde_json::Value::as_array) {
                    let segments: Vec<&str> = arr.iter().filter_map(serde_json::Value::as_str).map(str::trim).filter(|s| !s.is_empty()).collect();
                    if !segments.is_empty() {
                        obj.insert(String::from("path"), serde_json::Value::String(segments.join("/")));
                    }
                }
            }
            _ => {}
        }
    }
    params
}

impl lumencraft_mcp::GuiSessionHost for InProcessUiGuiHost {
    fn call_gui(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, lumencraft_engine::EngineError> {
        let normalized = normalize_in_process_gui_params(method, params);
        lumencraft_ui_egui::dispatch_gui_action(&mut self.app, method, normalized)
    }

    fn set_gui_only(&mut self, gui_only: bool) {
        self.app.set_gui_only(gui_only);
    }

    fn is_gui_only(&self) -> bool {
        self.app.is_gui_only()
    }
}

fn run_mcp(args: &[String]) -> Result<(), CliError> {
    let cfg = parse_mcp_args(args)?;
    let _ = lumencraft_mcp::SERVER_NAME;
    let mut state = match (cfg.control_port, cfg.gui_only) {
        (Some(port), true) => lumencraft_mcp::McpServerState::with_control_port_and_gui_only(port, true)
            .map_err(|err| CliError::Runtime(format!("MCP stdio server failed: {err}")))?,
        (Some(port), false) => {
            lumencraft_mcp::McpServerState::with_control_port(port).map_err(|err| CliError::Runtime(format!("MCP stdio server failed: {err}")))?
        }
        (None, true) => {
            let host = InProcessUiGuiHost::new(true);
            lumencraft_mcp::McpServerState::with_gui_host(Box::new(host)).with_gui_only(true)
        }
        (None, false) => lumencraft_mcp::McpServerState::new(),
    };
    if let Some(ref workdir) = cfg.workdir {
        let _ = sandbox::WorkdirGuard::new(workdir)?;
        state.set_workdir(Some(workdir)).map_err(|err| CliError::Runtime(format!("MCP workdir setup failed: {err}")))?;
    }
    lumencraft_mcp::run_stdio_with_state(&mut state).map_err(|err| CliError::Runtime(format!("MCP stdio server failed: {err}")))
}

fn parse_mcp_args(args: &[String]) -> Result<McpCliConfig, CliError> {
    const MCP_USAGE: &str = "usage: lumencraft-cli mcp [--control-port <1..=65535>] [--gui-only] [--workdir <dir>]";
    let mut cfg = McpCliConfig::default();
    let mut idx = 0usize;
    while idx < args.len() {
        let arg = args[idx].as_str();
        if let Some(raw) = arg.strip_prefix("--control-port=") {
            cfg.control_port = Some(parse_control_port_value(raw)?);
        } else if arg == "--control-port" {
            idx += 1;
            let raw = args.get(idx).ok_or_else(|| CliError::Usage(String::from(MCP_USAGE)))?;
            cfg.control_port = Some(parse_control_port_value(raw)?);
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
        } else if arg == "--gui-only" {
            cfg.gui_only = true;
        } else {
            return Err(CliError::Usage(String::from(MCP_USAGE)));
        }
        idx += 1;
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
        let err = dispatch(&[String::from("info"), String::from("/definitely-missing-lumencraft-archive-9f8e.lumencraft")]).err();
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
        assert_eq!(parse_mcp_args(&[]).unwrap(), super::McpCliConfig { control_port: None, gui_only: false, workdir: None });
        assert_eq!(
            parse_mcp_args(&[String::from("--control-port"), String::from("49152")]).unwrap(),
            super::McpCliConfig { control_port: Some(49152), gui_only: false, workdir: None }
        );
        assert_eq!(
            parse_mcp_args(&[String::from("--control-port=1")]).unwrap(),
            super::McpCliConfig { control_port: Some(1), gui_only: false, workdir: None }
        );
        assert_eq!(
            parse_mcp_args(&[String::from("--control-port=65535")]).unwrap(),
            super::McpCliConfig { control_port: Some(65535), gui_only: false, workdir: None }
        );
        assert_eq!(parse_mcp_args(&[String::from("--gui-only")]).unwrap(), super::McpCliConfig { control_port: None, gui_only: true, workdir: None });
        assert_eq!(
            parse_mcp_args(&[String::from("--control-port"), String::from("49152"), String::from("--gui-only")]).unwrap(),
            super::McpCliConfig { control_port: Some(49152), gui_only: true, workdir: None }
        );
        assert_eq!(
            parse_mcp_args(&[String::from("--gui-only"), String::from("--control-port=49153")]).unwrap(),
            super::McpCliConfig { control_port: Some(49153), gui_only: true, workdir: None }
        );
        assert_eq!(
            parse_mcp_args(&[String::from("--workdir"), String::from("/tmp/ws")]).unwrap(),
            super::McpCliConfig { control_port: None, gui_only: false, workdir: Some(std::path::PathBuf::from("/tmp/ws")) }
        );
        assert_eq!(
            parse_mcp_args(&[String::from("--gui-only"), String::from("--workdir=/tmp/ws"), String::from("--control-port=49154")]).unwrap(),
            super::McpCliConfig { control_port: Some(49154), gui_only: true, workdir: Some(std::path::PathBuf::from("/tmp/ws")) }
        );
        assert!(super::USAGE.contains("--gui-only"));
        assert!(super::USAGE.contains("mcp [--control-port <port>] [--gui-only] [--workdir <dir>]"));

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
    fn in_process_ui_gui_host_toggles_policy_and_dispatches_normalized_actions() {
        use lumencraft_mcp::GuiSessionHost;

        let mut host = super::InProcessUiGuiHost::new(true);
        assert!(host.is_gui_only());

        let inspect = host.call_gui("ui.inspect", serde_json::json!({"scope": "all"})).unwrap();
        assert_eq!(inspect["gui_only"], serde_json::json!(true));

        let shot = host.call_gui("ui.screenshot", serde_json::json!({"target": "workspace"})).unwrap();
        assert!(shot["width_px"].as_u64().is_some_and(|w| w > 0));
        assert!(shot["height_px"].as_u64().is_some_and(|h| h > 0));
        let png_b64 = shot["data_base64"].as_str().unwrap();
        assert!(png_b64.starts_with("iVBORw0KGgo"));

        host.set_gui_only(false);
        assert!(!host.is_gui_only());
        let inspect_relaxed = host.call_gui("ui.inspect", serde_json::json!({})).unwrap();
        assert_eq!(inspect_relaxed["gui_only"], serde_json::json!(false));
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
