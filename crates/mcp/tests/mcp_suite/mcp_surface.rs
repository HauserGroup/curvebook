//! Wave 10 MCP surface integration tests:
//! - Progressive toolsets per `CommandGroup` + `notifications/tools/list_changed`
//! - `Effects` -> MCP `annotations` fidelity and normalized `[a-z0-9_]` per-command tools
//! - Structured `isError` + `structuredContent` error contracts
//! - `initialize` instructions & capabilities, `resources/list` + `resources/read`, `prompts/list` + `prompts/get`
//! - `WorkdirGuard` path-escape refusal across `open_file`, `save_file`, `export`, and command path params.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::rc::Rc;

use curvebook_doc::{Cell, DataSheet, ProjectDocument, SubcolumnMode, TableArchetype, XyTable};
use curvebook_engine::cmd::{CommandGroup, Effects, registry};
use curvebook_engine::{EngineError, ExternalPathPolicy, Session};
use curvebook_mcp::{
    ERR_INVALID_PARAMS, ERR_RESOURCE_NOT_FOUND, GuiSessionHost, McpServerState, ResourceError, ToolAnnotations, ToolCallError, call_tool,
    command_id_to_mcp_tool_name, list_tools, list_tools_for_group, list_tools_for_state, read_resource, serve,
};
use serde_json::{Value, json};

use super::common::{TempDir, decode_cl_frames, frame_cl, rpc_roundtrip};

fn snapshot_tool_names(tools: &[curvebook_mcp::ToolDefinition]) -> Vec<String> {
    tools.iter().map(|t| t.name.clone()).collect()
}

#[test]
fn progressive_toolsets_snapshot_per_command_group_and_list_changed_notification() {
    let mut state = McpServerState::new();

    let default_names = snapshot_tool_names(&list_tools_for_state(&state));
    assert_eq!(default_names.len(), 10);
    assert!(default_names.contains(&String::from("list_toolsets")));
    assert!(default_names.contains(&String::from("enable_toolset")));

    // Snapshot per-CommandGroup normalized tool names directly against engine registry.
    let reg = registry();
    for &group in CommandGroup::all() {
        let expected_names: Vec<String> = reg.specs_by_group(group).into_iter().map(|spec| command_id_to_mcp_tool_name(spec.id)).collect();
        let actual_names = snapshot_tool_names(&list_tools_for_group(group));
        assert_eq!(actual_names, expected_names, "group snapshot mismatch for {}", group.as_str());
    }

    // Exercise enable_toolset over framed Content-Length transport and verify list_changed notification.
    let enable_req = frame_cl(&json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "tools/call",
        "params": {
            "name": "enable_toolset",
            "arguments": { "group": "sheet" }
        }
    }))
    .expect("frame_cl");
    let mut wire_out = Vec::new();
    serve(&mut Cursor::new(enable_req), &mut wire_out, &mut state).expect("serve enable_toolset");
    let frames = decode_cl_frames(&wire_out).expect("decode frames");
    assert_eq!(frames.len(), 2, "expected response + notifications/tools/list_changed");
    assert_eq!(frames[0]["id"], json!(10));
    assert_eq!(frames[0]["result"]["isError"], json!(false));
    assert_eq!(frames[0]["result"]["structuredContent"]["changed"], json!(true));
    assert_eq!(frames[0]["result"]["structuredContent"]["enabled_groups"], json!(["sheet"]));
    assert_eq!(
        frames[1],
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/tools/list_changed"
        })
    );

    // Enabling the same group again must NOT emit a duplicate notification.
    let mut second_out = Vec::new();
    let enable_again = frame_cl(&json!({
        "jsonrpc": "2.0",
        "id": 11,
        "method": "tools/call",
        "params": {
            "name": "enable_toolset",
            "arguments": { "group": "sheet" }
        }
    }))
    .expect("frame_cl repeat");
    serve(&mut Cursor::new(enable_again), &mut second_out, &mut state).expect("serve repeat enable");
    let second_frames = decode_cl_frames(&second_out).expect("decode second frames");
    assert_eq!(second_frames.len(), 1);
    assert_eq!(second_frames[0]["result"]["structuredContent"]["changed"], json!(false));

    // tools/list now includes the core 10 + exact Sheet group tools.
    let list_reply = rpc_roundtrip(&mut state, 12, "tools/list", json!({})).expect("tools/list");
    let listed_tools = list_reply["result"]["tools"].as_array().expect("tools array");
    let sheet_count = reg.specs_by_group(CommandGroup::Sheet).len();
    assert_eq!(listed_tools.len(), 10 + sheet_count);

    // Invoke normalized per-command tool (`sheet_create_data` -> `sheet.createData`) directly.
    let call_reply = rpc_roundtrip(
        &mut state,
        13,
        "tools/call",
        json!({
            "name": "sheet_create_data",
            "arguments": {
                "sheet_id": "xy-toolset",
                "title": "Toolset Created XY",
                "kind": "xy",
                "rows": 4,
                "cols": 1
            }
        }),
    )
    .expect("sheet_create_data rpc");
    assert_eq!(call_reply["result"]["isError"], json!(false));
    assert_eq!(call_reply["result"]["structuredContent"]["command_id"], json!("sheet.createData"));
    assert_eq!(call_reply["result"]["structuredContent"]["mutated"], json!(true));

    // list_toolsets reflects enabled groups and complete command counts.
    let inv = call_tool(&mut state, "list_toolsets", json!({})).expect("list_toolsets");
    let inv_json = inv.structured_content.expect("structured inventory");
    assert_eq!(inv_json["total_engine_commands"], json!(reg.len()));
    assert_eq!(inv_json["enabled_groups"], json!(["sheet"]));
}

#[test]
fn tools_list_annotations_match_engine_effects_and_names_are_valid() {
    let mut state = McpServerState::new();
    call_tool(&mut state, "enable_toolset", json!({ "group": "all" })).expect("enable all");

    let list_reply = rpc_roundtrip(&mut state, 1, "tools/list", json!({})).expect("tools/list");
    let wire_tools = list_reply["result"]["tools"].as_array().expect("tools");
    assert_eq!(wire_tools.len(), list_tools().len() + registry().len());

    let mut unique_names = BTreeSet::new();
    for tool in wire_tools {
        let name = tool["name"].as_str().expect("tool name");
        assert!(name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'), "non-normalized name: {name}");
        assert!(unique_names.insert(name.to_owned()), "duplicate tool name: {name}");
        assert!(tool["annotations"]["readOnlyHint"].is_boolean());
        assert!(tool["annotations"]["destructiveHint"].is_boolean());
        assert!(tool["annotations"]["idempotentHint"].is_boolean());
        assert!(tool["annotations"]["openWorldHint"].is_boolean());
    }

    // Verify specific Effects -> ToolAnnotations mappings across representative commands.
    let cases = [
        ("project.inspect", Effects::NONE.with(Effects::IDEMPOTENT)),
        ("sheet.delete", Effects::MUTATES_DOCUMENT.with(Effects::MAY_DISCARD_DATA)),
        ("export.svg", Effects::WRITES_FILESYSTEM.with(Effects::IDEMPOTENT)),
        ("project.open", Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM).with(Effects::MAY_DISCARD_DATA)),
    ];
    for (cmd_id, expected_effects) in cases {
        let spec = registry().get(cmd_id).expect("spec exists");
        assert_eq!(spec.effects, expected_effects);
        let tool_name = command_id_to_mcp_tool_name(cmd_id);
        let wire = wire_tools.iter().find(|t| t["name"] == tool_name).expect("wire tool");
        let expected_ann = ToolAnnotations::from_effects(expected_effects);
        assert_eq!(wire["annotations"], serde_json::to_value(expected_ann).expect("serialize ann"), "annotations mismatch for {cmd_id}");
    }
}

#[test]
fn structured_is_error_snapshot_includes_all_canonical_fields() {
    let mut state = McpServerState::new();

    // 1. Unknown sheet ID -> structured NotFound error.
    let missing_sheet = rpc_roundtrip(
        &mut state,
        1,
        "tools/call",
        json!({
            "name": "doc_inspect",
            "arguments": { "sheet_id": "does-not-exist" }
        }),
    )
    .expect("missing sheet rpc");
    let result = &missing_sheet["result"];
    assert_eq!(result["isError"], json!(true));
    let sc = result["structuredContent"].as_object().expect("structuredContent object");
    for key in ["code", "category", "message", "retryable", "remediation", "details"] {
        assert!(sc.contains_key(key), "missing structured error key '{key}' in {sc:?}");
    }
    assert_eq!(sc["code"], json!("not_found"));
    assert_eq!(sc["category"], json!("state"));
    assert_eq!(sc["retryable"], json!(false));

    let text_json: Value = serde_json::from_str(result["content"][0]["text"].as_str().expect("error json text")).expect("parse text json");
    assert_eq!(text_json, result["structuredContent"]);

    // 2. UI command in headless mode -> structured Conflict error with remediation.
    let ui_headless = rpc_roundtrip(
        &mut state,
        2,
        "tools/call",
        json!({
            "name": "command_run",
            "arguments": { "command_id": "ui.navigator.toggle", "params": {} }
        }),
    )
    .expect("ui headless rpc");
    let ui_res = &ui_headless["result"];
    assert_eq!(ui_res["isError"], json!(true));
    assert_eq!(ui_res["structuredContent"]["code"], json!("conflict"));
    assert_eq!(ui_res["structuredContent"]["category"], json!("state"));
    assert!(ui_res["structuredContent"]["remediation"].as_str().is_some_and(|s| !s.is_empty()));
}

#[test]
fn initialize_resources_and_prompts_work_end_to_end() {
    let mut state = McpServerState::new();

    // Initialize advertises instructions, tools.listChanged, resources, and prompts.
    let init = rpc_roundtrip(
        &mut state,
        1,
        "initialize",
        json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "surface-test", "version": "1.0" }
        }),
    )
    .expect("initialize rpc");
    let init_res = &init["result"];
    assert_eq!(init_res["capabilities"]["tools"]["listChanged"], json!(true));
    assert!(init_res["capabilities"]["resources"].is_object());
    assert!(init_res["capabilities"]["prompts"].is_object());
    assert!(init_res["instructions"].as_str().is_some_and(|s| s.contains("enable_toolset")));

    // Populate an XY table + 4PL fit + linked graph so resources include sheet:// and result:// URIs.
    call_tool(
        &mut state,
        "command_run",
        json!({
            "command_id": "sheet.createData",
            "params": {
                "sheet_id": "xy-res",
                "title": "Resource Assay",
                "kind": "xy",
                "rows": 5,
                "cols": 1,
                "replicates": 1,
                "auto_family": true,
                "info_sheet_id": "info-res",
                "graph_sheet_id": "graph-res"
            }
        }),
    )
    .expect("createData");
    call_tool(
        &mut state,
        "command_run",
        json!({ "command_id": "table.setXCells", "params": { "sheet_id": "xy-res", "values": [-9.0, -8.0, -7.0, -6.0, -5.0] } }),
    )
    .expect("setX");
    call_tool(
        &mut state,
        "command_run",
        json!({
            "command_id": "table.setCells",
            "params": {
                "sheet_id": "xy-res",
                "edits": [
                    { "row": 0, "col": 0, "value": 5.0 },
                    { "row": 1, "col": 0, "value": 20.0 },
                    { "row": 2, "col": 0, "value": 50.0 },
                    { "row": 3, "col": 0, "value": 80.0 },
                    { "row": 4, "col": 0, "value": 95.0 }
                ]
            }
        }),
    )
    .expect("setCells");
    call_tool(
        &mut state,
        "command_run",
        json!({
            "command_id": "fit.run",
            "params": {
                "sheet_id": "fit-res",
                "source_sheet_id": "xy-res",
                "equation_id": "log_agonist_vs_response_4pl"
            }
        }),
    )
    .expect("fit.run");

    let list_res = rpc_roundtrip(&mut state, 2, "resources/list", json!({})).expect("resources/list");
    let resources = list_res["result"]["resources"].as_array().expect("resources array");
    let uris: Vec<&str> = resources.iter().filter_map(|r| r["uri"].as_str()).collect();
    for expected in ["sheet://xy-res", "sheet://info-res", "sheet://fit-res", "sheet://graph-res", "result://fit-res"] {
        assert!(uris.contains(&expected), "missing resource URI {expected} in {uris:?}");
    }

    let read_sheet = rpc_roundtrip(&mut state, 3, "resources/read", json!({ "uri": "sheet://xy-res" })).expect("read sheet");
    let sheet_text = read_sheet["result"]["contents"][0]["text"].as_str().expect("sheet resource text");
    let sheet_val: Value = serde_json::from_str(sheet_text).expect("sheet resource json");
    assert_eq!(sheet_val["section"], json!("data"));
    assert_eq!(sheet_val["id"], json!("xy-res"));

    let read_result = rpc_roundtrip(&mut state, 4, "resources/read", json!({ "uri": "result://fit-res" })).expect("read result");
    assert_eq!(read_result["result"]["contents"][0]["mimeType"], json!("application/json"));
    let result_text = read_result["result"]["contents"][0]["text"].as_str().expect("result resource text");
    let result_val: Value = serde_json::from_str(result_text).expect("result resource json");
    assert_eq!(result_val["id"], json!("fit-res"));
    assert!(!result_val["scalars"].as_array().expect("scalars").is_empty());

    let missing_res = rpc_roundtrip(&mut state, 5, "resources/read", json!({ "uri": "result://nope" })).expect("missing res");
    assert_eq!(missing_res["error"]["code"], json!(ERR_RESOURCE_NOT_FOUND));

    let bad_scheme = rpc_roundtrip(&mut state, 6, "resources/read", json!({ "uri": "file:///etc/passwd" })).expect("bad scheme");
    assert_eq!(bad_scheme["error"]["code"], json!(ERR_INVALID_PARAMS));

    // Prompts list + get
    let prompts_list = rpc_roundtrip(&mut state, 7, "prompts/list", json!({})).expect("prompts/list");
    let prompts = prompts_list["result"]["prompts"].as_array().expect("prompts array");
    let prompt_names: Vec<&str> = prompts.iter().filter_map(|p| p["name"].as_str()).collect();
    assert!(prompt_names.contains(&"dose_response_curve_fit"));
    assert!(prompt_names.contains(&"choose_two_group_test"));

    let dose_prompt = rpc_roundtrip(
        &mut state,
        8,
        "prompts/get",
        json!({
            "name": "dose_response_curve_fit",
            "arguments": { "sheet_id": "xy-res", "response_type": "antagonist" }
        }),
    )
    .expect("dose prompt");
    let prompt_text = dose_prompt["result"]["messages"][0]["content"]["text"].as_str().expect("prompt text");
    assert!(prompt_text.contains("xy-res"));
    assert!(prompt_text.contains("log_inhibitor_vs_response_4pl"));

    let two_group = rpc_roundtrip(
        &mut state,
        9,
        "prompts/get",
        json!({
            "name": "choose_two_group_test",
            "arguments": { "sheet_id": "col-a", "paired": "false", "parametric": "false" }
        }),
    )
    .expect("two group prompt");
    assert!(two_group["result"]["messages"][0]["content"]["text"].as_str().is_some_and(|t| t.contains("Mann-Whitney")));

    let unknown_prompt = rpc_roundtrip(&mut state, 10, "prompts/get", json!({ "name": "nonexistent_prompt" })).expect("unknown prompt");
    assert_eq!(unknown_prompt["error"]["code"], json!(ERR_INVALID_PARAMS));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn workdir_guard_blocks_parent_traversal_and_outside_paths_on_all_io_surfaces() {
    let dir = TempDir::new("mcp-workdir").expect("temp dir");
    let sandbox_root = dir.path.join("sandbox");
    let outside_dir = dir.path.join("outside");
    fs::create_dir_all(&sandbox_root).expect("sandbox dir");
    fs::create_dir_all(&outside_dir).expect("outside dir");

    let outside_csv = outside_dir.join("secret.csv");
    fs::write(&outside_csv, "X,Y\n1,2\n").expect("outside csv");
    let inside_csv = sandbox_root.join("inside.csv");
    fs::write(&inside_csv, "X,Y\n1,10\n2,20\n").expect("inside csv");

    let mut state = McpServerState::with_workdir(&sandbox_root).expect("with_workdir");

    // 1. Refuse ../ escape and absolute outside path in open_file
    for bad in ["../outside/secret.csv", &outside_csv.display().to_string()] {
        let res = call_tool(&mut state, "open_file", json!({ "path": bad })).expect("open_file refusal");
        assert!(res.is_error, "expected path escape refusal for '{bad}'");
        let sc = res.structured_content.expect("structured error");
        assert_eq!(sc["code"], json!("path_escape"), "unexpected error for '{bad}': {sc}");
        assert_eq!(sc["category"], json!("policy"));
    }

    // 2. Refuse ../ escape and outside path in save_file
    for bad in ["../escape.curvebook", &outside_dir.join("escape.curvebook").display().to_string()] {
        let res = call_tool(&mut state, "save_file", json!({ "path": bad })).expect("save_file refusal");
        assert!(res.is_error, "expected save_file refusal for '{bad}'");
        let sc = res.structured_content.expect("structured error");
        assert_eq!(sc["code"], json!("path_escape"));
    }
    assert!(!outside_dir.join("escape.curvebook").exists());

    // 3. Legitimate inside path succeeds via open_file + export + save_file (atomic)
    let open_ok = call_tool(&mut state, "open_file", json!({ "path": "inside.csv" })).expect("inside open_file");
    assert!(!open_ok.is_error);

    let export_bad = call_tool(&mut state, "export", json!({ "format": "csv", "output_path": "../outside/leak.csv" })).expect("export refusal");
    assert!(export_bad.is_error);
    assert_eq!(export_bad.structured_content.expect("sc")["code"], json!("path_escape"));
    assert!(!outside_dir.join("leak.csv").exists());

    let export_ok = call_tool(&mut state, "export", json!({ "format": "csv", "output_path": "nested/out.csv" })).expect("inside export");
    assert!(!export_ok.is_error);
    assert!(Path::new(&sandbox_root.join("nested/out.csv")).exists());

    let save_ok = call_tool(&mut state, "save_file", json!({ "path": "saved.curvebook" })).expect("inside save");
    assert!(!save_ok.is_error);
    assert!(sandbox_root.join("saved.curvebook").exists());

    // 4. Command path parameters inside command_run must also respect WorkdirGuard.
    let cmd_escape = call_tool(
        &mut state,
        "command_run",
        json!({
            "command_id": "project.save",
            "params": { "path": "../outside/cmd_escape.curvebook" }
        }),
    )
    .expect("command_run path escape");
    assert!(cmd_escape.is_error);
    assert_eq!(cmd_escape.structured_content.expect("sc")["code"], json!("path_escape"));
    assert!(!outside_dir.join("cmd_escape.curvebook").exists());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn mcp_workdir_persists_across_open_file_and_blocks_stored_external_link_recalculate() {
    use curvebook_doc::{
        CellCoord, CellValue, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, LinkSource, LinkedDataRegion, ProjectDocument, SheetCellRange,
        SheetId,
    };
    use curvebook_engine::Session;
    use curvebook_format::save_project_file;

    let dir = TempDir::new("mcp-stored-link").expect("temp dir");
    let sandbox_root = dir.path.join("sandbox");
    let outside_dir = dir.path.join("outside");
    fs::create_dir_all(&sandbox_root).expect("sandbox dir");
    fs::create_dir_all(&outside_dir).expect("outside dir");

    let outside_secret = outside_dir.join("secret.csv");
    fs::write(&outside_secret, "999.0\n").expect("write outside");
    let inside_csv = sandbox_root.join("nested").join("allowed.csv");
    fs::create_dir_all(inside_csv.parent().expect("parent")).expect("mkdir nested");
    fs::write(&inside_csv, "77.5\n").expect("write inside");

    let build_archive = |archive_path: &Path, link_target: &str| {
        let mut builder = Session::from_document(ProjectDocument::new("Linked").expect("doc"));
        builder
            .execute("sheet.createData", json!({ "sheet_id": "col-1", "title": "Target", "kind": "Column", "rows": 2, "cols": 1 }))
            .expect("create sheet");
        builder.execute("table.setCells", json!({ "sheet_id": "col-1", "edits": [{ "row": 0, "col": 0, "value": 10.0 }] })).expect("seed cell");
        let mut doc = builder.document().clone();
        doc.data_sheet_mut(&SheetId::new("col-1")).expect("col-1").linked_regions.push(LinkedDataRegion {
            id: String::from("link-1"),
            spec: ImportLinkSpec {
                source: LinkSource::ExternalDelimitedFile { path: link_target.to_owned(), delimiter: ',', decimal_separator: '.' },
                filter: ImportFilterSpec::default(),
                placement: ImportPlacementSpec::default(),
            },
            target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
            provenance_summary: String::from("external link"),
        });
        save_project_file(archive_path, &doc).expect("save archive");
    };

    let escaping_archive = sandbox_root.join("escape_link.curvebook");
    build_archive(&escaping_archive, &outside_secret.display().to_string());
    let allowed_archive = sandbox_root.join("inside_link.curvebook");
    build_archive(&allowed_archive, "nested/allowed.csv");

    let mut state = McpServerState::with_workdir(&sandbox_root).expect("with_workdir");

    // Opening a valid archive inside workdir must preserve ExternalPathPolicy::Workdir on Session.
    let open_esc = call_tool(&mut state, "open_file", json!({ "path": "escape_link.curvebook" })).expect("open escape_link");
    assert!(!open_esc.is_error, "open_file inside workdir failed: {open_esc:?}");
    assert!(state.session().path_policy().is_constrained(), "path_policy must survive open_file");

    let recalc_bad =
        call_tool(&mut state, "command_run", json!({ "command_id": "project.recalculate", "params": { "include_external_links": true } }))
            .expect("recalc escape");
    assert!(recalc_bad.is_error, "project.recalculate must block stored external link escape");
    assert_eq!(recalc_bad.structured_content.expect("sc")["code"], json!("path_escape"));
    let cell_after_bad =
        state.session().document().data_sheet(&SheetId::new("col-1")).expect("col-1").archetype.data_cell(CellCoord::new(0, 0, 0)).map(|c| &c.value);
    assert_eq!(cell_after_bad, Some(&CellValue::Number(10.0)));

    // Opening an archive whose stored external link stays inside workdir recalculates cleanly.
    let open_ok = call_tool(&mut state, "open_file", json!({ "path": "inside_link.curvebook" })).expect("open inside_link");
    assert!(!open_ok.is_error);
    let recalc_ok =
        call_tool(&mut state, "command_run", json!({ "command_id": "project.recalculate", "params": { "include_external_links": true } }))
            .expect("recalc allowed");
    assert!(!recalc_ok.is_error, "expected inside link recalc to succeed: {recalc_ok:?}");
    let cell_after_ok =
        state.session().document().data_sheet(&SheetId::new("col-1")).expect("col-1").archetype.data_cell(CellCoord::new(0, 0, 0)).map(|c| &c.value);
    assert_eq!(cell_after_ok, Some(&CellValue::Number(77.5)));
}

#[test]
fn r10_10_analysis_toolset_exposes_and_executes_stats_pvalue_stack() {
    let mut state = McpServerState::new();
    call_tool(&mut state, "enable_toolset", json!({ "group": "analysis" })).expect("enable analysis");
    call_tool(&mut state, "enable_toolset", json!({ "group": "sheet" })).expect("enable sheet");
    call_tool(&mut state, "enable_toolset", json!({ "group": "table" })).expect("enable table");

    assert_eq!(command_id_to_mcp_tool_name("stats.pvalueStack"), "stats_pvalue_stack");

    let listed = rpc_roundtrip(&mut state, 100, "tools/list", json!({})).expect("tools/list");
    let tools = listed["result"]["tools"].as_array().expect("tools array");
    let pstack_tool = tools
        .iter()
        .find(|t| t["name"] == command_id_to_mcp_tool_name("stats.pvalueStack"))
        .expect("stats_pvalue_stack registered in analysis toolset");
    assert_eq!(pstack_tool["annotations"]["readOnlyHint"], json!(false));

    let created = call_tool(
        &mut state,
        "sheet_create_data",
        json!({ "sheet_id": "mcp-pvals", "title": "MCP P Stack", "kind": "column", "rows": 4, "cols": 1 }),
    )
    .expect("sheet_create_data");
    assert!(!created.is_error, "sheet_create_data failed: {created:?}");

    let edits = call_tool(
        &mut state,
        "table_set_cells",
        json!({
            "sheet_id": "mcp-pvals",
            "edits": [
                { "row": 0, "col": 0, "value": 0.002 },
                { "row": 1, "col": 0, "value": 0.015 },
                { "row": 2, "col": 0, "value": 0.045 },
                { "row": 3, "col": 0, "value": 0.600 }
            ]
        }),
    )
    .expect("table_set_cells");
    assert!(!edits.is_error, "table_set_cells failed: {edits:?}");

    let res = call_tool(
        &mut state,
        "stats_pvalue_stack",
        json!({
            "sheet_id": "mcp-res-pstack",
            "source_sheet_id": "mcp-pvals",
            "method": "holm",
            "threshold": 0.05
        }),
    )
    .expect("stats_pvalue_stack call");
    assert!(!res.is_error, "stats_pvalue_stack failed: {res:?}");
    let payload = &res.structured_content.expect("structured_content")["payload"];
    assert_eq!(payload["sheet_id"], json!("mcp-res-pstack"));
    assert_eq!(payload["scalars"]["TotalInputCount"]["value"], json!(4.0));
    assert_eq!(payload["scalars"]["TotalDiscoveries"]["value"], json!(2.0));
}

#[test]
fn rv94_two_group_prompt_interpolates_sheet_id_when_unpaired_unknown() {
    let mut state = McpServerState::new();
    let resp = rpc_roundtrip(
        &mut state,
        101,
        "prompts/get",
        json!({
            "name": "choose_two_group_test",
            "arguments": { "sheet_id": "col-cohort-94", "paired": "false", "parametric": "unknown" }
        }),
    )
    .expect("prompts/get");
    let text = resp["result"]["messages"][0]["content"]["text"].as_str().expect("prompt text");
    assert!(text.contains("sheet://col-cohort-94"), "expected interpolated sheet://col-cohort-94, got:\n{text}");
    assert!(!text.contains("{sheet_id}"), "unsubstituted literal {{sheet_id}} leaked into prompt:\n{text}");
}

#[test]
fn rv95_resources_read_caps_csv_preview_rows_and_bytes_and_surfaces_export_errors() {
    // 1. Row-capped preview (> 100 rows): returns header + 100 rows and sets csv_preview_truncated = true.
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 250).unwrap();
    for r in 0..250 {
        xy.x_cells[r][0] = Cell::number(r as f64);
        xy.data_sets[0].cells[r][0] = Cell::number((r * 2) as f64);
    }
    let ds_rows = DataSheet::new("xy-many-rows", "Many Rows", TableArchetype::Xy(xy)).unwrap();
    let mut doc = ProjectDocument::new("RV-95 Preview Caps").unwrap();
    doc.add_data_sheet(ds_rows).unwrap();

    let contents = read_resource(&doc, "sheet://xy-many-rows").expect("read row-capped sheet");
    let val: Value = serde_json::from_str(&contents[0].text).unwrap();
    let preview = val["csv_preview"].as_str().unwrap();
    assert_eq!(val["rows"], json!(250));
    assert_eq!(val["csv_preview_truncated"], json!(true));
    assert_eq!(preview.lines().count(), 101, "expected 1 header + 100 preview rows, got {}", preview.lines().count());
    assert!(!preview.contains("249,498"), "tail rows must not be serialized in preview");

    // 2. Byte-capped preview (wide rows exceeding 64 KiB within <= 100 rows).
    let mut wide_xy = XyTable::new(SubcolumnMode::Single, 200, 80).unwrap();
    for c in 0..200 {
        wide_xy.data_sets[c].title = format!("Col_{c:03}_LongTitleHeader");
        for r in 0..80 {
            wide_xy.data_sets[c].cells[r][0] = Cell::number(1000.123456 + (r as f64) + (c as f64) * 0.01);
        }
    }
    let ds_wide = DataSheet::new("xy-wide", "Wide Sheet", TableArchetype::Xy(wide_xy)).unwrap();
    doc.add_data_sheet(ds_wide).unwrap();

    let wide_contents = read_resource(&doc, "sheet://xy-wide").expect("read byte-capped sheet");
    let wide_val: Value = serde_json::from_str(&wide_contents[0].text).unwrap();
    let wide_preview = wide_val["csv_preview"].as_str().unwrap();
    assert_eq!(wide_val["csv_preview_truncated"], json!(true));
    assert!(wide_preview.len() <= 65_536, "byte cap exceeded: {} bytes", wide_preview.len());

    // 3. Invalid sheet archetype must return ResourceError::Internal rather than silent "".
    let mut bad_xy = XyTable::new(SubcolumnMode::Single, 1, 3).unwrap();
    bad_xy.x_cells.pop(); // break rectangular invariant
    let mut bad_doc = ProjectDocument::new("Invalid Preview").unwrap();
    let mut ok_ds = DataSheet::new("xy-bad", "Bad Sheet", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 3).unwrap())).unwrap();
    ok_ds.archetype = TableArchetype::Xy(bad_xy);
    bad_doc.data_sheets.push(ok_ds);

    let err = read_resource(&bad_doc, "sheet://xy-bad").expect_err("corrupt sheet export must surface error instead of empty csv_preview");
    assert!(matches!(err, ResourceError::Internal(msg) if msg.contains("CSV preview export failed")));
}

#[test]
fn ui_state_and_filesystem_write_tools_report_worst_case_annotations() {
    let mut state = McpServerState::new();
    let res = call_tool(&mut state, "enable_toolset", json!({ "group": "all" })).expect("enable all toolsets");
    assert!(!res.is_error, "enable_toolset all failed: {res:?}");

    let tools = list_tools_for_state(&state);
    let by_name: std::collections::BTreeMap<&str, ToolAnnotations> = tools.iter().map(|t| (t.name.as_str(), t.annotations)).collect();

    // Audit #24: CHANGES_UI_STATE tools are not read-only.
    for tool_name in ["enable_toolset", "sheet_select"] {
        let ann = by_name.get(tool_name).copied().unwrap_or_else(|| panic!("missing tool {tool_name}"));
        assert!(!ann.read_only_hint, "{tool_name} modifies UI/session state and must report readOnlyHint=false");
    }

    // Audit #25: tools that write/overwrite files on disk must report destructiveHint=true and readOnlyHint=false.
    for tool_name in [
        "export",
        "save_file",
        "project_save",
        "project_save_template",
        "table_export_csv",
        "export_svg",
        "export_pdf",
        "export_png",
        "export_tiff",
        "analysis_export_csv",
        "analysis_export_json",
    ] {
        let ann = by_name.get(tool_name).copied().unwrap_or_else(|| panic!("missing tool {tool_name}"));
        assert!(ann.destructive_hint, "{tool_name} can overwrite destination files and must report destructiveHint=true");
        assert!(!ann.read_only_hint, "{tool_name} writes filesystem and must report readOnlyHint=false");
        assert!(ann.open_world_hint, "{tool_name} interacts with external filesystem paths");
    }

    // Read-only introspection tools stay readOnlyHint=true, destructiveHint=false.
    for tool_name in ["doc_inspect", "command_list", "render_preview", "list_toolsets"] {
        let ann = by_name.get(tool_name).copied().unwrap_or_else(|| panic!("missing tool {tool_name}"));
        assert!(ann.read_only_hint, "{tool_name} must remain readOnlyHint=true");
        assert!(!ann.destructive_hint, "{tool_name} must remain destructiveHint=false");
    }

    // Exhaustive invariant across every engine CommandSpec mapped into MCP tools.
    for spec in registry().specs() {
        let name = command_id_to_mcp_tool_name(spec.id);
        let ann = by_name.get(name.as_str()).copied().unwrap_or_else(|| panic!("missing mapped tool {name}"));
        if spec.effects.contains(Effects::WRITES_FILESYSTEM) {
            assert!(ann.destructive_hint, "command `{}` ({name}) has WRITES_FILESYSTEM -> destructiveHint=true", spec.id);
            assert!(!ann.read_only_hint, "command `{}` ({name}) has WRITES_FILESYSTEM -> readOnlyHint=false", spec.id);
        }
        if spec.effects.contains(Effects::CHANGES_UI_STATE) {
            assert!(!ann.read_only_hint, "command `{}` ({name}) has CHANGES_UI_STATE -> readOnlyHint=false", spec.id);
        }
    }
}

#[test]
fn path_policy_extra_keys_gui_tab_screenshot_and_batch_cap_overflow() {
    // 1. RejectExternal & Workdir extra path keys (`dir`, `directory`, `destination_path`, `unpacked_dir`, `output_path`).
    let td = TempDir::new("policy_keys").unwrap();
    let csv = td.child("in.csv");
    fs::write(&csv, "X,Y\n1,2\n").unwrap();
    let (out_cb, out_csv) = (td.child("out.curvebook"), td.child("out.csv"));
    let mut state = McpServerState::new();
    state.session_mut().set_path_policy(ExternalPathPolicy::RejectExternal);
    for (tool, args) in [
        ("open_file", json!({ "path": csv.display().to_string() })),
        ("save_file", json!({ "path": out_cb.display().to_string() })),
        ("export", json!({ "format": "csv", "output_path": out_csv.display().to_string() })),
        ("command_run", json!({ "command_id": "project.save", "params": { "path": out_cb.display().to_string() } })),
        ("command_run", json!({ "command_id": "project.save", "params": { "destination_path": out_cb.display().to_string() } })),
        ("command_batch", json!({ "commands": [{ "command_id": "project.save", "params": { "output_path": out_cb.display().to_string() } }] })),
    ] {
        let res = call_tool(&mut state, tool, args).unwrap();
        assert!(res.is_error && res.structured_content.unwrap()["code"] == json!("path_escape"), "{tool}");
    }
    assert!(!out_cb.exists() && !out_csv.exists());

    let wd = td.child("wd");
    fs::create_dir_all(&wd).unwrap();
    let mut sandboxed = McpServerState::with_workdir(&wd).unwrap();
    for key in ["dir", "directory", "destination_path", "unpacked_dir", "output_path"] {
        let mut obj = serde_json::Map::new();
        obj.insert(key.to_owned(), json!("../escape.out"));
        let res = call_tool(&mut sandboxed, "command_run", json!({ "command_id": "project.save", "params": Value::Object(obj) })).unwrap();
        assert!(res.is_error && res.structured_content.unwrap()["code"] == json!("path_escape"), "{key}");
    }

    // 2. `ui_switch_tab` normalizes integer index -> string; path-only `ui_screenshot` succeeds cleanly.
    struct RecHost(Session, Rc<RefCell<Vec<(String, Value)>>>, Rc<RefCell<Option<Value>>>);
    impl GuiSessionHost for RecHost {
        fn call_gui(&mut self, method: &str, params: Value) -> Result<Value, EngineError> {
            self.1.borrow_mut().push((method.to_owned(), params));
            Ok(self.2.borrow_mut().take().unwrap_or_else(|| json!({ "ok": true })))
        }
        fn hosted_session(&self) -> Option<&Session> {
            Some(&self.0)
        }
        fn hosted_session_mut(&mut self) -> Option<&mut Session> {
            Some(&mut self.0)
        }
        fn sync_after_session_mutation(&mut self) {}
    }
    let (calls, next_reply) = (Rc::new(RefCell::new(Vec::new())), Rc::new(RefCell::new(None)));
    let mut gui = McpServerState::with_gui_host(Box::new(RecHost(Session::new(), Rc::clone(&calls), Rc::clone(&next_reply))));
    assert!(!call_tool(&mut gui, "ui_switch_tab", json!({ "tab": 1 })).unwrap().is_error);
    *next_reply.borrow_mut() =
        Some(json!({ "path": "/tmp/shot.png", "width_px": 2400, "height_px": 1800, "dpi": 300, "byte_len": 13_000_000, "inline_omitted": true }));
    let shot = call_tool(&mut gui, "ui_screenshot", json!({ "target": "full_window" })).unwrap();
    assert!(!shot.is_error && shot.structured_content.as_ref().and_then(|v| v.get("inline_omitted")) == Some(&json!(true)));
    assert_eq!(calls.borrow()[0].1["tab"], json!("1"));

    // 3. `command_batch` step cap = 1000; live response overflow after `project.save` flags persisted fs side effects,
    //    while pre-execution rejection reports `rolled_back: true` and `filesystem_side_effects_persisted: false`.
    let mut headless = McpServerState::new();
    let too_many: Vec<Value> = (0..1_001).map(|_| json!({ "command_id": "project.inspect", "params": {} })).collect();
    assert!(
        matches!(call_tool(&mut headless, "command_batch", json!({ "commands": too_many })), Err(ToolCallError::InvalidParams(ref m)) if m.contains("1000"))
    );

    let mut live = McpServerState::with_control_port(49813).unwrap().with_control_handler(|method, params| {
        if method == "command.batch" {
            if params["commands"].as_array().map_or(0, Vec::len) == 2 {
                return Err(String::from("control response exceeds maximum control line size (16777216 bytes)"));
            }
            return Err(String::from("command.batch exceeds maximum step limit (1000)"));
        }
        Ok(json!({}))
    });
    let ovf = call_tool(&mut live, "command_batch", json!({ "atomic": true, "commands": [{ "command_id": "project.save", "params": { "path": "saved.curvebook" } }, { "command_id": "project.inspect", "params": {} }] })).unwrap();
    let sc = ovf.structured_content.unwrap();
    assert!(ovf.is_error && sc["filesystem_side_effects_persisted"] == json!(true) && sc["failed_index"] == json!(1));
    let pre = call_tool(
        &mut live,
        "command_batch",
        json!({ "atomic": true, "commands": [{ "command_id": "project.save", "params": { "path": "saved.curvebook" } }] }),
    )
    .unwrap();
    let psc = pre.structured_content.unwrap();
    assert!(pre.is_error && psc["rolled_back"] == json!(true) && psc["filesystem_side_effects_persisted"] == json!(false));
}
