//! Integration tests for Wave 16 `--gui-only` policy enforcement, the 8-tool `gui` MCP
//! surface, and `GuiSessionHost` / `--control-port` routing.

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Cursor, ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::rc::Rc;
use std::thread;
use std::time::Duration;

use curvebook_doc::ErrorCode;
use curvebook_engine::{EngineError, GuiOnlyPolicy};
use curvebook_mcp::{
    ContentBlock, GUI_ONLY_SERVER_INSTRUCTIONS, GUI_TOOL_NAMES, GuiSessionHost, McpServerState, SERVER_INSTRUCTIONS, ToolCallError, call_tool,
    list_tools, list_tools_for_state, serve,
};
use serde_json::{Value, json};

fn env_truthy(var: &str) -> bool {
    std::env::var(var).is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

fn loopback_required() -> bool {
    env_truthy("CI") || env_truthy("CURVEBOOK_REQUIRE_LOOPBACK")
}

#[derive(Default)]
struct MockGuiHost {
    gui_only: bool,
    calls: Vec<(String, Value)>,
    next_error: Option<EngineError>,
    png_b64: String,
}

impl MockGuiHost {
    fn new(gui_only: bool) -> Self {
        Self {
            gui_only,
            calls: Vec::new(),
            next_error: None,
            // Minimal valid 1x1 transparent PNG encoded as base64.
            png_b64: String::from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII="),
        }
    }
}

impl GuiSessionHost for MockGuiHost {
    fn call_gui(&mut self, method: &str, params: Value) -> Result<Value, EngineError> {
        self.calls.push((method.to_owned(), params.clone()));
        if let Some(err) = self.next_error.take() {
            return Err(err);
        }
        match method {
            "ui.screenshot" => Ok(json!({
                "format": "png",
                "target": params.get("target").cloned().unwrap_or_else(|| json!("workspace")),
                "width_px": params.get("width_px").cloned().unwrap_or_else(|| json!(1024)),
                "height_px": params.get("height_px").cloned().unwrap_or_else(|| json!(640)),
                "dpi": params.get("dpi").cloned().unwrap_or_else(|| json!(144)),
                "bytes_len": 68,
                "data_base64": self.png_b64,
            })),
            _ => Ok(json!({
                "ok": true,
                "method": method,
                "params": params,
            })),
        }
    }

    fn set_gui_only(&mut self, gui_only: bool) {
        self.gui_only = gui_only;
    }

    fn is_gui_only(&self) -> bool {
        self.gui_only
    }
}

/// Shared handle wrapper so tests can inspect recorded calls after moving the trait object into
/// [`McpServerState`].
struct SharedMockHost {
    inner: Rc<RefCell<MockGuiHost>>,
}

impl SharedMockHost {
    fn new(gui_only: bool) -> (Self, Rc<RefCell<MockGuiHost>>) {
        let rc = Rc::new(RefCell::new(MockGuiHost::new(gui_only)));
        (Self { inner: Rc::clone(&rc) }, rc)
    }
}

impl GuiSessionHost for SharedMockHost {
    fn call_gui(&mut self, method: &str, params: Value) -> Result<Value, EngineError> {
        self.inner.borrow_mut().call_gui(method, params)
    }

    fn set_gui_only(&mut self, gui_only: bool) {
        self.inner.borrow_mut().set_gui_only(gui_only);
    }

    fn is_gui_only(&self) -> bool {
        self.inner.borrow().is_gui_only()
    }
}

fn frame_cl(body: &Value) -> Result<Vec<u8>, String> {
    let raw = serde_json::to_string(body).map_err(|err| format!("serialize request: {err}"))?;
    Ok(format!("Content-Length: {}\r\n\r\n{raw}", raw.len()).into_bytes())
}

fn decode_cl_frames(raw: &[u8]) -> Result<Vec<Value>, String> {
    let mut cur = raw;
    let mut out = Vec::new();
    while !cur.is_empty() {
        let text = std::str::from_utf8(cur).map_err(|err| format!("utf8: {err}"))?;
        let (headers, rest) = text.split_once("\r\n\r\n").ok_or_else(|| String::from("header sep"))?;
        let len_raw = headers.strip_prefix("Content-Length: ").ok_or_else(|| String::from("content-length"))?;
        let len: usize = len_raw.trim().parse().map_err(|err| format!("len: {err}"))?;
        let (body, tail) = rest.as_bytes().split_at_checked(len).ok_or_else(|| String::from("truncated body"))?;
        out.push(serde_json::from_slice(body).map_err(|err| format!("json frame: {err}"))?);
        cur = tail;
    }
    Ok(out)
}

fn rpc_roundtrip(state: &mut McpServerState, id: i64, method: &str, params: Value) -> Result<Value, String> {
    let framed = frame_cl(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    }))?;
    let mut out = Vec::new();
    serve(&mut Cursor::new(framed), &mut out, state).map_err(|err| format!("serve {method}: {err}"))?;
    decode_cl_frames(&out)?.into_iter().next().ok_or_else(|| format!("expected response frame for {method}"))
}

#[test]
fn gui_only_tools_list_shrinks_strictly_to_eight_gui_tools() {
    let (host, _) = SharedMockHost::new(true);
    let mut state = McpServerState::with_gui_host(Box::new(host));
    assert!(state.is_gui_only());
    assert_eq!(state.session().gui_only(), GuiOnlyPolicy::Strict);

    // Enabling a CommandGroup beforehand must not leak tools once --gui-only is active.
    let mut pre_enabled = McpServerState::new();
    call_tool(&mut pre_enabled, "enable_toolset", json!({ "group": "sheet" })).expect("enable sheet");
    pre_enabled.set_gui_only(true);
    let shrunk_names: Vec<String> = list_tools_for_state(&pre_enabled).into_iter().map(|t| t.name).collect();
    assert_eq!(shrunk_names, GUI_TOOL_NAMES);

    let init = rpc_roundtrip(&mut state, 1, "initialize", json!({ "protocolVersion": "2024-11-05" })).expect("initialize");
    assert_eq!(init["result"]["instructions"], json!(GUI_ONLY_SERVER_INSTRUCTIONS));

    let list_reply = rpc_roundtrip(&mut state, 2, "tools/list", json!({})).expect("tools/list");
    let wire_tools = list_reply["result"]["tools"].as_array().expect("tools array");
    let wire_names: Vec<&str> = wire_tools.iter().map(|t| t["name"].as_str().expect("tool name")).collect();
    assert_eq!(wire_names, GUI_TOOL_NAMES);

    // Ensure annotations match worst-case Effects across all 8 GUI tools.
    assert_eq!(
        wire_tools[0]["annotations"],
        json!({ "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false })
    );
    assert_eq!(
        wire_tools[1]["annotations"],
        json!({ "readOnlyHint": false, "destructiveHint": true, "idempotentHint": true, "openWorldHint": true })
    );
    assert_eq!(
        wire_tools[2]["annotations"],
        json!({ "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true })
    );
    assert_eq!(
        wire_tools[3]["annotations"],
        json!({ "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true })
    );
    assert_eq!(
        wire_tools[4]["annotations"],
        json!({ "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true })
    );
    assert_eq!(
        wire_tools[5]["annotations"],
        json!({ "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true })
    );
    assert_eq!(
        wire_tools[6]["annotations"],
        json!({ "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false })
    );
    assert_eq!(
        wire_tools[7]["annotations"],
        json!({ "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true })
    );

    // resources/list is empty and resources/read is refused under --gui-only.
    let res_list = rpc_roundtrip(&mut state, 3, "resources/list", json!({})).expect("resources/list");
    assert_eq!(res_list["result"]["resources"], json!([]));

    let res_read = rpc_roundtrip(&mut state, 4, "resources/read", json!({ "uri": "project://summary" })).expect("resources/read");
    assert_eq!(res_read["error"]["data"]["code"], json!("gui_only_violation"));
    assert_eq!(res_read["error"]["data"]["category"], json!("policy"));
}

#[test]
fn gui_only_blocks_all_non_gui_tools_with_gui_only_violation() {
    let (host, shared) = SharedMockHost::new(false);
    let mut state = McpServerState::new().with_gui_only(true);
    state.set_gui_host(Some(Box::new(host)));

    // Policy propagates to both Session and newly attached GuiSessionHost.
    assert_eq!(state.session().gui_only(), GuiOnlyPolicy::Strict);
    assert!(shared.borrow().is_gui_only());

    let blocked_calls = [
        ("command_run", json!({ "command_id": "sheet.createData" })),
        ("command_batch", json!({ "commands": [] })),
        ("open_file", json!({ "path": "data.csv" })),
        ("save_file", json!({ "path": "out.curvebook" })),
        ("export", json!({ "format": "csv" })),
        ("render_preview", json!({ "sheet_id": "graph-1" })),
        ("doc_inspect", json!({})),
        ("command_list", json!({})),
        ("list_toolsets", json!({})),
        ("enable_toolset", json!({ "group": "all" })),
        ("sheet_create_data", json!({})),
        ("stats_run", json!({})),
        ("arbitrary_unknown_tool", json!({})),
    ];

    for (idx, (tool, args)) in blocked_calls.iter().enumerate() {
        let direct = call_tool(&mut state, tool, args.clone()).expect("structured violation result");
        assert!(direct.is_error, "expected is_error=true for {tool}");
        let payload = direct.structured_content.as_ref().expect("structured error payload");
        assert_eq!(payload["code"], json!("gui_only_violation"), "wrong code for {tool}");
        assert_eq!(payload["category"], json!("policy"), "wrong category for {tool}");
        assert_eq!(payload["details"]["tool"], json!(tool));

        let wire = rpc_roundtrip(&mut state, i64::try_from(idx).unwrap() + 10, "tools/call", json!({ "name": tool, "arguments": args }))
            .expect("rpc wire call");
        assert_eq!(wire["result"]["isError"], json!(true));
        assert_eq!(wire["result"]["structuredContent"]["code"], json!("gui_only_violation"));
    }

    assert!(shared.borrow().calls.is_empty(), "blocked calls must never reach GuiSessionHost");

    // Verify disabling propagates cleanly as well.
    state.set_gui_only(false);
    assert_eq!(state.session().gui_only(), GuiOnlyPolicy::Disabled);
    assert!(!shared.borrow().is_gui_only());
}

#[test]
fn gui_tools_route_through_gui_session_host_including_png_screenshot_and_tsv() {
    let (host, shared) = SharedMockHost::new(true);
    let mut state = McpServerState::with_gui_host(Box::new(host));

    let inspect_res = call_tool(&mut state, "ui_inspect", json!({ "scope": "dialog", "include_bounds": true })).expect("ui_inspect");
    assert!(!inspect_res.is_error);

    let shot_reply = rpc_roundtrip(
        &mut state,
        2,
        "tools/call",
        json!({
            "name": "ui_screenshot",
            "arguments": { "target": "workspace", "width_px": 320, "height_px": 200, "dpi": 144 }
        }),
    )
    .expect("ui_screenshot rpc");
    assert_eq!(shot_reply["result"]["isError"], json!(false));
    let content = shot_reply["result"]["content"].as_array().expect("content array");
    assert_eq!(content.len(), 2);
    assert_eq!(content[0]["type"], json!("image"));
    assert_eq!(content[0]["mimeType"], json!("image/png"));
    assert!(!content[0]["data"].as_str().expect("png b64").is_empty());
    assert_eq!(content[1]["type"], json!("text"));
    let meta = &shot_reply["result"]["structuredContent"];
    assert_eq!(meta["format"], json!("png"));
    assert_eq!(meta["width_px"], json!(320));
    assert_eq!(meta["height_px"], json!(200));
    assert!(meta.get("data_base64").is_none(), "base64 must not be duplicated inside structuredContent");

    call_tool(
        &mut state,
        "ui_pointer_gesture",
        json!({
            "action": "Drag",
            "widget_id": "pt_0",
            "to_x": 250.5,
            "to_y": 120.0,
            "modifiers": ["cmd", "shift"],
        }),
    )
    .expect("ui_pointer_gesture");

    let tsv = "  X\tY1\tY2\n1\t2.5\t3.0\n";
    call_tool(
        &mut state,
        "ui_type_text",
        json!({
            "text": tsv,
            "widget_id": "grid_cell_0_0",
            "clear_existing": true,
            "commit_enter": false,
        }),
    )
    .expect("ui_type_text");

    call_tool(&mut state, "ui_press_key", json!({ "key": "t", "modifiers": { "cmd": true } })).expect("ui_press_key");
    call_tool(&mut state, "ui_invoke_menu", json!({ "path": "Analyze > Column Analyses > One-way ANOVA", "command_id": "stats.run" }))
        .expect("ui_invoke_menu");
    call_tool(&mut state, "ui_switch_tab", json!({ "tab": "multiple_comparisons", "sheet_id": "result-1", "widget_id": "dialog_tabs" }))
        .expect("ui_switch_tab");
    call_tool(
        &mut state,
        "ui_answer_dialog",
        json!({
            "action": "ok",
            "fields": { "alpha": 0.01 },
            "values": { "posthoc": "tukey" },
        }),
    )
    .expect("ui_answer_dialog");

    // Backend EngineError propagates with canonical structured fields.
    shared.borrow_mut().next_error =
        Some(EngineError::with_code(ErrorCode::Conflict, "Repeated Measures tab is disabled").with_remediation("Select matched pairing first."));
    let disabled_err = call_tool(&mut state, "ui_switch_tab", json!({ "tab": "repeated_measures" })).expect("disabled error");
    assert!(disabled_err.is_error);
    let err_payload = disabled_err.structured_content.expect("structured error");
    assert_eq!(err_payload["code"], json!("conflict"));
    assert_eq!(err_payload["remediation"], json!("Select matched pairing first."));

    let host_guard = shared.borrow();
    let calls = &host_guard.calls;
    let methods: Vec<&str> = calls.iter().map(|(m, _)| m.as_str()).collect();
    assert_eq!(
        methods,
        vec![
            "ui.inspect",
            "ui.screenshot",
            "ui.pointer_gesture",
            "ui.type_text",
            "ui.press_key",
            "ui.invoke_menu",
            "ui.switch_tab",
            "ui.answer_dialog",
            "ui.switch_tab",
        ]
    );
    assert_eq!(calls[2].1["action"], json!("drag"));
    assert_eq!(calls[2].1["modifiers"], json!({ "shift": true, "ctrl": false, "alt": false, "meta": true }));
    assert_eq!(calls[3].1["text"], json!(tsv));
    assert_eq!(calls[4].1["modifiers"], json!({ "shift": false, "ctrl": false, "alt": false, "meta": true }));
}

#[test]
fn gui_tools_return_conflict_when_unattached_and_route_over_control_port() {
    let mut unattached = McpServerState::new().with_gui_only(true);
    let err = call_tool(&mut unattached, "ui_inspect", json!({ "scope": "all" })).expect("structured conflict");
    assert!(err.is_error);
    let payload = err.structured_content.expect("conflict payload");
    assert_eq!(payload["code"], json!("conflict"));
    assert_eq!(payload["category"], json!("state"));
    assert!(payload["remediation"].as_str().is_some_and(|r| r.contains("GuiSessionHost") && r.contains("--control-port")));

    // Probe whether loopback TCP bind+connect is allowed in the active environment (mirrors mcp_e2e).
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(err) if err.kind() == ErrorKind::PermissionDenied => {
            assert!(!loopback_required(), "FAIL-CLOSED loopback bind denied under CI/CURVEBOOK_REQUIRE_LOOPBACK: {err}");
            let mut attached = McpServerState::with_control_port_and_gui_only(59999, true).expect("control port state");
            let inspect = call_tool(&mut attached, "ui_inspect", json!({ "scope": "grid" })).expect("sandbox err");
            assert!(inspect.is_error);
            return;
        }
        Err(err) => panic!("unexpected loopback bind error: {err}"),
    };
    let probe_addr = listener.local_addr().expect("local addr");
    let port = probe_addr.port();
    match TcpStream::connect_timeout(&probe_addr, Duration::from_millis(200)) {
        Ok(stream) => {
            drop(stream);
            let _ = listener.accept();
        }
        Err(err) if err.kind() == ErrorKind::PermissionDenied => {
            assert!(!loopback_required(), "FAIL-CLOSED loopback connect denied under CI/CURVEBOOK_REQUIRE_LOOPBACK: {err}");
            let mut attached = McpServerState::with_control_port_and_gui_only(port, true).expect("control port state");
            let inspect = call_tool(&mut attached, "ui_inspect", json!({ "scope": "grid" })).expect("sandbox err");
            assert!(inspect.is_error);
            let detail = inspect.structured_content.expect("structured error");
            assert_eq!(detail["code"], json!("io_error"));
            assert!(detail["message"].as_str().is_some_and(|m| m.contains("failed to connect")));
            return;
        }
        Err(err) => panic!("unexpected loopback connect probe error: {err}"),
    }

    // Spin up a loopback --control server handling initial `ping` handshake + three sequential requests.
    let server = thread::spawn(move || -> Result<(), String> {
        for expected in 0..4 {
            let (mut stream, _) = listener.accept().map_err(|e| format!("accept: {e}"))?;
            let mut line = String::new();
            let cloned = stream.try_clone().map_err(|e| format!("clone: {e}"))?;
            let mut reader = BufReader::new(cloned);
            reader.read_line(&mut line).map_err(|e| format!("read req: {e}"))?;
            let req: Value = serde_json::from_str(&line).map_err(|e| format!("req json: {e}"))?;
            let id = req["id"].as_str().ok_or_else(|| String::from("missing id"))?;
            let resp = match expected {
                0 => {
                    assert_eq!(req["method"], json!("ping"));
                    json!({ "id": id, "ok": true, "result": { "pong": true, "gui_only": true } })
                }
                1 => {
                    assert_eq!(req["method"], json!("ui.inspect"));
                    assert_eq!(req["params"]["scope"], json!("grid"));
                    json!({ "id": id, "ok": true, "result": { "scope": "grid", "active_sheet": "data-1" } })
                }
                2 => {
                    assert_eq!(req["method"], json!("ui.screenshot"));
                    json!({
                        "id": id,
                        "ok": true,
                        "result": {
                            "width_px": 256,
                            "height_px": 160,
                            "dpi": 72,
                            "data_base64": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=",
                        }
                    })
                }
                _ => {
                    json!({
                        "id": id,
                        "ok": false,
                        "error": "menu disabled under gui_only",
                        "error_detail": {
                            "code": "gui_only_violation",
                            "category": "policy",
                            "message": "direct shortcut rejected by remote host",
                            "retryable": false,
                            "remediation": "Use visible menu path instead.",
                            "details": { "remote": true }
                        }
                    })
                }
            };
            let encoded = serde_json::to_vec(&resp).map_err(|e| format!("encode resp: {e}"))?;
            stream
                .write_all(&encoded)
                .and_then(|()| stream.write_all(b"\n"))
                .and_then(|()| stream.flush())
                .map_err(|e| format!("write resp: {e}"))?;
        }
        Ok(())
    });

    let mut attached = McpServerState::with_control_port_and_gui_only(port, true).expect("control port state");
    let inspect = call_tool(&mut attached, "ui_inspect", json!({ "scope": "grid" })).expect("remote ui_inspect");
    assert!(!inspect.is_error);
    assert_eq!(inspect.structured_content.expect("inspect json")["active_sheet"], json!("data-1"));

    let shot = call_tool(&mut attached, "ui_screenshot", json!({ "width_px": 256, "height_px": 160 })).expect("remote ui_screenshot");
    assert!(!shot.is_error);
    assert!(matches!(&shot.content[0], ContentBlock::Image { mime_type, .. } if mime_type == "image/png"));

    let remote_err = call_tool(&mut attached, "ui_invoke_menu", json!({ "command_id": "file.open" })).expect("remote err");
    assert!(remote_err.is_error);
    let detail = remote_err.structured_content.expect("remote error detail");
    assert_eq!(detail["code"], json!("gui_only_violation"));
    assert_eq!(detail["remediation"], json!("Use visible menu path instead."));

    server.join().expect("control server thread").expect("control server io");
}

#[test]
fn normal_mode_preserves_defaults_and_supports_progressive_gui_toolset() {
    let mut state = McpServerState::new();
    assert!(!state.is_gui_only());
    assert_eq!(list_tools_for_state(&state).len(), list_tools().len());

    let init = rpc_roundtrip(&mut state, 1, "initialize", json!({ "protocolVersion": "2024-11-05" })).expect("initialize");
    assert_eq!(init["result"]["instructions"], json!(SERVER_INSTRUCTIONS));

    let enable_reply =
        rpc_roundtrip(&mut state, 2, "tools/call", json!({ "name": "enable_toolset", "arguments": { "group": "gui" } })).expect("enable gui");
    assert_eq!(enable_reply["result"]["isError"], json!(false));
    assert_eq!(enable_reply["result"]["structuredContent"]["changed"], json!(true));
    assert_eq!(enable_reply["result"]["structuredContent"]["enabled_groups"], json!(["gui"]));

    let names: Vec<String> = list_tools_for_state(&state).into_iter().map(|t| t.name).collect();
    assert_eq!(names.len(), 18);
    for expected in GUI_TOOL_NAMES {
        assert!(names.contains(&expected.to_owned()), "missing gui tool {expected}");
    }

    let disable_reply = call_tool(&mut state, "enable_toolset", json!({ "group": "gui", "enabled": false })).expect("disable gui");
    assert_eq!(disable_reply.structured_content.expect("disable json")["enabled_groups"], json!([]));
    assert_eq!(list_tools_for_state(&state).len(), 10);

    // State constructed with GuiSessionHost (gui_only == false) exposes both core + gui tools.
    let (host, _) = SharedMockHost::new(false);
    let mut hosted = McpServerState::with_gui_host(Box::new(host));
    assert!(!hosted.is_gui_only());
    assert_eq!(list_tools_for_state(&hosted).len(), 18);
    assert!(!call_tool(&mut hosted, "doc_inspect", json!({})).expect("doc_inspect").is_error);
    assert!(!call_tool(&mut hosted, "ui_inspect", json!({})).expect("ui_inspect").is_error);
}

#[test]
fn gui_tools_parameter_validation_rejects_malformed_inputs() {
    let (host, _) = SharedMockHost::new(true);
    let mut state = McpServerState::with_gui_host(Box::new(host));

    let bad_cases = [
        ("ui_inspect", json!({ "scope": "invalid_scope" })),
        ("ui_screenshot", json!({ "width_px": 16 })),
        ("ui_pointer_gesture", json!({ "action": "click", "x": 10.0 })),
        ("ui_pointer_gesture", json!({ "action": "drag", "widget_id": "pt_0" })),
        ("ui_pointer_gesture", json!({ "action": "spin", "widget_id": "pt_0" })),
        ("ui_type_text", json!({ "text": 42 })),
        ("ui_press_key", json!({ "key": "   " })),
        ("ui_invoke_menu", json!({})),
        ("ui_switch_tab", json!({})),
        ("ui_switch_tab", json!({ "tab": -1 })),
        ("ui_switch_tab", json!({ "tab": "   " })),
        ("ui_answer_dialog", json!({})),
    ];

    for (tool, args) in bad_cases {
        let res = call_tool(&mut state, tool, args);
        assert!(matches!(res, Err(ToolCallError::InvalidParams(_))), "expected InvalidParams for {tool}, got {res:?}");
    }
}

#[test]
fn gui_tools_forward_extended_parameters_and_reject_screenshot_path_in_gui_only() {
    let (host, shared) = SharedMockHost::new(true);
    let mut state = McpServerState::with_gui_host(Box::new(host));

    for scope in ["workspace", "toolbar", "tabs"] {
        let res = call_tool(&mut state, "ui_inspect", json!({ "scope": scope })).expect("ui_inspect extended scope");
        assert!(!res.is_error);
    }

    let shot = call_tool(&mut state, "ui_screenshot", json!({ "target": "full_window", "width_px": 640, "height_px": 400 }))
        .expect("full_window screenshot");
    assert!(!shot.is_error);

    let rejected = call_tool(&mut state, "ui_screenshot", json!({ "target": "workspace", "path": "blocked.png" }))
        .expect("gui-only screenshot path error result");
    assert!(rejected.is_error);
    assert_eq!(rejected.structured_content.as_ref().and_then(|v| v.get("code")), Some(&json!("gui_only_violation")));

    call_tool(
        &mut state,
        "ui_pointer_gesture",
        json!({
            "action": "hover",
            "widget_id": "power.curve_hover",
            "hover_fraction_x": 0.45,
            "hover_n": 18.0,
            "metrics": { "width_px": 1280.0, "height_px": 800.0, "scale_factor": 1.0 }
        }),
    )
    .expect("power hover");

    call_tool(
        &mut state,
        "ui_invoke_menu",
        json!({
            "command_id": "sheet.createData",
            "params": { "kind": "Column", "rows": 5, "cols": 3 }
        }),
    )
    .expect("invoke menu with params");

    call_tool(&mut state, "ui_switch_tab", json!({ "sheet_id": "result-1" })).expect("sheet_id only");
    call_tool(&mut state, "ui_switch_tab", json!({ "tab": 2 })).expect("integer tab index");

    call_tool(
        &mut state,
        "ui_answer_dialog",
        json!({
            "action": "ok",
            "patches": { "alpha": 0.01 },
            "force_create_new": true,
            "choice": "create_new"
        }),
    )
    .expect("answer dialog extended");

    let host_guard = shared.borrow();
    let calls = &host_guard.calls;
    assert_eq!(calls[3].1["target"], json!("full_window"));
    assert_eq!(calls[4].1["hover_fraction_x"], json!(0.45));
    assert_eq!(calls[4].1["hover_n"], json!(18.0));
    assert_eq!(calls[4].1["metrics"], json!({ "logical_width": 1280.0, "logical_height": 800.0, "scale_factor": 1.0 }));
    assert!(calls[4].1["metrics"].get("width_px").is_none());
    assert!(calls[4].1["metrics"].get("height_px").is_none());
    assert_eq!(calls[5].1["params"]["kind"], json!("Column"));
    assert_eq!(calls[6].1["sheet_id"], json!("result-1"));
    assert_eq!(calls[7].1["tab"], json!(2));
    assert_eq!(calls[8].1["patches"]["alpha"], json!(0.01));
    assert_eq!(calls[8].1["force_create_new"], json!(true));
    assert_eq!(calls[8].1["choice"], json!("create_new"));
    drop(host_guard);

    call_tool(
        &mut state,
        "ui_pointer_gesture",
        json!({
            "action": "click",
            "widget_id": "btn.ok",
            "metrics": { "logical_width": 1440.0, "logical_height": 900.0, "scale_factor": 2.0 }
        }),
    )
    .expect("canonical pointer metrics");
    call_tool(
        &mut state,
        "ui_inspect",
        json!({
            "scope": "canvas",
            "include_bounds": true,
            "metrics": { "width_px": 1600.0, "height_px": 1000.0, "scale_factor": 1.5 }
        }),
    )
    .expect("ui_inspect alias metrics");

    let host_guard = shared.borrow();
    assert_eq!(host_guard.calls[9].1["metrics"], json!({ "logical_width": 1440.0, "logical_height": 900.0, "scale_factor": 2.0 }));
    assert_eq!(host_guard.calls[10].1["metrics"], json!({ "logical_width": 1600.0, "logical_height": 1000.0, "scale_factor": 1.5 }));
}

#[test]
fn gui_tools_worst_case_annotations_cover_all_eight_tools() {
    let expected = [
        ("ui_inspect", true, false, true, false),
        ("ui_screenshot", false, true, true, true),
        ("ui_pointer_gesture", false, true, false, true),
        ("ui_type_text", false, true, false, true),
        ("ui_press_key", false, true, false, true),
        ("ui_invoke_menu", false, true, false, true),
        ("ui_switch_tab", false, false, true, false),
        ("ui_answer_dialog", false, true, false, true),
    ];

    let gui_defs = curvebook_mcp::list_gui_tools();
    assert_eq!(gui_defs.len(), GUI_TOOL_NAMES.len());
    // Confirm `ui_inspect` input schema uses canonical ViewportMetrics keys (`logical_width`, `logical_height`, `scale_factor`).
    let ui_inspect_metrics =
        gui_defs[0].input_schema.pointer("/properties/metrics/properties").and_then(Value::as_object).expect("ui_inspect metrics schema");
    for key in ["logical_width", "logical_height", "scale_factor"] {
        assert!(ui_inspect_metrics.contains_key(key), "ui_inspect schema missing canonical ViewportMetrics key `{key}`");
    }

    let mut gui_only_state = McpServerState::new().with_gui_only(true);
    let gui_only_defs = list_tools_for_state(&gui_only_state);
    assert_eq!(gui_only_defs.len(), GUI_TOOL_NAMES.len());

    let wire_list = rpc_roundtrip(&mut gui_only_state, 10, "tools/list", json!({})).expect("tools/list");
    let wire_tools = wire_list["result"]["tools"].as_array().expect("wire tools array");

    let mut enabled_state = McpServerState::new();
    call_tool(&mut enabled_state, "enable_toolset", json!({ "group": "gui" })).expect("enable gui toolset");
    let full_defs = list_tools_for_state(&enabled_state);

    for (idx, &(name, ro, destr, idemp, open_w)) in expected.iter().enumerate() {
        assert_eq!(gui_defs[idx].name, name);
        assert_eq!(gui_only_defs[idx].name, name);
        assert_eq!(wire_tools[idx]["name"], json!(name));

        let expected_json = json!({
            "readOnlyHint": ro,
            "destructiveHint": destr,
            "idempotentHint": idemp,
            "openWorldHint": open_w,
        });
        assert_eq!(serde_json::to_value(gui_defs[idx].annotations).unwrap(), expected_json, "list_gui_tools annotation mismatch for {name}");
        assert_eq!(serde_json::to_value(gui_only_defs[idx].annotations).unwrap(), expected_json, "gui-only list_tools_for_state mismatch for {name}");
        assert_eq!(wire_tools[idx]["annotations"], expected_json, "wire tools/list mismatch for {name}");

        let exposed = full_defs.iter().find(|t| t.name == name).expect("enabled gui tool missing");
        assert_eq!(serde_json::to_value(exposed.annotations).unwrap(), expected_json, "enabled gui toolset mismatch for {name}");
    }
}
