//! End-to-end parity tests for `lumencraft-cli mcp --gui-only` (`InProcessUiGuiHost` & `--control-port`).
#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::TcpListener;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use lumencraft_mcp::{GUI_ONLY_SERVER_INSTRUCTIONS, GUI_TOOL_NAMES, McpServerState};
use lumencraft_ui_egui::UiApp;
use serde_json::{Value, json};

fn write_framed_rpc(writer: &mut impl Write, payload: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let body = serde_json::to_string(payload)?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    writer.write_all(header.as_bytes())?;
    writer.write_all(body.as_bytes())?;
    writer.flush()?;
    Ok(())
}

fn read_framed_rpc(reader: &mut BufReader<impl Read>) -> Result<Value, Box<dyn std::error::Error>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            return Err("unexpected EOF while reading MCP headers".into());
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':')
            && name.eq_ignore_ascii_case("Content-Length")
        {
            content_length = Some(value.trim().parse::<usize>()?);
        }
    }
    let len = content_length.ok_or("missing Content-Length header in MCP response")?;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    Ok(serde_json::from_slice(&buf)?)
}

struct McpChild {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl McpChild {
    fn spawn(args: &[&str]) -> Result<Self, Box<dyn std::error::Error>> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_lumencraft-cli"))
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take().ok_or("missing child stdin")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("missing child stdout")?);
        Ok(Self { child, stdin, stdout })
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Result<Value, Box<dyn std::error::Error>> {
        write_framed_rpc(&mut self.stdin, &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        read_framed_rpc(&mut self.stdout)
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), Box<dyn std::error::Error>> {
        write_framed_rpc(&mut self.stdin, &json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn call_tool(&mut self, id: u64, name: &str, arguments: Value) -> Result<Value, Box<dyn std::error::Error>> {
        self.request(id, "tools/call", json!({ "name": name, "arguments": arguments }))
    }

    fn finish(self) -> Result<(), Box<dyn std::error::Error>> {
        drop(self.stdin);
        let out = self.child.wait_with_output()?;
        assert!(out.status.success(), "lumencraft-cli mcp exited with {:?}: {}", out.status, String::from_utf8_lossy(&out.stderr));
        Ok(())
    }
}

#[test]
fn cli_help_and_mcp_gui_only_flag_validation() -> Result<(), Box<dyn std::error::Error>> {
    let bin = env!("CARGO_BIN_EXE_lumencraft-cli");

    let help = Command::new(bin).arg("--help").output()?;
    assert!(help.status.success());
    let help_text = String::from_utf8(help.stdout)?;
    assert!(help_text.contains("mcp [--control-port <port>] [--gui-only]"), "unexpected --help:\n{help_text}");

    let bad = Command::new(bin).args(["mcp", "--gui-only", "--control-port", "0"]).output()?;
    assert_eq!(bad.status.code(), Some(2));
    Ok(())
}

#[test]
fn mcp_in_process_gui_only_enforces_tool_surface_violations_and_real_ui_app() -> Result<(), Box<dyn std::error::Error>> {
    let mut mcp = McpChild::spawn(&["mcp", "--gui-only"])?;

    let init = mcp.request(
        1,
        "initialize",
        json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": { "name": "mcp-gui-only-parity", "version": "0.1.0" }
        }),
    )?;
    assert_eq!(init["result"]["instructions"], json!(GUI_ONLY_SERVER_INSTRUCTIONS));
    mcp.notify("notifications/initialized", json!({}))?;

    let listed = mcp.request(2, "tools/list", json!({}))?;
    let tools = listed["result"]["tools"].as_array().ok_or("tools/list missing array")?;
    let returned_names: BTreeSet<&str> = tools.iter().filter_map(|t| t.get("name").and_then(Value::as_str)).collect();
    let expected_names: BTreeSet<&str> = GUI_TOOL_NAMES.iter().copied().collect();
    assert_eq!(returned_names, expected_names, "expected strictly the 8 gui tools");
    assert_eq!(tools.len(), 8);

    // Direct non-GUI calls must fail with `gui_only_violation`.
    for (idx, (tool, args)) in [
        ("command_run", json!({ "command": "sheet.createData", "params": { "kind": "xy" } })),
        ("open_file", json!({ "path": "/tmp/missing-gui-only.lumencraft" })),
        ("save_file", json!({ "path": "/tmp/out-gui-only.lumencraft" })),
        ("export", json!({ "path": "/tmp/out-gui-only.svg", "format": "svg" })),
        ("enable_toolset", json!({ "toolset": "analysis" })),
    ]
    .into_iter()
    .enumerate()
    {
        let resp = mcp.call_tool(10 + idx as u64, tool, args)?;
        assert_eq!(resp["result"]["isError"], json!(true), "expected isError for `{tool}`: {resp}");
        let serialized = serde_json::to_string(&resp["result"])?;
        assert!(serialized.contains("gui_only_violation"), "expected gui_only_violation for `{tool}`, got {serialized}");
    }

    // Drive the real in-process UiApp via ui_inspect -> ui_invoke_menu -> ui_answer_dialog -> ui_screenshot.
    let initial_inspect = mcp.call_tool(20, "ui_inspect", json!({ "scope": "all" }))?;
    assert_ne!(initial_inspect["result"]["isError"], json!(true), "initial inspect failed: {initial_inspect}");
    let initial_text = initial_inspect["result"]["content"][0]["text"].as_str().ok_or("missing inspect text")?;
    let initial_snap: Value = serde_json::from_str(initial_text)?;
    assert_eq!(initial_snap["gui_only"], json!(true));
    assert_eq!(initial_snap["active_dialog"], json!("none"));

    // Open Power & Sample Size wizard via array path segments to exercise InProcessUiGuiHost normalization too.
    let open_wiz = mcp.call_tool(21, "ui_invoke_menu", json!({ "path": ["Analyze", "Power and Sample Size..."] }))?;
    assert_ne!(open_wiz["result"]["isError"], json!(true), "ui_invoke_menu failed: {open_wiz}");

    let wiz_inspect = mcp.call_tool(22, "ui_inspect", json!({ "scope": "dialog" }))?;
    let wiz_text = wiz_inspect["result"]["content"][0]["text"].as_str().ok_or("missing dialog inspect text")?;
    let wiz_snap: Value = serde_json::from_str(wiz_text)?;
    assert_eq!(wiz_snap["active_dialog"], json!("power_and_sample_size"));

    for step in 0..3 {
        let next = mcp.call_tool(23 + step, "ui_answer_dialog", json!({ "action": "next" }))?;
        assert_ne!(next["result"]["isError"], json!(true), "wizard next step {step} failed: {next}");
    }
    let finish = mcp.call_tool(26, "ui_answer_dialog", json!({ "action": "create" }))?;
    assert_ne!(finish["result"]["isError"], json!(true), "wizard create failed: {finish}");

    let post_inspect = mcp.call_tool(27, "ui_inspect", json!({ "scope": "all" }))?;
    let post_text = post_inspect["result"]["content"][0]["text"].as_str().ok_or("missing post inspect text")?;
    let post_snap: Value = serde_json::from_str(post_text)?;
    assert_eq!(post_snap["active_dialog"], json!("none"), "expected dialog closed after create: {post_snap}");
    let nav_sheets = post_snap["navigator_sheets"].as_array().ok_or("missing navigator_sheets")?;
    assert!(nav_sheets.len() >= 3, "expected power wizard to create data/result/graph sheets, got {nav_sheets:?}");

    // Screenshot with "workspace" exercises target normalization and returns inline image/png content.
    let shot = mcp.call_tool(28, "ui_screenshot", json!({ "target": "workspace", "width_px": 640, "height_px": 400 }))?;
    assert_ne!(shot["result"]["isError"], json!(true), "ui_screenshot failed: {shot}");
    let blocks = shot["result"]["content"].as_array().ok_or("missing screenshot content blocks")?;
    let img_block = blocks.iter().find(|b| b.get("type").and_then(Value::as_str) == Some("image")).ok_or("missing inline image content block")?;
    assert_eq!(img_block["mimeType"], json!("image/png"));
    assert!(img_block["data"].as_str().is_some_and(|b64| b64.starts_with("iVBORw0KGgo")), "expected PNG base64 payload in screenshot response");

    mcp.finish()
}

#[test]
fn mcp_control_port_gui_only_drives_live_control_tcp_session() -> Result<(), Box<dyn std::error::Error>> {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(err) if err.kind() == ErrorKind::PermissionDenied => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    let port = listener.local_addr()?.port();
    assert!(McpServerState::with_control_port_and_gui_only(port, true)?.is_gui_only());

    listener.set_nonblocking(true)?;
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);

    let server_thread = thread::spawn(move || -> Result<(), String> {
        let mut app = UiApp::new();
        app.set_gui_only(true);
        let stream = loop {
            if stop_flag.load(Ordering::Relaxed) {
                return Ok(());
            }
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(err) => return Err(err.to_string()),
            }
        };
        stream.set_nonblocking(false).map_err(|e| e.to_string())?;
        let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
        let reader = BufReader::new(stream);
        for line_res in reader.lines() {
            let line = line_res.map_err(|e| e.to_string())?;
            if line.trim().is_empty() {
                continue;
            }
            let reply = app.dispatch_control_line(&line);
            writer.write_all(reply.as_bytes()).map_err(|e| e.to_string())?;
            writer.write_all(b"\n").map_err(|e| e.to_string())?;
            writer.flush().map_err(|e| e.to_string())?;
        }
        Ok(())
    });

    let port_str = port.to_string();
    let mut mcp = McpChild::spawn(&["mcp", "--control-port", &port_str, "--gui-only"])?;
    let init = mcp.request(
        1,
        "initialize",
        json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": { "name": "mcp-tcp-gui-only", "version": "0.1.0" }
        }),
    )?;
    assert_eq!(init["result"]["instructions"], json!(GUI_ONLY_SERVER_INSTRUCTIONS));
    mcp.notify("notifications/initialized", json!({}))?;

    let listed = mcp.request(2, "tools/list", json!({}))?;
    let tools = listed["result"]["tools"].as_array().ok_or("tools/list missing array")?;
    assert_eq!(tools.len(), 8);

    let blocked = mcp.call_tool(3, "command_run", json!({ "command": "sheet.createData", "params": { "kind": "xy" } }))?;
    assert_eq!(blocked["result"]["isError"], json!(true));
    assert!(serde_json::to_string(&blocked["result"])?.contains("gui_only_violation"));

    let inspected = mcp.call_tool(4, "ui_inspect", json!({ "scope": "all" }))?;
    if inspected["result"]["isError"] == json!(true)
        && inspected["result"]["structuredContent"]["code"] == json!("io_error")
        && inspected["result"]["structuredContent"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("Operation not permitted") || m.contains("Permission denied"))
    {
        stop.store(true, Ordering::Relaxed);
        mcp.finish()?;
        let thread_res = server_thread.join().map_err(|_| "TCP server thread panicked")?;
        thread_res.map_err(|err| -> Box<dyn std::error::Error> { err.into() })?;
        return Ok(());
    }

    assert_ne!(inspected["result"]["isError"], json!(true), "TCP ui_inspect failed: {inspected}");
    let inspect_text = inspected["result"]["content"][0]["text"].as_str().ok_or("missing inspect text")?;
    let snap: Value = serde_json::from_str(inspect_text)?;
    assert_eq!(snap["gui_only"], json!(true));

    let shot = mcp.call_tool(5, "ui_screenshot", json!({ "width_px": 512, "height_px": 320 }))?;
    assert_ne!(shot["result"]["isError"], json!(true), "TCP ui_screenshot failed: {shot}");
    let has_png = shot["result"]["content"]
        .as_array()
        .is_some_and(|arr| arr.iter().any(|b| b["type"] == json!("image") && b["mimeType"] == json!("image/png")));
    assert!(has_png, "expected inline image/png from --control-port --gui-only: {shot}");

    stop.store(true, Ordering::Relaxed);
    mcp.finish()?;
    let thread_res = server_thread.join().map_err(|_| "TCP server thread panicked")?;
    thread_res.map_err(|err| -> Box<dyn std::error::Error> { err.into() })?;
    Ok(())
}
