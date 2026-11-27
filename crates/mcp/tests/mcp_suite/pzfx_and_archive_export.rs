//! MCP integration tests for Wave 3 Track 3B (`§4.2`, `§6.1`, `§6.3`):
//! - `save_file` and `export` schemas include `"pzfx"`, experimental archive format, and `experimental` opt-in
//! - `.pzfx` export returns structured warnings, writes atomically, never mutates `file_path` or `dirty`, and round-trips
//! - `.prism` export refuses without `experimental: true` pointing to `.pzfx` and succeeds when opted in
//! - `WorkdirGuard` path containment enforced on `.pzfx` and `.prism` destinations
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ConstantValue, DataSheet, GraphSheet, InfoSheet, ProjectDocument, ResultSheet, SheetId, SubcolumnMode,
    TableArchetype, XyTable,
};
#[cfg(not(target_arch = "wasm32"))]
use curvebook_engine::Caller;
use curvebook_format::registry::DESC_PRISM;
use curvebook_format::{compare_pzfx_data_layer, import_pzfx_bytes, save_project_file};
use curvebook_mcp::{McpServerState, ToolCallError, call_tool, list_tools};
#[cfg(not(target_arch = "wasm32"))]
use curvebook_platform::sync::Mutex;
use serde_json::{Value, json};

use super::common::{TempDir, parse_result_json};

fn seed_workdir_session(dir: &TempDir) -> (McpServerState, ProjectDocument, std::path::PathBuf) {
    let mut doc = ProjectDocument::new("MCP Export Surface Study").unwrap();
    let mut info = InfoSheet::new_preseeded("info-1", "Protocol").unwrap();
    info.set_constant("Date", ConstantValue::Date(String::from("2026-09-01")), None).unwrap();
    doc.add_info_sheet(info).unwrap();

    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 2).unwrap();
    xy.x_cells[0][0] = Cell::number(1.0);
    xy.x_cells[1][0] = Cell::number(2.0);
    xy.data_sets[0].cells[0][0] = Cell::number(5.0);
    xy.data_sets[0].cells[1][0] = Cell::number(10.0);
    let ds = DataSheet::new("xy-1", "Response", TableArchetype::Xy(xy)).unwrap();
    let gs = GraphSheet::new_uninitialized_for_data(SheetId::new("gr-1"), &ds).unwrap();
    doc.add_data_sheet(ds).unwrap();
    doc.add_graph_sheet(gs).unwrap();
    doc.add_result_sheet(ResultSheet::new(AnalysisSpec::new("res-1", "xy-1", AnalysisKind::DescriptiveStats(Default::default())).unwrap()).unwrap())
        .unwrap();

    let cb_path = dir.child("study.curvebook");
    save_project_file(&cb_path, &doc).unwrap();

    let mut state = McpServerState::with_workdir(dir.path()).unwrap();
    let opened = call_tool(&mut state, "open_file", json!({ "path": "study.curvebook" })).unwrap();
    assert!(!opened.is_error);

    // Edit one cell so `dirty == true` while `file_path == Some(cb_path)`.
    let edit = call_tool(
        &mut state,
        "command_run",
        json!({ "command_id": "table.setCell", "params": { "sheet_id": "xy-1", "row": 0, "col": 0, "value": 6.5 } }),
    )
    .unwrap();
    assert!(!edit.is_error);
    assert!(state.session().is_dirty());
    let expected_doc = state.session().document().clone();
    (state, expected_doc, cb_path)
}

#[test]
fn mcp_save_file_and_export_tools_expose_pzfx_and_experimental_archive_in_schemas() {
    let tools = list_tools();
    for tool_name in ["save_file", "export"] {
        let def = tools.iter().find(|t| t.name == tool_name).unwrap_or_else(|| panic!("missing tool {tool_name}"));
        let enums: Vec<&str> =
            def.input_schema.pointer("/properties/format/enum").and_then(Value::as_array).unwrap().iter().filter_map(Value::as_str).collect();
        assert!(enums.contains(&"pzfx"), "{tool_name} format enum missing pzfx: {enums:?}");
        assert!(enums.contains(&DESC_PRISM.id), "{tool_name} format enum missing archive format: {enums:?}");
        assert_eq!(def.input_schema.pointer("/properties/experimental/type"), Some(&json!("boolean")));
    }
}

#[test]
fn mcp_save_file_and_export_pzfx_preserve_file_path_dirty_and_roundtrip_with_warnings() {
    let dir = TempDir::new_canonical("mcp-pzfx").unwrap();
    let (mut state, expected_doc, cb_path) = seed_workdir_session(&dir);

    // 1. `save_file` without explicit `path` for `pzfx` refuses so it can never overwrite `.curvebook`.
    let missing_path = call_tool(&mut state, "save_file", json!({ "format": "pzfx" }));
    assert!(matches!(missing_path, Err(ToolCallError::InvalidParams(_))));

    // 2. `save_file` with `.pzfx` path writes atomically, returns warnings, keeps `file_path` & `dirty` intact.
    let saved = call_tool(&mut state, "save_file", json!({ "path": "exports/data_only.pzfx" })).unwrap();
    assert!(!saved.is_error, "save_file .pzfx failed: {:?}", saved.structured_content);
    let saved_json = parse_result_json(&saved).unwrap();
    assert_eq!(saved_json["saved"], json!(true));
    assert_eq!(saved_json["format"], json!("pzfx"));
    assert!(!saved_json["warnings"].as_array().unwrap().is_empty());
    assert!(saved_json["notice"].as_str().unwrap().contains("not tested in other applications"));
    assert_eq!(state.session().file_path(), Some(cb_path.as_path()));
    assert!(state.session().is_dirty());

    let disk_bytes = fs::read(dir.child("exports/data_only.pzfx")).unwrap();
    let re = import_pzfx_bytes(&disk_bytes).unwrap();
    assert!(compare_pzfx_data_layer(&expected_doc, &re.document).is_identical());

    // 3. `export` to disk and inline both work and preserve session state.
    let exp_disk = call_tool(&mut state, "export", json!({ "format": "pzfx", "output_path": "exports/via_export.pzfx" })).unwrap();
    assert!(!exp_disk.is_error);
    assert_eq!(fs::read(dir.child("exports/via_export.pzfx")).unwrap(), disk_bytes);

    let exp_inline = call_tool(&mut state, "export", json!({ "format": "pzfx" })).unwrap();
    assert!(!exp_inline.is_error);
    let inline_json = parse_result_json(&exp_inline).unwrap();
    assert_eq!(inline_json["xml"].as_str().unwrap().as_bytes(), disk_bytes.as_slice());
    assert!(!inline_json["warnings"].as_array().unwrap().is_empty());

    // 4. Workdir escape is blocked.
    let esc = call_tool(&mut state, "save_file", json!({ "path": "../escape.pzfx" })).unwrap();
    assert!(esc.is_error);
    let esc_exp = call_tool(&mut state, "export", json!({ "format": "pzfx", "path": "../escape2.pzfx" })).unwrap();
    assert!(esc_exp.is_error);
}

#[test]
fn mcp_save_file_and_export_gate_experimental_archive_and_preserve_session_state() {
    let dir = TempDir::new_canonical("mcp-archive").unwrap();
    let (mut state, _expected_doc, cb_path) = seed_workdir_session(&dir);

    // 1. Default (without `experimental: true`) refuses cleanly pointing to `.pzfx`.
    let refused_save = call_tool(&mut state, "save_file", json!({ "path": "refused.prism" })).unwrap();
    assert!(refused_save.is_error);
    assert!(!dir.child("refused.prism").exists());
    let refused_save_json = parse_result_json(&refused_save).unwrap();
    assert!(refused_save_json.to_string().contains(".pzfx"), "expected .pzfx guidance: {refused_save_json}");

    let refused_exp = call_tool(&mut state, "export", json!({ "format": DESC_PRISM.id, "path": "refused2.prism" })).unwrap();
    assert!(refused_exp.is_error);
    assert!(!dir.child("refused2.prism").exists());

    // 2. Explicit `experimental: true` succeeds on disk and inline without touching `file_path` or `dirty`.
    let ok_save = call_tool(&mut state, "save_file", json!({ "path": "allowed.prism", "experimental": true })).unwrap();
    assert!(!ok_save.is_error, "experimental save_file failed: {:?}", ok_save.structured_content);
    let ok_save_json = parse_result_json(&ok_save).unwrap();
    assert_eq!(ok_save_json["format"], json!(DESC_PRISM.id));
    assert_eq!(ok_save_json["experimental"], json!(true));
    assert!(!ok_save_json["warnings"].as_array().unwrap().is_empty());
    assert!(dir.child("allowed.prism").is_file());
    assert_eq!(state.session().file_path(), Some(cb_path.as_path()));
    assert!(state.session().is_dirty());

    let ok_inline = call_tool(&mut state, "export", json!({ "format": DESC_PRISM.id, "experimental": true })).unwrap();
    assert!(!ok_inline.is_error);
    let ok_inline_json = parse_result_json(&ok_inline).unwrap();
    assert_eq!(ok_inline_json["encoding"], json!("base64"));
    assert!(!ok_inline_json["data"].as_str().unwrap().is_empty());
}

#[cfg(not(target_arch = "wasm32"))]
fn live_attach_state_for_session(desktop: &Arc<Mutex<curvebook_engine::Session>>) -> McpServerState {
    let desktop_handler = Arc::clone(desktop);
    McpServerState::with_control_port(49341).unwrap().with_control_handler(move |method, params| match method {
        "engine.execute" => {
            let cmd_id = params.get("command_id").and_then(Value::as_str).ok_or_else(|| String::from("missing command_id"))?;
            let cmd_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
            let cancel = std::sync::atomic::AtomicBool::new(false);
            let mut guard = desktop_handler.lock_or_recover().map_err(|e| format!("lock: {e}"))?;
            match guard.execute_with_caller(cmd_id, cmd_params, Caller::Control, &cancel) {
                Ok(outcome) => Ok(json!({
                    "command_id": cmd_id,
                    "mutated": outcome.mutated,
                    "payload": outcome.payload,
                })),
                Err(err) => Err(err.message().to_owned()),
            }
        }
        other => Err(format!("unexpected live method {other}")),
    })
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn mcp_live_attach_pzfx_save_file_and_export_unwrap_payload() {
    let dir = TempDir::new_canonical("mcp-live-pzfx").unwrap();
    let (headless_seed, expected_doc, cb_path) = seed_workdir_session(&dir);
    let live_session = headless_seed.session().clone();
    let desktop = Arc::new(Mutex::named("test.live_pzfx.session", live_session));
    let mut state = live_attach_state_for_session(&desktop);

    let pzfx_path = dir.child("live_out.pzfx");
    let saved = call_tool(&mut state, "save_file", json!({ "path": pzfx_path.display().to_string() })).unwrap();
    assert!(!saved.is_error, "live save_file .pzfx failed: {:?}", saved.structured_content);
    let saved_json = parse_result_json(&saved).unwrap();
    assert_eq!(saved_json["mode"], json!("live_attach"));
    assert_eq!(saved_json["saved"], json!(true));
    assert_eq!(saved_json["format"], json!("pzfx"));
    assert!(saved_json["preflight"].is_object(), "expected preflight object: {saved_json}");
    assert_eq!(saved_json["data_sheet_count"], json!(1));
    assert_eq!(saved_json["info_sheet_count"], json!(1));
    assert!(saved_json["total_dropped_count"].as_u64().unwrap_or(0) >= 2);
    assert!(!saved_json["warnings"].as_array().unwrap().is_empty(), "expected structured warnings: {saved_json}");

    let disk_bytes = fs::read(&pzfx_path).unwrap();
    assert!(!disk_bytes.is_empty() && saved_json["bytes_written"].as_u64() == Some(disk_bytes.len() as u64));
    let re = import_pzfx_bytes(&disk_bytes).unwrap();
    assert!(compare_pzfx_data_layer(&expected_doc, &re.document).is_identical());

    let inline = call_tool(&mut state, "export", json!({ "format": "pzfx" })).unwrap();
    assert!(!inline.is_error, "live inline export .pzfx failed: {:?}", inline.structured_content);
    let inline_json = parse_result_json(&inline).unwrap();
    assert_eq!(inline_json["mode"], json!("live_attach"));
    assert_eq!(inline_json["exported"], json!(true));
    assert_eq!(inline_json["xml"].as_str().unwrap().as_bytes(), disk_bytes.as_slice());
    assert!(inline_json["preflight"].is_object() && !inline_json["warnings"].as_array().unwrap().is_empty());

    let guard = desktop.lock_or_recover().unwrap();
    assert_eq!(guard.file_path(), Some(cb_path.as_path()));
    assert!(guard.is_dirty());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn mcp_live_attach_experimental_archive_export_unwraps_payload() {
    let dir = TempDir::new_canonical("mcp-live-archive").unwrap();
    let (headless_seed, _expected_doc, cb_path) = seed_workdir_session(&dir);
    let live_session = headless_seed.session().clone();
    let desktop = Arc::new(Mutex::named("test.live_archive.session", live_session));
    let mut state = live_attach_state_for_session(&desktop);

    let refused = call_tool(&mut state, "export", json!({ "format": DESC_PRISM.id })).unwrap();
    assert!(refused.is_error);
    assert!(parse_result_json(&refused).unwrap().to_string().contains(".pzfx"));

    let exp_inline = call_tool(&mut state, "export", json!({ "format": DESC_PRISM.id, "experimental": true })).unwrap();
    assert!(!exp_inline.is_error, "live experimental archive export failed: {:?}", exp_inline.structured_content);
    let exp_json = parse_result_json(&exp_inline).unwrap();
    assert_eq!(exp_json["mode"], json!("live_attach"));
    assert_eq!(exp_json["experimental"], json!(true));
    assert_eq!(exp_json["encoding"], json!("base64"));
    assert!(exp_json["bytes_len"].as_u64().unwrap_or(0) > 0);
    assert!(!exp_json["data"].as_str().unwrap().is_empty());
    assert!(!exp_json["warnings"].as_array().unwrap().is_empty());

    let guard = desktop.lock_or_recover().unwrap();
    assert_eq!(guard.file_path(), Some(cb_path.as_path()));
    assert!(guard.is_dirty());
}
