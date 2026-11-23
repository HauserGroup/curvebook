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
    gui_only: bool,
    workdir: Option<PathBuf>,
}

/// In-process desktop UI host backing `curvebook-cli mcp --gui-only`.
struct InProcessUiGuiHost {
    app: curvebook_ui_egui::UiApp,
}

impl InProcessUiGuiHost {
    fn new(gui_only: bool) -> Self {
        let mut app = curvebook_ui_egui::UiApp::new();
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

impl curvebook_mcp::GuiSessionHost for InProcessUiGuiHost {
    fn call_gui(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, curvebook_engine::EngineError> {
        let normalized = normalize_in_process_gui_params(method, params);
        curvebook_ui_egui::dispatch_gui_action(&mut self.app, method, normalized)
    }

    fn set_gui_only(&mut self, gui_only: bool) {
        self.app.set_gui_only(gui_only);
    }

    fn is_gui_only(&self) -> bool {
        self.app.is_gui_only()
    }

    fn hosted_session(&self) -> Option<&curvebook_engine::Session> {
        Some(self.app.session())
    }

    fn hosted_session_mut(&mut self) -> Option<&mut curvebook_engine::Session> {
        Some(self.app.session_mut())
    }

    fn sync_after_session_mutation(&mut self) {
        self.app.sync_views();
    }
}

fn run_mcp(args: &[String]) -> Result<(), CliError> {
    let cfg = parse_mcp_args_with_env(args, &|key| std::env::var_os(key))?;
    let _ = curvebook_mcp::SERVER_NAME;
    if let Some(ref workdir) = cfg.workdir {
        let _ = sandbox::WorkdirGuard::new(workdir)?;
    }
    let mut state = match (cfg.control_port, cfg.gui_only) {
        (Some(port), true) => curvebook_mcp::McpServerState::with_control_port_and_gui_only(port, true)
            .map_err(|err| CliError::Runtime(format!("MCP stdio server failed: {err}")))?,
        (Some(port), false) => {
            curvebook_mcp::McpServerState::with_control_port(port).map_err(|err| CliError::Runtime(format!("MCP stdio server failed: {err}")))?
        }
        (None, true) => {
            let mut host = InProcessUiGuiHost::new(true);
            if let Some(ref workdir) = cfg.workdir {
                let policy = curvebook_engine::ExternalPathPolicy::workdir(workdir)
                    .map_err(|err| CliError::Runtime(format!("MCP workdir setup failed: {err}")))?;
                host.app.session_mut().set_path_policy(policy);
            }
            curvebook_mcp::McpServerState::with_gui_host(Box::new(host)).with_gui_only(true)
        }
        (None, false) => curvebook_mcp::McpServerState::new(),
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
    const MCP_USAGE: &str = "usage: curvebook-cli mcp [--control-port <1..=65535>] [--gui-only] [--workdir <dir>] [--control-token <token>]";
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
            let auth = curvebook_ui_egui::ControlAuth::from_token(raw).map_err(|err| CliError::Usage(format!("invalid --control-token: {err}")))?;
            cfg.control_token = Some(auth.as_str().to_owned());
        } else if arg == "--control-token" {
            idx += 1;
            let raw = args.get(idx).ok_or_else(|| CliError::Usage(String::from(MCP_USAGE)))?;
            let auth = curvebook_ui_egui::ControlAuth::from_token(raw).map_err(|err| CliError::Usage(format!("invalid --control-token: {err}")))?;
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
        } else if arg == "--gui-only" {
            cfg.gui_only = true;
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
        assert_eq!(parse_mcp_args(&[]).unwrap(), super::McpCliConfig { control_port: None, gui_only: false, workdir: None, control_token: None });
        assert_eq!(
            parse_mcp_args(&[String::from("--control-port"), String::from("49152")]).unwrap(),
            super::McpCliConfig { control_port: Some(49152), gui_only: false, workdir: None, control_token: None }
        );
        assert_eq!(
            parse_mcp_args(&[String::from("--control-port=1")]).unwrap(),
            super::McpCliConfig { control_port: Some(1), gui_only: false, workdir: None, control_token: None }
        );
        assert_eq!(
            parse_mcp_args(&[String::from("--control-port=65535")]).unwrap(),
            super::McpCliConfig { control_port: Some(65535), gui_only: false, workdir: None, control_token: None }
        );
        assert_eq!(
            parse_mcp_args(&[String::from("--gui-only")]).unwrap(),
            super::McpCliConfig { control_port: None, gui_only: true, workdir: None, control_token: None }
        );
        assert_eq!(
            parse_mcp_args(&[String::from("--control-port"), String::from("49152"), String::from("--gui-only")]).unwrap(),
            super::McpCliConfig { control_port: Some(49152), gui_only: true, workdir: None, control_token: None }
        );
        assert_eq!(
            parse_mcp_args(&[String::from("--gui-only"), String::from("--control-port=49153")]).unwrap(),
            super::McpCliConfig { control_port: Some(49153), gui_only: true, workdir: None, control_token: None }
        );
        assert_eq!(
            parse_mcp_args(&[String::from("--workdir"), String::from("/tmp/ws")]).unwrap(),
            super::McpCliConfig { control_port: None, gui_only: false, workdir: Some(std::path::PathBuf::from("/tmp/ws")), control_token: None }
        );
        let combined_flags_err =
            parse_mcp_args(&[String::from("--gui-only"), String::from("--workdir=/tmp/ws"), String::from("--control-port=49154")]).unwrap_err();
        assert!(matches!(combined_flags_err, CliError::Usage(ref msg) if msg.contains("--workdir") && msg.contains("--control-port")));
        let combined_env_err = super::parse_mcp_args_with_env(&[String::from("--workdir=/tmp/ws")], &|key| {
            if key == "CURVEBOOK_CONTROL_PORT" { Some(std::ffi::OsString::from("49154")) } else { None }
        })
        .unwrap_err();
        assert!(matches!(combined_env_err, CliError::Usage(ref msg) if msg.contains("--workdir") && msg.contains("--control-port")));
        assert_eq!(
            parse_mcp_args(&[String::from("--control-token"), String::from("secret-tok")]).unwrap(),
            super::McpCliConfig { control_port: None, gui_only: false, workdir: None, control_token: Some(String::from("secret-tok")) }
        );
        assert_eq!(
            parse_mcp_args(&[String::from("--control-port=49155"), String::from("--control-token=tok-2")]).unwrap(),
            super::McpCliConfig { control_port: Some(49155), gui_only: false, workdir: None, control_token: Some(String::from("tok-2")) }
        );
        let usage = super::cli_catalog::format_usage();
        assert!(usage.contains("--gui-only"));
        assert!(usage.contains("mcp [--control-port <port>] [--gui-only] [--workdir <dir>]"));
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
    fn in_process_ui_gui_host_toggles_policy_and_dispatches_normalized_actions() {
        use curvebook_mcp::GuiSessionHost;

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
    fn r2_s12_07_09_in_process_ui_gui_host_shares_session_and_preserves_wheel() -> Result<(), String> {
        use curvebook_mcp::{McpServerState, call_tool, dispatch_json_value};
        use serde_json::json;

        let mut state = McpServerState::with_gui_host(Box::new(super::InProcessUiGuiHost::new(false)));
        assert!(!state.is_gui_only());

        // Core `command_run` mutates the hosted UiApp session and syncs views.
        let created = call_tool(
            &mut state,
            "command_run",
            json!({ "command_id": "sheet.createData", "params": { "sheet_id": "shared-s1", "kind": "Column" } }),
        )
        .map_err(|e| format!("command_run failed: {e}"))?;
        assert!(!created.is_error);

        let inspect = call_tool(&mut state, "ui_inspect", json!({ "scope": "navigator" })).map_err(|e| format!("ui_inspect failed: {e}"))?;
        assert!(!inspect.is_error);
        let nav_json = inspect.structured_content.ok_or_else(|| String::from("missing ui_inspect structured content"))?;
        let sheets = nav_json["navigator_sheets"].as_array().ok_or_else(|| format!("missing navigator_sheets: {nav_json}"))?;
        assert!(sheets.iter().any(|s| s["sheet_id"] == json!("shared-s1")), "shared-s1 missing from navigator: {nav_json}");

        // `resources/list` and `resources/read` read the same hosted UiApp session.
        let res_list = dispatch_json_value(json!({ "jsonrpc": "2.0", "id": 1, "method": "resources/list", "params": {} }), &mut state)
            .ok_or_else(|| String::from("missing resources/list reply"))?;
        let res_arr = res_list["result"]["resources"].as_array().ok_or_else(|| format!("missing resources array: {res_list}"))?;
        assert!(res_arr.iter().any(|r| r["uri"] == json!("sheet://shared-s1")), "resources/list missed hosted sheet: {res_list}");

        let res_read = dispatch_json_value(
            json!({ "jsonrpc": "2.0", "id": 2, "method": "resources/read", "params": { "uri": "sheet://shared-s1" } }),
            &mut state,
        )
        .ok_or_else(|| String::from("missing resources/read reply"))?;
        assert!(res_read.get("error").is_none(), "resources/read error: {res_read}");
        assert!(!res_read["result"]["contents"].as_array().is_none_or(Vec::is_empty));

        // `wheel` action is preserved verbatim (`"wheel"`, not rewritten to `"scroll"`).
        let wheel = call_tool(&mut state, "ui_pointer_gesture", json!({ "action": "wheel", "x": 200.0, "y": 200.0, "delta_y": -120.0 }))
            .map_err(|e| format!("wheel gesture failed: {e}"))?;
        assert!(!wheel.is_error, "wheel gesture errored: {:?}", wheel.structured_content);
        let wheel_sc = wheel.structured_content.ok_or_else(|| String::from("missing wheel structured content"))?;
        assert_eq!(wheel_sc["action"], json!("wheel"));
        Ok(())
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
