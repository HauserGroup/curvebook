//! Permanent Round 2 (`R0.19`) Slice `S12` audit regression suite for `lumencraft-mcp`.
//!
//! Covers:
//! - `R2-S12-01`: `--control-port` `ping` handshake & bidirectional `--gui-only` policy enforcement.
//! - `R2-S12-03`: `ui_invoke_menu` always supplies `params: {}` (never `null`/omitted) and blocks OS
//!   subprocess helpers / `submit_to_os` under `--gui-only`.
//! - `R2-S12-04`: `open_file` and `save_file` enforce `WorkdirGuard` before probing raw host paths
//!   or inferring format from unguarded host directories.
//! - `R2-S12-06`: Remove ignored `ui_inspect.sheet_id` and `ui_answer_dialog.dialog_id` schema drift.
//! - `R2-S12-07`: `McpServerState::with_gui_host` shares the hosted `Session`, propagates changes via
//!   `sync_after_session_mutation`, and wraps `resources/*` inside `run_guarded`.
//! - `R2-S12-09`: `prompts/list` + `prompts/get` serve GUI-only workflows when `--gui-only` is active.
#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use lumencraft_doc::{ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, TableArchetype};
use lumencraft_engine::{EngineError, Session};
use lumencraft_format::{FormatRegistry, save_project_unpacked_dir};
use lumencraft_mcp::{
    GUI_ONLY_SERVER_INSTRUCTIONS, GuiSessionHost, McpServerState, SERVER_INSTRUCTIONS, ToolCallError, call_tool, dispatch_json_value, list_gui_tools,
    list_tools_for_state,
};
use serde_json::{Value, json};

type TestResult = Result<(), String>;

static TEMP_SEQ: AtomicUsize = AtomicUsize::new(1);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("lc-r2-s12-mcp-{label}-{}-{seq}", std::process::id()));
        if dir.exists() {
            let _ = fs::remove_dir_all(&dir);
        }
        fs::create_dir_all(&dir).map_err(|err| format!("create temp dir `{}`: {err}", dir.display()))?;
        Ok(Self { path: dir })
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Default)]
struct RecordingHostState {
    gui_only: bool,
    calls: Vec<(String, Value)>,
}

struct RecordingHost(Rc<RefCell<RecordingHostState>>);

impl RecordingHost {
    fn new(gui_only: bool) -> (Self, Rc<RefCell<RecordingHostState>>) {
        let state = Rc::new(RefCell::new(RecordingHostState { gui_only, calls: Vec::new() }));
        (Self(Rc::clone(&state)), state)
    }
}

impl GuiSessionHost for RecordingHost {
    fn call_gui(&mut self, method: &str, params: Value) -> Result<Value, EngineError> {
        let mut guard = self.0.borrow_mut();
        guard.calls.push((method.to_owned(), params));
        Ok(json!({
            "method": method,
            "gui_only": guard.gui_only,
        }))
    }

    fn set_gui_only(&mut self, gui_only: bool) {
        self.0.borrow_mut().gui_only = gui_only;
    }

    fn is_gui_only(&self) -> bool {
        self.0.borrow().gui_only
    }
}

struct OwnedSessionHost {
    gui_only: bool,
    session: Session,
    sync_count: Arc<AtomicUsize>,
}

impl OwnedSessionHost {
    fn new(gui_only: bool) -> (Self, Arc<AtomicUsize>) {
        let sync_count = Arc::new(AtomicUsize::new(0));
        (Self { gui_only, session: Session::new(), sync_count: Arc::clone(&sync_count) }, sync_count)
    }
}

impl GuiSessionHost for OwnedSessionHost {
    fn call_gui(&mut self, method: &str, _params: Value) -> Result<Value, EngineError> {
        let sheets: Vec<Value> = self.session.document().data_sheets.iter().map(|d| json!({ "sheet_id": d.id.as_str(), "title": d.title })).collect();
        Ok(json!({ "method": method, "gui_only": self.gui_only, "navigator_sheets": sheets }))
    }

    fn set_gui_only(&mut self, gui_only: bool) {
        self.gui_only = gui_only;
    }

    fn is_gui_only(&self) -> bool {
        self.gui_only
    }

    fn hosted_session(&self) -> Option<&Session> {
        Some(&self.session)
    }

    fn hosted_session_mut(&mut self) -> Option<&mut Session> {
        Some(&mut self.session)
    }

    fn sync_after_session_mutation(&mut self) {
        self.sync_count.fetch_add(1, Ordering::Relaxed);
    }
}

fn rpc_ok(state: &mut McpServerState, id: i64, method: &str, params: Value) -> Result<Value, String> {
    dispatch_json_value(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }), state)
        .ok_or_else(|| format!("missing JSON-RPC reply for {method}"))
}

fn initialize_payload() -> Value {
    json!({
        "protocolVersion": "2025-03-26",
        "capabilities": {},
        "clientInfo": { "name": "audit-r2-s12", "version": "0.1.0" }
    })
}

#[test]
fn r2_s12_01_control_port_bidirectional_gui_only_handshake() -> TestResult {
    // 1. Unlocked MCP (`gui_only = false`) attaches to remote host reporting `gui_only: true`.
    let remote_calls = Arc::new(AtomicUsize::new(0));
    let remote_calls_a = Arc::clone(&remote_calls);
    let mut unlocked_client =
        McpServerState::with_control_port(49401).map_err(|e| format!("with_control_port: {e}"))?.with_control_handler(move |method, _params| {
            remote_calls_a.fetch_add(1, Ordering::Relaxed);
            match method {
                "ping" => Ok(json!({ "pong": true, "gui_only": true })),
                "ui.inspect" => Ok(json!({ "gui_only": true, "scope": "all" })),
                other => Err(format!("unexpected remote method {other}")),
            }
        });

    let init = rpc_ok(&mut unlocked_client, 1, "initialize", initialize_payload())?;
    assert_eq!(init["result"]["instructions"], json!(GUI_ONLY_SERVER_INSTRUCTIONS), "init did not adopt remote gui_only: {init}");
    assert!(unlocked_client.is_gui_only());

    let tools = rpc_ok(&mut unlocked_client, 2, "tools/list", json!({}))?;
    let tool_arr = tools["result"]["tools"].as_array().ok_or_else(|| format!("missing tools array: {tools}"))?;
    assert_eq!(tool_arr.len(), 8, "expected 8 GUI tools after adopting remote gui_only: {tools}");

    for tool_name in ["doc_inspect", "open_file", "enable_toolset"] {
        let res = call_tool(&mut unlocked_client, tool_name, json!({}))
            .map_err(|e| format!("expected structured gui_only_violation for {tool_name}: {e}"))?;
        assert!(res.is_error, "{tool_name} should error under adopted --gui-only");
        let code = res.structured_content.as_ref().and_then(|v| v.get("code")).and_then(Value::as_str);
        assert_eq!(code, Some("gui_only_violation"), "wrong error code for {tool_name}: {:?}", res.structured_content);
    }

    let rlist = rpc_ok(&mut unlocked_client, 3, "resources/list", json!({}))?;
    assert_eq!(rlist["result"]["resources"], json!([]));
    let rread = rpc_ok(&mut unlocked_client, 4, "resources/read", json!({ "uri": "sheet://d1" }))?;
    assert!(rread.get("error").is_some(), "resources/read should fail under adopted --gui-only: {rread}");

    // 2. Local `--gui-only` attaching to unlocked remote (`gui_only: false`) must fail closed.
    let non_ping_called = Arc::new(AtomicBool::new(false));
    let non_ping_flag = Arc::clone(&non_ping_called);
    let mut locked_client = McpServerState::with_control_port_and_gui_only(49402, true)
        .map_err(|e| format!("with_control_port_and_gui_only: {e}"))?
        .with_control_handler(move |method, _params| {
            if method != "ping" {
                non_ping_flag.store(true, Ordering::Relaxed);
            }
            match method {
                "ping" => Ok(json!({ "pong": true, "gui_only": false })),
                _ => Ok(json!({})),
            }
        });

    let init_err = rpc_ok(&mut locked_client, 10, "initialize", initialize_payload())?;
    assert_eq!(init_err["error"]["data"]["code"], json!("gui_only_violation"), "expected mismatch error on initialize: {init_err}");
    let list_err = rpc_ok(&mut locked_client, 11, "tools/list", json!({}))?;
    assert_eq!(list_err["error"]["data"]["code"], json!("gui_only_violation"), "expected mismatch error on tools/list: {list_err}");
    let call_err = call_tool(&mut locked_client, "ui_inspect", json!({})).map_err(|e| format!("ui_inspect call failed: {e}"))?;
    assert!(call_err.is_error);
    assert_eq!(call_err.structured_content.as_ref().and_then(|v| v.get("code")), Some(&json!("gui_only_violation")));
    assert!(!non_ping_called.load(Ordering::Relaxed), "ui.inspect must never reach unlocked remote host");

    // 3. Handshake failing / omitting boolean `gui_only` must fail closed.
    let mut broken_ping = McpServerState::with_control_port(49403)
        .map_err(|e| format!("with_control_port: {e}"))?
        .with_control_handler(|_method, _params| Ok(json!({ "pong": true })));
    let bad_init = rpc_ok(&mut broken_ping, 20, "initialize", initialize_payload())?;
    assert!(bad_init.get("error").is_some(), "missing `gui_only` in ping must fail closed: {bad_init}");
    let bad_doc = call_tool(&mut broken_ping, "doc_inspect", json!({})).map_err(|e| format!("doc_inspect failed: {e}"))?;
    assert!(bad_doc.is_error);

    let mut normal = McpServerState::new();
    let init_normal = rpc_ok(&mut normal, 21, "initialize", initialize_payload())?;
    assert_eq!(init_normal["result"]["instructions"], json!(SERVER_INSTRUCTIONS));
    Ok(())
}

#[test]
fn r2_s12_01_control_port_tcp_loopback_handshake_when_permitted() -> TestResult {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(err) if err.kind() == ErrorKind::PermissionDenied => return Ok(()),
        Err(err) => return Err(format!("bind failed: {err}")),
    };
    let addr = listener.local_addr().map_err(|e| format!("local_addr: {e}"))?;
    let port = addr.port();
    if let Err(err) = TcpStream::connect_timeout(&addr, Duration::from_millis(200)) {
        if err.kind() == ErrorKind::PermissionDenied {
            return Ok(());
        }
        return Err(format!("probe connect: {err}"));
    }
    let _ = listener.accept();

    let (methods_tx, methods_rx) = mpsc::channel::<String>();
    let server = thread::spawn(move || -> Result<(), String> {
        for expected in 0..2 {
            let (mut stream, _) = listener.accept().map_err(|e| format!("accept {expected}: {e}"))?;
            let mut reader = BufReader::new(stream.try_clone().map_err(|e| format!("clone: {e}"))?);
            let mut line = String::new();
            reader.read_line(&mut line).map_err(|e| format!("read_line: {e}"))?;
            let req: Value = serde_json::from_str(&line).map_err(|e| format!("json: {e}"))?;
            let id = req["id"].as_str().unwrap_or("mcp-1");
            let method = req["method"].as_str().unwrap_or("");
            methods_tx.send(method.to_owned()).map_err(|e| format!("send: {e}"))?;
            let resp = match method {
                "ping" => json!({ "id": id, "ok": true, "result": { "pong": true, "gui_only": true } }),
                "ui.inspect" => json!({ "id": id, "ok": true, "result": { "scope": "all", "gui_only": true } }),
                _ => json!({ "id": id, "ok": false, "error": "unexpected method" }),
            };
            let wire = format!("{resp}\n");
            stream.write_all(wire.as_bytes()).and_then(|()| stream.flush()).map_err(|e| format!("write: {e}"))?;
        }
        Ok(())
    });

    let mut attached = McpServerState::with_control_port(port).map_err(|e| format!("with_control_port: {e}"))?;
    let init = rpc_ok(&mut attached, 1, "initialize", initialize_payload())?;
    assert_eq!(init["result"]["instructions"], json!(GUI_ONLY_SERVER_INSTRUCTIONS));
    assert!(attached.is_gui_only());

    let doc_blocked = call_tool(&mut attached, "doc_inspect", json!({})).map_err(|e| format!("doc_inspect: {e}"))?;
    assert!(doc_blocked.is_error);
    assert_eq!(doc_blocked.structured_content.as_ref().and_then(|v| v.get("code")), Some(&json!("gui_only_violation")));

    let insp = call_tool(&mut attached, "ui_inspect", json!({ "scope": "all" })).map_err(|e| format!("ui_inspect: {e}"))?;
    assert!(!insp.is_error);
    server.join().map_err(|_| String::from("server thread panicked"))??;

    let observed: Vec<String> = methods_rx.iter().collect();
    assert_eq!(observed, vec!["ping", "ui.inspect"]);
    Ok(())
}

#[test]
fn r2_s12_03_ui_invoke_menu_params_object_and_gui_only_subprocess_blocks() -> TestResult {
    // 1. Omitted / null `params` must always be forwarded as `{}` (`Value::Object`).
    let (host, shared) = RecordingHost::new(false);
    let mut state = McpServerState::with_gui_host(Box::new(host));

    let res1 = call_tool(&mut state, "ui_invoke_menu", json!({ "path": ["File", "New Project"] })).map_err(|e| format!("invoke 1: {e}"))?;
    assert!(!res1.is_error);
    let res2 = call_tool(&mut state, "ui_invoke_menu", json!({ "command_id": "edit.undo", "params": null })).map_err(|e| format!("invoke 2: {e}"))?;
    assert!(!res2.is_error);

    let recorded = shared.borrow();
    assert_eq!(recorded.calls.len(), 2);
    assert_eq!(recorded.calls[0].1["params"], json!({}), "omitted params must serialize as empty JSON object");
    assert_eq!(recorded.calls[1].1["params"], json!({}), "null params must serialize as empty JSON object");
    drop(recorded);

    // 2. Under `--gui-only`, OS subprocess helpers & `submit_to_os` are blocked before reaching host.
    state.set_gui_only(true);
    shared.borrow_mut().calls.clear();

    let blocked_cases = [
        ("ui_invoke_menu", json!({ "command_id": "ui.result.copyMethodsReport" })),
        ("ui_invoke_menu", json!({ "path": ["Analyze", "Copy Methods Report"] })),
        ("ui_invoke_menu", json!({ "command_id": "file.print" })),
        ("ui_invoke_menu", json!({ "command_id": "ui.print.open", "params": { "submit_to_os": true } })),
        ("ui_invoke_menu", json!({ "command_id": "ui.screenshot", "params": { "path": "forbidden.png" } })),
        ("ui_answer_dialog", json!({ "action": "ok", "fields": { "submit_to_os": true } })),
    ];

    for (tool, args) in blocked_cases {
        let res = call_tool(&mut state, tool, args.clone()).map_err(|e| format!("blocked case {tool} failed: {e}"))?;
        assert!(res.is_error, "expected gui_only_violation for {tool} {args}");
        assert_eq!(
            res.structured_content.as_ref().and_then(|v| v.get("code")),
            Some(&json!("gui_only_violation")),
            "wrong error code for {tool} {args}: {:?}",
            res.structured_content
        );
    }
    assert!(shared.borrow().calls.is_empty(), "blocked helpers must never invoke GuiSessionHost");
    Ok(())
}

#[test]
fn r2_s12_04_open_and_save_check_workdir_guard_before_host_fs_probe() -> TestResult {
    let dir = TempDir::new("workdir-preguard")?;
    let workdir = dir.path.join("wd");
    let outside = dir.path.join("outside");
    fs::create_dir_all(&workdir).map_err(|e| e.to_string())?;
    fs::create_dir_all(&outside).map_err(|e| e.to_string())?;

    // Create a valid unpacked project directory WITHOUT `.lumencraft.d` suffix inside BOTH
    // `outside/secret_study` and `workdir/rel_study`, plus a plain directory `outside/plain_dir`.
    let mut doc = ProjectDocument::new("Pre-Guard Study").map_err(|e| e.to_string())?;
    let ds = DataSheet::new(
        "d1",
        "Assay",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2).map_err(|e| e.to_string())?),
    )
    .map_err(|e| e.to_string())?;
    doc.data_sheets.push(ds);

    let outside_study = outside.join("secret_study");
    save_project_unpacked_dir(&doc, &outside_study).map_err(|e| format!("save outside study: {e}"))?;
    let outside_plain = outside.join("plain_dir");
    fs::create_dir_all(&outside_plain).map_err(|e| e.to_string())?;

    let inside_study = workdir.join("rel_study");
    save_project_unpacked_dir(&doc, &inside_study).map_err(|e| format!("save inside study: {e}"))?;

    let mut state = McpServerState::with_workdir(&workdir).map_err(|e| format!("with_workdir: {e}"))?;

    // Probing external directories without extensions must yield identical `path_escape` (no probe oracle).
    for outside_target in [&outside_study, &outside_plain] {
        let open_res = call_tool(&mut state, "open_file", json!({ "path": outside_target.display().to_string() }))
            .map_err(|e| format!("open_file outside returned protocol err instead of path_escape: {e}"))?;
        assert!(open_res.is_error);
        assert_eq!(
            open_res.structured_content.as_ref().and_then(|v| v.get("code")),
            Some(&json!("path_escape")),
            "expected path_escape for open_file on {}: {:?}",
            outside_target.display(),
            open_res.structured_content
        );

        let save_res = call_tool(&mut state, "save_file", json!({ "path": outside_target.display().to_string() }))
            .map_err(|e| format!("save_file outside returned protocol err instead of path_escape: {e}"))?;
        assert!(save_res.is_error);
        assert_eq!(
            save_res.structured_content.as_ref().and_then(|v| v.get("code")),
            Some(&json!("path_escape")),
            "expected path_escape for save_file on {}: {:?}",
            outside_target.display(),
            save_res.structured_content
        );
    }

    // Relative unpacked project without `.lumencraft.d` suffix inside `--workdir` resolves via `&resolved_path`.
    let rel_open = call_tool(&mut state, "open_file", json!({ "path": "rel_study" })).map_err(|e| format!("relative open_file failed: {e}"))?;
    assert!(!rel_open.is_error, "expected relative unpacked workdir study to open cleanly: {:?}", rel_open.structured_content);
    let _ = FormatRegistry::default_registry();

    // `save_file { "path": "." }` inside workdir must fail with InvalidParams rather than matching `lumencraft.d`.
    let dot_err = call_tool(&mut state, "save_file", json!({ "path": "." }));
    assert!(matches!(dot_err, Err(ToolCallError::InvalidParams(_))), "expected InvalidParams for save_file `.`, got {dot_err:?}");
    Ok(())
}

#[test]
fn r2_s12_06_ui_inspect_and_ui_answer_dialog_drop_ignored_schema_drift() -> TestResult {
    let tools = list_gui_tools();
    let inspect_def = tools.iter().find(|t| t.name == "ui_inspect").ok_or_else(|| String::from("missing ui_inspect"))?;
    assert!(inspect_def.input_schema["properties"].get("sheet_id").is_none(), "ui_inspect must not advertise ignored `sheet_id`");

    let answer_def = tools.iter().find(|t| t.name == "ui_answer_dialog").ok_or_else(|| String::from("missing ui_answer_dialog"))?;
    assert!(answer_def.input_schema["properties"].get("dialog_id").is_none(), "ui_answer_dialog must not advertise ignored `dialog_id`");

    let (host, _) = RecordingHost::new(true);
    let mut state = McpServerState::with_gui_host(Box::new(host));

    let err_inspect = call_tool(&mut state, "ui_inspect", json!({ "scope": "all", "sheet_id": "data-1" }));
    assert!(matches!(err_inspect, Err(ToolCallError::InvalidParams(_))), "expected InvalidParams for ui_inspect sheet_id: {err_inspect:?}");

    let err_answer = call_tool(&mut state, "ui_answer_dialog", json!({ "action": "ok", "dialog_id": "dlg-1" }));
    assert!(matches!(err_answer, Err(ToolCallError::InvalidParams(_))), "expected InvalidParams for ui_answer_dialog dialog_id: {err_answer:?}");
    Ok(())
}

#[test]
fn r2_s12_07_hosted_session_shared_and_resources_guarded() -> TestResult {
    let dir = TempDir::new("hosted-session")?;
    let (host, sync_count) = OwnedSessionHost::new(false);
    let mut state = McpServerState::with_gui_host(Box::new(host));
    state.set_workdir(Some(&dir.path)).map_err(|e| format!("set_workdir: {e}"))?;
    assert!(!state.is_gui_only());
    assert!(list_tools_for_state(&state).len() >= 18);

    // Mutate via core `command_run` and verify `hosted_session` + GUI host inspect + resources/* see it.
    let created =
        call_tool(&mut state, "command_run", json!({ "command_id": "sheet.createData", "params": { "sheet_id": "hosted-1", "kind": "Column" } }))
            .map_err(|e| format!("command_run failed: {e}"))?;
    assert!(!created.is_error, "command_run errored: {:?}", created.structured_content);
    assert!(sync_count.load(Ordering::Relaxed) >= 1, "expected sync_after_session_mutation call");
    assert!(state.session().document().data_sheet(&lumencraft_doc::SheetId::new("hosted-1")).is_some());

    let ui_insp = call_tool(&mut state, "ui_inspect", json!({ "scope": "navigator" })).map_err(|e| format!("ui_inspect failed: {e}"))?;
    let ui_sc = ui_insp.structured_content.ok_or_else(|| String::from("missing ui_inspect structured_content"))?;
    let nav = ui_sc["navigator_sheets"].as_array().ok_or_else(|| format!("missing navigator_sheets: {ui_sc}"))?;
    assert!(nav.iter().any(|s| s["sheet_id"] == json!("hosted-1")), "hosted UI did not observe core command mutation: {ui_sc}");

    let rlist = rpc_ok(&mut state, 1, "resources/list", json!({}))?;
    let res_arr = rlist["result"]["resources"].as_array().ok_or_else(|| format!("missing resources array: {rlist}"))?;
    assert!(res_arr.iter().any(|r| r["uri"] == json!("sheet://hosted-1")), "resources/list missing hosted-1: {rlist}");

    let rread = rpc_ok(&mut state, 2, "resources/read", json!({ "uri": "sheet://hosted-1" }))?;
    assert!(rread.get("error").is_none(), "resources/read failed: {rread}");
    assert!(!rread["result"]["contents"].as_array().is_none_or(Vec::is_empty));

    // Save + export from the hosted session succeed inside `--workdir`.
    let saved = call_tool(&mut state, "save_file", json!({ "path": "hosted.lumencraft" })).map_err(|e| format!("save_file failed: {e}"))?;
    assert!(!saved.is_error, "save_file failed: {:?}", saved.structured_content);
    assert!(dir.path.join("hosted.lumencraft").is_file());
    let exported = call_tool(&mut state, "export", json!({ "format": "csv", "sheet_id": "hosted-1", "path": "hosted.csv" }))
        .map_err(|e| format!("export failed: {e}"))?;
    assert!(!exported.is_error, "export failed: {:?}", exported.structured_content);
    assert!(dir.path.join("hosted.csv").is_file());

    // Live `--control-port` `resources/list` + `resources/read` routes through live snapshot rather than empty local session.
    let mut live = McpServerState::with_control_port(49404).map_err(|e| format!("with_control_port: {e}"))?.with_control_handler(
        |method, _params| match method {
            "ping" => Ok(json!({ "pong": true, "gui_only": false })),
            "doc.inspect" => Ok(json!({
                "data_sheets": [{ "id": "live-1", "title": "Live Data", "table_kind": "Column", "rows": 2, "cols": 1 }],
                "result_sheets": [],
                "columns": ["A"],
                "cells": [[1.0], [2.0]],
            })),
            other => Err(format!("unexpected method {other}")),
        },
    );
    let live_list = rpc_ok(&mut live, 3, "resources/list", json!({}))?;
    let live_arr = live_list["result"]["resources"].as_array().ok_or_else(|| format!("missing live resources: {live_list}"))?;
    assert!(live_arr.iter().any(|r| r["uri"] == json!("sheet://live-1")), "live resources/list missed live-1: {live_list}");
    let live_read = rpc_ok(&mut live, 4, "resources/read", json!({ "uri": "sheet://live-1" }))?;
    assert!(live_read.get("error").is_none(), "live resources/read failed: {live_read}");
    Ok(())
}

#[test]
fn r2_s12_09_prompts_adapt_to_gui_only_mode() -> TestResult {
    let (host, _) = RecordingHost::new(true);
    let mut gui_state = McpServerState::with_gui_host(Box::new(host));
    let listed = rpc_ok(&mut gui_state, 1, "prompts/list", json!({}))?;
    let prompts = listed["result"]["prompts"].as_array().ok_or_else(|| format!("missing prompts array: {listed}"))?;
    assert!(!prompts.is_empty());

    let forbidden_tokens = ["doc_inspect", "enable_toolset", "command_run", "render_preview", "`export`", "sheet://", "result://"];
    for prompt_entry in prompts {
        let name = prompt_entry["name"].as_str().ok_or_else(|| String::from("prompt missing name"))?;
        let got = rpc_ok(&mut gui_state, 2, "prompts/get", json!({ "name": name }))?;
        let msgs = got["result"]["messages"].as_array().ok_or_else(|| format!("missing messages for {name}: {got}"))?;
        let body = msgs.iter().filter_map(|m| m["content"]["text"].as_str()).collect::<Vec<_>>().join("\n");
        for forbidden in forbidden_tokens {
            assert!(!body.contains(forbidden), "gui_only prompt `{name}` emitted forbidden headless reference `{forbidden}`:\n{body}");
        }
        assert!(body.contains("ui_inspect") || body.contains("ui_invoke_menu"), "gui_only prompt `{name}` should reference GUI tools:\n{body}");
    }

    // Normal headless mode keeps headless workflow prompts.
    let mut headless = McpServerState::new();
    let got_headless = rpc_ok(&mut headless, 3, "prompts/get", json!({ "name": "dose_response_curve_fit" }))?;
    let h_text = got_headless["result"]["messages"][0]["content"]["text"].as_str().unwrap_or("");
    assert!(h_text.contains("doc_inspect") && h_text.contains("render_preview"), "headless prompt missing expected guidance: {h_text}");
    Ok(())
}
