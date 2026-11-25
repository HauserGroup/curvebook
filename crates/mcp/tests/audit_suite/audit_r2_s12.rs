//! Permanent Round 2 (`R0.19`) Slice `S12` audit regression suite for `curvebook-mcp`.
//!
//! Covers:
//! - `R2-S12-03`: `ui_invoke_menu` always supplies `params: {}` (never `null`/omitted).
//! - `R2-S12-04`: `open_file` and `save_file` enforce `WorkdirGuard` before probing raw host paths
//!   or inferring format from unguarded host directories.
//! - `R2-S12-06`: Remove ignored `ui_inspect.sheet_id` and `ui_answer_dialog.dialog_id` schema drift.
//! - `R2-S12-07`: `McpServerState::with_gui_host` shares the hosted `Session`, propagates changes via
//!   `sync_after_session_mutation`, and wraps `resources/*` inside `run_guarded`.
use std::cell::RefCell;
use std::fs;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use curvebook_doc::{ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, TableArchetype};
use curvebook_engine::{EngineError, Session};
use curvebook_format::{FormatRegistry, save_project_unpacked_dir};
use curvebook_mcp::{GuiSessionHost, McpServerState, ToolCallError, call_tool, dispatch_json_value, list_gui_tools, list_tools_for_state};
use serde_json::{Value, json};

use super::common::TempDir;

type TestResult = Result<(), String>;

#[derive(Default)]
struct RecordingHostState {
    calls: Vec<(String, Value)>,
}

struct RecordingHost(Rc<RefCell<RecordingHostState>>);

impl RecordingHost {
    fn new() -> (Self, Rc<RefCell<RecordingHostState>>) {
        let state = Rc::new(RefCell::new(RecordingHostState { calls: Vec::new() }));
        (Self(Rc::clone(&state)), state)
    }
}

impl GuiSessionHost for RecordingHost {
    fn call_gui(&mut self, method: &str, params: Value) -> Result<Value, EngineError> {
        let mut guard = self.0.borrow_mut();
        guard.calls.push((method.to_owned(), params));
        Ok(json!({
            "method": method,
        }))
    }
}

struct OwnedSessionHost {
    session: Session,
    sync_count: Arc<AtomicUsize>,
}

impl OwnedSessionHost {
    fn new() -> (Self, Arc<AtomicUsize>) {
        let sync_count = Arc::new(AtomicUsize::new(0));
        (Self { session: Session::new(), sync_count: Arc::clone(&sync_count) }, sync_count)
    }
}

impl GuiSessionHost for OwnedSessionHost {
    fn call_gui(&mut self, method: &str, _params: Value) -> Result<Value, EngineError> {
        let sheets: Vec<Value> = self.session.document().data_sheets.iter().map(|d| json!({ "sheet_id": d.id.as_str(), "title": d.title })).collect();
        Ok(json!({ "method": method, "navigator_sheets": sheets }))
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

#[test]
fn r2_s12_03_ui_invoke_menu_params_object() -> TestResult {
    // Omitted / null `params` must always be forwarded as `{}` (`Value::Object`).
    let (host, shared) = RecordingHost::new();
    let mut state = McpServerState::with_gui_host(Box::new(host));

    let res1 = call_tool(&mut state, "ui_invoke_menu", json!({ "path": ["File", "New Project"] })).map_err(|e| format!("invoke 1: {e}"))?;
    assert!(!res1.is_error);
    let res2 = call_tool(&mut state, "ui_invoke_menu", json!({ "command_id": "edit.undo", "params": null })).map_err(|e| format!("invoke 2: {e}"))?;
    assert!(!res2.is_error);

    let recorded = shared.borrow();
    assert_eq!(recorded.calls.len(), 2);
    assert_eq!(recorded.calls[0].1["params"], json!({}), "omitted params must serialize as empty JSON object");
    assert_eq!(recorded.calls[1].1["params"], json!({}), "null params must serialize as empty JSON object");
    Ok(())
}

#[test]
fn r2_s12_04_open_and_save_check_workdir_guard_before_host_fs_probe() -> TestResult {
    let dir = TempDir::new("workdir-preguard")?;
    let workdir = dir.path.join("wd");
    let outside = dir.path.join("outside");
    fs::create_dir_all(&workdir).map_err(|e| e.to_string())?;
    fs::create_dir_all(&outside).map_err(|e| e.to_string())?;

    // Create a valid unpacked project directory WITHOUT `.curvebook.d` suffix inside BOTH
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

    // Relative unpacked project without `.curvebook.d` suffix inside `--workdir` resolves via `&resolved_path`.
    let rel_open = call_tool(&mut state, "open_file", json!({ "path": "rel_study" })).map_err(|e| format!("relative open_file failed: {e}"))?;
    assert!(!rel_open.is_error, "expected relative unpacked workdir study to open cleanly: {:?}", rel_open.structured_content);
    let _ = FormatRegistry::default_registry();

    // `save_file { "path": "." }` inside workdir must fail with InvalidParams rather than matching `curvebook.d`.
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

    let (host, _) = RecordingHost::new();
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
    let (host, sync_count) = OwnedSessionHost::new();
    let mut state = McpServerState::with_gui_host(Box::new(host));
    state.set_workdir(Some(&dir.path)).map_err(|e| format!("set_workdir: {e}"))?;
    assert!(list_tools_for_state(&state).len() >= 18);

    // Mutate via core `command_run` and verify `hosted_session` + GUI host inspect + resources/* see it.
    let created =
        call_tool(&mut state, "command_run", json!({ "command_id": "sheet.createData", "params": { "sheet_id": "hosted-1", "kind": "Column" } }))
            .map_err(|e| format!("command_run failed: {e}"))?;
    assert!(!created.is_error, "command_run errored: {:?}", created.structured_content);
    assert!(sync_count.load(Ordering::Relaxed) >= 1, "expected sync_after_session_mutation call");
    assert!(state.session().document().data_sheet(&curvebook_doc::SheetId::new("hosted-1")).is_some());

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
    let saved = call_tool(&mut state, "save_file", json!({ "path": "hosted.curvebook" })).map_err(|e| format!("save_file failed: {e}"))?;
    assert!(!saved.is_error, "save_file failed: {:?}", saved.structured_content);
    assert!(dir.path.join("hosted.curvebook").is_file());
    let exported = call_tool(&mut state, "export", json!({ "format": "csv", "sheet_id": "hosted-1", "path": "hosted.csv" }))
        .map_err(|e| format!("export failed: {e}"))?;
    assert!(!exported.is_error, "export failed: {:?}", exported.structured_content);
    assert!(dir.path.join("hosted.csv").is_file());

    // Live `--control-port` `resources/list` + `resources/read` routes through live snapshot rather than empty local session.
    let mut live = McpServerState::with_control_port(49404).map_err(|e| format!("with_control_port: {e}"))?.with_control_handler(
        |method, _params| match method {
            "ping" => Ok(json!({ "pong": true })),
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
fn r020_legacy_lumencraft_formats_are_rejected_without_aliases() -> TestResult {
    let dir = TempDir::new("legacy-reject")?;
    let legacy_proj = dir.path.join("legacy.lumencraft");
    let legacy_tpl = dir.path.join("legacy.lctemplate");
    let legacy_dir = dir.path.join("legacy.lumencraft.d");
    fs::write(&legacy_proj, b"not-a-curvebook-archive").map_err(|e| e.to_string())?;
    fs::write(&legacy_tpl, b"not-a-curvebook-template").map_err(|e| e.to_string())?;
    fs::create_dir_all(&legacy_dir).map_err(|e| e.to_string())?;

    let mut state = McpServerState::new();
    for fmt in ["lumencraft", "lumencraft.d", "lctemplate"] {
        let save_err = call_tool(&mut state, "save_file", json!({ "path": dir.path.join("out.curvebook").display().to_string(), "format": fmt }));
        assert!(
            matches!(save_err, Err(ToolCallError::InvalidParams(_))),
            "expected InvalidParams rejecting legacy save format `{fmt}`, got {save_err:?}"
        );

        let open_err = call_tool(&mut state, "open_file", json!({ "path": legacy_proj.display().to_string(), "format": fmt }));
        assert!(
            matches!(open_err, Err(ToolCallError::InvalidParams(_))),
            "expected InvalidParams rejecting legacy open format `{fmt}`, got {open_err:?}"
        );
    }
    for legacy_path in [&legacy_proj, &legacy_tpl, &legacy_dir] {
        let err = call_tool(&mut state, "open_file", json!({ "path": legacy_path.display().to_string() }));
        assert!(
            matches!(err, Err(ToolCallError::InvalidParams(_))),
            "expected InvalidParams rejecting legacy path {}, got {err:?}",
            legacy_path.display()
        );
    }
    Ok(())
}

#[test]
fn audit_23_mcp_server_state_rejects_combining_control_port_and_workdir() -> TestResult {
    let dir = TempDir::new("audit23-exclusive")?;

    // 1. Live control_port configured first -> set_workdir(Some(..)) must fail without mutating guard.
    let mut live_state = McpServerState::with_control_port(49291)?;
    let err = live_state.set_workdir(Some(&dir.path)).expect_err("set_workdir must fail when control_port is active");
    let msg = err.to_string();
    assert!(msg.contains("--workdir") && msg.contains("--control-port"), "unexpected error: {msg}");
    assert!(live_state.workdir_guard().is_none(), "failed set_workdir must not attach workdir_guard");

    // 2. Workdir configured first -> set_control_port(Some(..)) must fail without attaching control_port.
    let mut confined = McpServerState::with_workdir(&dir.path).map_err(|e| e.to_string())?;
    let err = confined.set_control_port(Some(49293)).expect_err("set_control_port must fail when workdir is active");
    assert!(err.contains("--control-port") && err.contains("--workdir"), "unexpected error: {err}");
    assert_eq!(confined.control_port(), None);

    // 3. GuiSessionHost with WorkdirGuard -> set_control_port(Some(..)) must also fail.
    let (host, _sync_count) = OwnedSessionHost::new();
    let mut hosted_state = McpServerState::with_gui_host(Box::new(host));
    hosted_state.set_workdir(Some(&dir.path)).map_err(|e| e.to_string())?;
    let host_err = hosted_state.set_control_port(Some(49294)).expect_err("set_control_port must fail on workdir-confined GuiSessionHost");
    assert!(host_err.contains("--control-port") && host_err.contains("--workdir"), "unexpected host error: {host_err}");
    assert_eq!(hosted_state.control_port(), None);

    // 4. Detaching one side cleanly permits attaching the other.
    live_state.set_control_port(None)?;
    live_state.set_workdir(Some(&dir.path)).map_err(|e| e.to_string())?;
    assert!(live_state.workdir_guard().is_some());

    confined.set_workdir(None).map_err(|e| e.to_string())?;
    confined.set_control_port(Some(49295))?;
    assert_eq!(confined.control_port(), Some(49295));
    Ok(())
}
