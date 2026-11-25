//! End-to-end parity tests for `curvebook-cli mcp --control-port` live desktop attachment.

use std::io::{BufReader, ErrorKind};
use std::net::TcpListener;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use curvebook_mcp::McpServerState;
use curvebook_ui_egui::UiApp;
use serde_json::{Value, json};

use super::common::{CliHarness, loopback_required};

#[test]
fn cli_help_and_mcp_flag_validation() -> Result<(), Box<dyn std::error::Error>> {
    let bin = CliHarness::bin();

    let help = Command::new(bin).arg("--help").output()?;
    assert!(help.status.success());
    let help_text = String::from_utf8(help.stdout)?;
    assert!(help_text.contains("mcp [--control-port <port>] [--workdir <dir>]"), "unexpected --help:\n{help_text}");
    assert!(!help_text.contains("--gui-only"), "removed --gui-only flag must not appear in --help:\n{help_text}");

    let bad_port = Command::new(bin).args(["mcp", "--control-port", "0"]).output()?;
    assert_eq!(bad_port.status.code(), Some(2));

    let removed_flag = Command::new(bin).args(["mcp", "--gui-only"]).output()?;
    assert_eq!(removed_flag.status.code(), Some(2));
    Ok(())
}

fn verify_live_attach_end_to_end(state: &mut McpServerState, workdir: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let call = |st: &mut McpServerState, name: &str, args: Value| -> Result<curvebook_mcp::ToolCallResult, String> {
        curvebook_mcp::call_tool(st, name, args).map_err(|e| format!("{e:?}"))
    };

    let cmd_list = call(state, "command_list", json!({}))?;
    let list_json = cmd_list.structured_content.as_ref().ok_or("missing structured command_list")?;
    assert_eq!(list_json["live_attached"], json!(true));
    assert_eq!(list_json["mode"], json!("live_attach"));
    let nav_cmd = list_json["commands"]
        .as_array()
        .and_then(|arr| arr.iter().find(|c| c["id"] == "ui.navigator.toggle"))
        .ok_or("missing ui.navigator.toggle in command_list")?;
    assert_eq!(nav_cmd["available"], json!(true));
    assert_eq!(nav_cmd["requires_live_attach"], json!(true));

    let ui_run = call(state, "command_run", json!({ "command_id": "ui.navigator.toggle", "params": {} }))?;
    assert!(!ui_run.is_error, "ui command failed: {ui_run:?}");
    assert_eq!(ui_run.structured_content.as_ref().and_then(|v| v.get("mutated")), Some(&json!(true)));

    let live_csv = workdir.join("live.csv");
    let opened =
        call(state, "open_file", json!({ "path": live_csv.display().to_string(), "format": "csv", "sheet_id": "live-data", "title": "LiveData" }))?;
    assert!(!opened.is_error, "open_file failed: {opened:?}");
    assert_eq!(opened.structured_content.as_ref().and_then(|v| v.get("mode")), Some(&json!("live_attach")));
    assert_eq!(state.session().document().total_sheet_count(), 0, "phantom local MCP session must remain untouched in live-attach mode");

    let graph_run = call(
        state,
        "command_run",
        json!({
            "command_id": "graph.create",
            "params": { "kind": "xy", "sheet_id": "live-graph", "title": "LiveGraph", "data_sheet_id": "live-data" }
        }),
    )?;
    assert!(!graph_run.is_error, "graph.create failed: {graph_run:?}");

    let inspected = call(state, "doc_inspect", json!({ "sheet_id": "live-data", "include_cells": true }))?;
    assert!(!inspected.is_error, "doc_inspect failed: {inspected:?}");
    let inspect_json = inspected.structured_content.as_ref().ok_or("missing inspect payload")?;
    assert_eq!(inspect_json["mode"], json!("live_attach"));
    assert_eq!(inspect_json["id"], json!("live-data"));
    assert!(
        inspect_json["csv_preview"].as_str().is_some_and(|s| s.contains("GroupA") && s.contains("25")),
        "unexpected live csv_preview: {inspect_json}"
    );

    // Progressive `gui` toolset forwards `ui_inspect` and `ui_screenshot` over the same live channel.
    let enabled_gui = call(state, "enable_toolset", json!({ "group": "gui" }))?;
    assert!(!enabled_gui.is_error, "enable_toolset(gui) failed: {enabled_gui:?}");
    let ui_insp = call(state, "ui_inspect", json!({ "scope": "navigator" }))?;
    assert!(!ui_insp.is_error, "live ui_inspect failed: {ui_insp:?}");

    let res_list = curvebook_mcp::dispatch_json_value(json!({"jsonrpc": "2.0", "id": 101, "method": "resources/list", "params": {}}), state)
        .ok_or("unexpected None from resources/list")?;
    let resources = res_list.pointer("/result/resources").and_then(Value::as_array).ok_or("missing resources array")?;
    assert!(resources.iter().any(|r| r.get("uri") == Some(&json!("sheet://live-data"))), "missing live sheet resource: {res_list}");

    let res_read = curvebook_mcp::dispatch_json_value(
        json!({"jsonrpc": "2.0", "id": 102, "method": "resources/read", "params": {"uri": "sheet://live-data"}}),
        state,
    )
    .ok_or("unexpected None from resources/read")?;
    let res_text = res_read.pointer("/result/contents/0/text").and_then(Value::as_str).ok_or("missing resource text")?;
    assert!(res_text.contains("GroupA") && res_text.contains("25"), "unexpected live resource content: {res_text}");

    let preview = call(state, "render_preview", json!({ "sheet_id": "live-graph", "width_px": 256, "height_px": 160 }))?;
    assert!(!preview.is_error, "render_preview failed: {preview:?}");
    assert!(preview.content.iter().any(|b| matches!(b, curvebook_mcp::ContentBlock::Image { mime_type, .. } if mime_type == "image/png")));

    let exported_csv_path = workdir.join("exported.csv");
    let exported = call(state, "export", json!({ "format": "csv", "sheet_id": "live-data", "path": exported_csv_path.display().to_string() }))?;
    assert!(!exported.is_error, "export failed: {exported:?}");
    let exported_csv = std::fs::read_to_string(&exported_csv_path)?;
    assert!(exported_csv.contains("GroupA") && exported_csv.contains("GroupB"), "unexpected exported CSV: {exported_csv}");

    let project_path = workdir.join("live_project.curvebook");
    let saved = call(state, "save_file", json!({ "path": project_path.display().to_string() }))?;
    assert!(!saved.is_error, "save_file with path failed: {saved:?}");
    assert!(project_path.is_file());

    let reopened = call(state, "open_file", json!({ "path": project_path.display().to_string() }))?;
    assert!(!reopened.is_error, "reopen saved project failed: {reopened:?}");

    let saved_default = call(state, "save_file", json!({}))?;
    assert!(!saved_default.is_error, "save_file without explicit path failed in live-attach mode: {saved_default:?}");
    assert_eq!(state.session().document().total_sheet_count(), 0);
    Ok(())
}

#[test]
fn r1_s12_09_live_attach_stream_dispatch_forwards_tools_and_resources() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Cursor;

    let workdir = std::env::temp_dir().join(format!("cb_r1_s12_09_stream_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir)?;
    std::fs::write(workdir.join("live.csv"), "X,GroupA,GroupB\n1,10,20\n2,15,25\n")?;

    let token = String::from("r1-s12-09-stream-token");
    let auth = curvebook_platform::ControlAuth::from_token(&token)?;
    let mut app = UiApp::new();
    let mut state = McpServerState::with_control_port(49299)?.with_control_token(&token)?.with_control_handler(move |method, params| {
        let auth_line = serde_json::to_string(&json!({"id": 0, "method": "auth", "params": {"token": auth.as_str()}})).map_err(|e| e.to_string())?;
        let req_line = serde_json::to_string(&json!({"id": "mcp-1", "method": method, "params": params})).map_err(|e| e.to_string())?;
        let input = format!("{auth_line}\n{req_line}\n");
        let mut reader = BufReader::new(Cursor::new(input.as_bytes()));
        let mut out = Vec::new();
        let _ = curvebook_ui_egui::serve_control_stream_with_quit(&mut app, &mut reader, &mut out, &auth).map_err(|e| e.to_string())?;
        let text = String::from_utf8(out).map_err(|e| e.to_string())?;
        let reply_line = text.lines().nth(1).ok_or_else(|| format!("missing control reply in `{text}`"))?;
        let parsed: Value = serde_json::from_str(reply_line).map_err(|e| e.to_string())?;
        if parsed["ok"] == json!(true) { Ok(parsed["result"].clone()) } else { Err(parsed["error"].as_str().unwrap_or("control error").to_owned()) }
    });
    assert!(state.set_workdir(Some(&workdir)).is_err(), "combining control_port and workdir must be rejected");

    verify_live_attach_end_to_end(&mut state, &workdir)?;
    let _ = std::fs::remove_dir_all(&workdir);
    Ok(())
}

#[test]
fn r1_s12_09_live_attach_forwards_tools_and_resources() -> Result<(), Box<dyn std::error::Error>> {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(err) if err.kind() == ErrorKind::PermissionDenied && !loopback_required() => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    let port = listener.local_addr()?.port();
    listener.set_nonblocking(true)?;

    let token = "r1-s12-09-live-attach-test-token";
    let auth = curvebook_platform::ControlAuth::from_token(token)?;
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);

    let server_thread = thread::spawn(move || -> Result<(), String> {
        let mut app = UiApp::new();
        loop {
            if stop_flag.load(Ordering::Relaxed) {
                return Ok(());
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
                    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
                    let mut reader = BufReader::new(stream);
                    let quit =
                        curvebook_ui_egui::serve_control_stream_with_quit(&mut app, &mut reader, &mut writer, &auth).map_err(|e| e.to_string())?;
                    if quit {
                        return Ok(());
                    }
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(5)),
                Err(err) => return Err(err.to_string()),
            }
        }
    });

    let workdir = std::env::temp_dir().join(format!("cb_r1_s12_09_live_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir)?;
    std::fs::write(workdir.join("live.csv"), "X,GroupA,GroupB\n1,10,20\n2,15,25\n")?;

    let mut state = McpServerState::with_control_port(port)?.with_control_token(token)?;
    assert!(state.set_workdir(Some(&workdir)).is_err(), "combining control_port and workdir must be rejected");
    let probe = curvebook_mcp::call_tool(&mut state, "command_run", json!({ "command_id": "ui.navigator.toggle", "params": {} }))
        .map_err(|e| format!("{e:?}"))?;
    if !loopback_required()
        && probe.is_error
        && probe.structured_content.as_ref().is_some_and(|s| {
            s["code"] == json!("io_error")
                && s["message"].as_str().is_some_and(|m| m.contains("Operation not permitted") || m.contains("Permission denied"))
        })
    {
        stop.store(true, Ordering::Relaxed);
        let _ = server_thread.join();
        let _ = std::fs::remove_dir_all(&workdir);
        return Ok(());
    }

    verify_live_attach_end_to_end(&mut state, &workdir)?;
    stop.store(true, Ordering::Relaxed);
    let thread_res = server_thread.join().map_err(|_| "TCP server thread panicked")?;
    thread_res.map_err(|err| -> Box<dyn std::error::Error> { err.into() })?;
    let _ = std::fs::remove_dir_all(&workdir);
    Ok(())
}
