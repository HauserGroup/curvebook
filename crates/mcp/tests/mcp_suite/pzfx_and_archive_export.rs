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
    AnalysisKind, AnalysisSpec, Cell, ConstantValue, DataSheet, GraphSheet, InfoSheet, ProjectDocument, ResultProvenance, ResultSheet, ResultStatus,
    SheetId, SubcolumnMode, TableArchetype, XyTable,
};
#[cfg(not(target_arch = "wasm32"))]
use curvebook_engine::Caller;
use curvebook_format::registry::DESC_PRISM;
use curvebook_format::{ZipWriter, compare_pzfx_data_layer, import_pzfx_bytes, save_project_file};
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

fn build_promoted_archive_fixture_bytes() -> Vec<u8> {
    let mut zip = ZipWriter::new();
    let add_json = |zw: &mut ZipWriter, path: &str, value: Value| {
        zw.add_file(path, serde_json::to_vec(&value).unwrap()).unwrap();
    };
    add_json(
        &mut zip,
        "document.json",
        json!({
            "formatVersion": "1-6-0",
            "title": "MCP Promoted Archive",
            "sheets": { "data": ["d-groups", "d-single"], "analyses": ["an-ttest", "an-stale", "an-fallback"] },
            "sheetAttributesMap": {
                "d-groups": { "title": "Groups" },
                "d-single": { "title": "Single" },
                "an-ttest": { "title": "Agreeing T-Test" },
                "an-stale": { "title": "Stale T-Test" },
                "an-fallback": { "title": "Degenerate T-Test" }
            }
        }),
    );
    add_json(
        &mut zip,
        "data/sheets/d-groups/sheet.json",
        json!({"uid": "d-groups", "title": "Groups", "table": {"uid": "t-groups", "format": "column", "dataFormat": "y_single", "dataSets": ["ds-a", "ds-b"]}}),
    );
    add_json(&mut zip, "data/sets/ds-a.json", json!({"title": "Control", "replicates": [{"valueType": "real"}]}));
    add_json(&mut zip, "data/sets/ds-b.json", json!({"title": "Treated", "replicates": [{"valueType": "real"}]}));
    zip.add_file("data/tables/t-groups/data.csv", b"10,20\n12,22\n14,24\n".to_vec()).unwrap();

    add_json(
        &mut zip,
        "data/sheets/d-single/sheet.json",
        json!({"uid": "d-single", "title": "Single", "table": {"uid": "t-single", "format": "column", "dataFormat": "y_single", "dataSets": ["ds-only"]}}),
    );
    add_json(&mut zip, "data/sets/ds-only.json", json!({"title": "Only", "replicates": [{"valueType": "real"}]}));
    zip.add_file("data/tables/t-single/data.csv", b"1\n2\n3\n".to_vec()).unwrap();

    let ttest_params = json!({"content": {"experimentalDesign": "UNPAIRED_T_TEST", "options": {"pValueTailType": "P_VALUE_TWO_TAILED"}}});
    add_json(
        &mut zip,
        "analyses/an-ttest/sheet.json",
        json!({"uid": "an-ttest", "title": "Agreeing T-Test", "analysisClass": "T_TESTS_AND_NONPARAMETRIC", "inputSheets": [{"uid": "d-groups"}]}),
    );
    add_json(&mut zip, "analyses/an-ttest/parameters.json", ttest_params.clone());
    add_json(
        &mut zip,
        "analyses/an-ttest/results.json",
        json!({"content": {"unpairedTTestModel": {"unpairedTTest": {"t": -6.123_724_356_957_945, "pValue": {"state": "P_VALUE_EQUAL", "value": 0.003_602_232_609_1}}}}}),
    );

    add_json(
        &mut zip,
        "analyses/an-stale/sheet.json",
        json!({"uid": "an-stale", "title": "Stale T-Test", "analysisClass": "T_TESTS_AND_NONPARAMETRIC", "inputSheets": [{"uid": "d-groups"}]}),
    );
    add_json(&mut zip, "analyses/an-stale/parameters.json", ttest_params.clone());
    add_json(
        &mut zip,
        "analyses/an-stale/results.json",
        json!({"content": {"unpairedTTestModel": {"unpairedTTest": {"t": -6.123_724_356_957_945, "pValue": {"state": "P_VALUE_EQUAL", "value": 0.156}}}}}),
    );

    add_json(
        &mut zip,
        "analyses/an-fallback/sheet.json",
        json!({
            "uid": "an-fallback",
            "title": "Degenerate T-Test",
            "analysisClass": "T_TESTS_AND_NONPARAMETRIC",
            "inputSheets": [{"uid": "d-single"}],
            "preferredResultSheet": "rs-fb",
            "resultSheets": [{"uid": "rs-fb", "title": "Cached Fallback"}]
        }),
    );
    add_json(&mut zip, "analyses/an-fallback/parameters.json", ttest_params);
    add_json(&mut zip, "analyses/an-fallback/result_sheets/rs-fb.json", json!({"uid": "rs-fb", "name": "Cached Fallback", "dataSheet": "view-fb"}));
    add_json(
        &mut zip,
        "data/sheets/view-fb/sheet.json",
        json!({"uid": "view-fb", "flags": ["ANALYSIS_VIEW"], "table": {"uid": "t-view-fb", "format": "view", "dataFormat": "text", "rowTitlesDataSet": "ds-fb-rt", "dataSets": ["ds-fb-v"]}}),
    );
    add_json(&mut zip, "data/sets/ds-fb-rt.json", json!({"title": "Metric", "replicates": [{"valueType": "string"}]}));
    add_json(&mut zip, "data/sets/ds-fb-v.json", json!({"title": "Value", "replicates": [{"valueType": "string"}]}));
    zip.add_file("data/tables/t-view-fb/data.csv", b"P value,0.042\n".to_vec()).unwrap();

    zip.finish().unwrap()
}

#[test]
fn mcp_open_file_and_import_archive_recompute_promoted_analyses_and_surface_warnings() {
    let dir = TempDir::new_canonical("mcp-promoted-archive").unwrap();
    let archive_path = dir.child("promoted.prism");
    fs::write(&archive_path, build_promoted_archive_fixture_bytes()).unwrap();

    // 1. Standalone `open_file`: finalizes promoted analyses and returns typed warnings.
    let mut state = McpServerState::with_workdir(dir.path()).unwrap();
    let opened = call_tool(&mut state, "open_file", json!({ "path": "promoted.prism" })).unwrap();
    assert!(!opened.is_error, "open_file failed: {:?}", opened.structured_content);
    let opened_json = parse_result_json(&opened).unwrap();
    let open_warns = opened_json["warnings"].as_array().unwrap();
    assert!(
        open_warns.iter().any(|w| w["kind"] == json!("RecomputedResultDisagreement") && w["message"].as_str().unwrap_or("").contains("0.156")),
        "missing disagreement warning on standalone open_file: {open_warns:?}"
    );
    assert!(
        open_warns
            .iter()
            .any(|w| w["kind"] == json!("AnalysisRecomputeFallback") && w["message"].as_str().unwrap_or("").contains("Degenerate T-Test")),
        "missing fallback warning on standalone open_file: {open_warns:?}"
    );
    assert!(!open_warns.iter().any(|w| w["source"] == json!("analyses/an-ttest")));

    let doc = state.session().document();
    assert_eq!(doc.result_sheet(&SheetId::new("an-ttest")).unwrap().status, ResultStatus::UpToDate);
    let stale = doc.result_sheet(&SheetId::new("an-stale")).unwrap();
    assert_eq!(stale.status, ResultStatus::UpToDate);
    assert_eq!(stale.provenance, ResultProvenance::Computed);
    assert!((stale.lookup_scalar("PValue", None).unwrap() - 0.003_602_232_609_1).abs() < 1e-9);
    let fb = doc.result_sheet(&SheetId::new("an-fallback")).unwrap();
    assert_eq!(fb.status, ResultStatus::Imported);
    assert!(matches!(fb.analysis.kind, AnalysisKind::ImportedSnapshot(_)));
    assert_eq!(fb.tabs.len(), 1);

    // 2. Direct `command_run` -> `table.importPrism`: same post-recompute warnings & live state.
    let mut cmd_state = McpServerState::with_workdir(dir.path()).unwrap();
    let imported =
        call_tool(&mut cmd_state, "command_run", json!({ "command_id": "table.importPrism", "params": { "path": "promoted.prism" } })).unwrap();
    assert!(!imported.is_error, "table.importPrism failed: {:?}", imported.structured_content);
    let imp_json = parse_result_json(&imported).unwrap();
    let imp_warns = imp_json.pointer("/payload/warnings").and_then(Value::as_array).unwrap();
    assert!(imp_warns.iter().any(|w| w["kind"] == json!("RecomputedResultDisagreement")));
    assert!(imp_warns.iter().any(|w| w["kind"] == json!("AnalysisRecomputeFallback")));
    assert_eq!(imp_json.pointer("/payload/failed_count"), Some(&json!(0)));
    assert_eq!(cmd_state.session().document().result_sheet(&SheetId::new("an-ttest")).unwrap().status, ResultStatus::UpToDate);
    assert_eq!(cmd_state.session().document().result_sheet(&SheetId::new("an-fallback")).unwrap().status, ResultStatus::Imported);

    // 3. Live-attach `open_file` (replace via `command.batch` and append via `engine.execute`)
    //    must surface post-recompute warnings returned by `table.importPrism`.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let desktop = Arc::new(Mutex::named("test.live_promoted.session", curvebook_engine::Session::new()));
        let desktop_handler = Arc::clone(&desktop);
        let mut live = McpServerState::with_control_port(49342).unwrap().with_control_handler(move |method, params| match method {
            "engine.execute" => {
                let cmd_id = params.get("command_id").and_then(Value::as_str).ok_or_else(|| String::from("missing command_id"))?;
                let cmd_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
                let cancel = std::sync::atomic::AtomicBool::new(false);
                let mut guard = desktop_handler.lock_or_recover().map_err(|e| format!("lock: {e}"))?;
                let outcome = guard.execute_with_caller(cmd_id, cmd_params, Caller::Control, &cancel).map_err(|e| e.message().to_owned())?;
                serde_json::to_value(outcome).map_err(|e| e.to_string())
            }
            "command.batch" => {
                let cmds = params.get("commands").and_then(Value::as_array).ok_or_else(|| String::from("missing commands"))?;
                let cancel = std::sync::atomic::AtomicBool::new(false);
                let mut guard = desktop_handler.lock_or_recover().map_err(|e| format!("lock: {e}"))?;
                let mut results = Vec::new();
                for (idx, item) in cmds.iter().enumerate() {
                    let cmd_id = item.get("command").and_then(Value::as_str).ok_or_else(|| String::from("missing command"))?;
                    let cmd_params = item.get("params").cloned().unwrap_or_else(|| json!({}));
                    let outcome = guard.execute_with_caller(cmd_id, cmd_params, Caller::Control, &cancel).map_err(|e| e.message().to_owned())?;
                    results.push(json!({ "index": idx, "command": cmd_id, "result": serde_json::to_value(outcome).map_err(|e| e.to_string())? }));
                }
                Ok(json!({ "atomic": true, "count": results.len(), "results": results }))
            }
            "doc.inspect" => {
                let guard = desktop_handler.lock_or_recover().map_err(|e| format!("lock: {e}"))?;
                Ok(json!({ "title": guard.document().title, "counts": { "total": guard.document().total_sheet_count() } }))
            }
            other => Err(format!("unexpected live method {other}")),
        });

        let live_replace = call_tool(&mut live, "open_file", json!({ "path": archive_path.display().to_string() })).unwrap();
        assert!(!live_replace.is_error, "live open_file replace failed: {:?}", live_replace.structured_content);
        let lr_json = parse_result_json(&live_replace).unwrap();
        let lr_warns = lr_json["warnings"].as_array().unwrap();
        assert!(lr_warns.iter().any(|w| w["kind"] == json!("RecomputedResultDisagreement")), "live replace warnings: {lr_warns:?}");
        assert!(lr_warns.iter().any(|w| w["kind"] == json!("AnalysisRecomputeFallback")), "live replace warnings: {lr_warns:?}");

        let live_append = call_tool(&mut live, "open_file", json!({ "path": archive_path.display().to_string(), "append": true })).unwrap();
        assert!(!live_append.is_error, "live open_file append failed: {:?}", live_append.structured_content);
        let la_json = parse_result_json(&live_append).unwrap();
        let la_warns = la_json["warnings"].as_array().unwrap();
        assert!(la_warns.iter().any(|w| w["kind"] == json!("RecomputedResultDisagreement")), "live append warnings: {la_warns:?}");
        assert!(la_warns.iter().any(|w| w["kind"] == json!("AnalysisRecomputeFallback")), "live append warnings: {la_warns:?}");
    }
}
