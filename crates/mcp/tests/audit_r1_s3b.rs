//! Regression tests for `R1-S12-03` in `curvebook-mcp`:
//! MCP `save_file {}` without `path` refuses with `InvalidParams` and never overwrites a previously
//! opened `.curvebook` archive after `project.new` or any other wholesale non-path replacement.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::ProjectDocument;
use curvebook_engine::Session;
use curvebook_mcp::{McpServerState, ToolCallError, call_tool};
use serde_json::{Value, json};

type TestResult = Result<(), String>;

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("cb-mcp-s3b-{label}-{}-{seq}", std::process::id()));
        fs::create_dir_all(&dir).map_err(|err| format!("create temp dir `{}`: {err}", dir.display()))?;
        Ok(Self { path: dir })
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn call_json_str(state: &mut McpServerState, tool: &str, args: Value) -> Result<Value, String> {
    let res = call_tool(state, tool, args).map_err(|err| format!("tool `{tool}` failed: {err}"))?;
    let text = res
        .content
        .first()
        .and_then(|block| serde_json::to_value(block).ok())
        .and_then(|val| val.get("text").and_then(Value::as_str).map(ToOwned::to_owned))
        .ok_or_else(|| format!("missing text response block for `{tool}`"))?;
    serde_json::from_str(&text).map_err(|err| format!("parse `{tool}` response JSON: {err}"))
}

#[test]
fn r1_s12_03_mcp_save_without_path_refuses_after_project_new_and_preserves_old_file() -> TestResult {
    let dir = TempDir::new("save-guard")?;
    let path_a = dir.path.join("A.curvebook");
    let path_b = dir.path.join("B.curvebook");
    let path_tpl = dir.path.join("tpl.cbtemplate");

    // Seed A.curvebook, B.curvebook, and tpl.cbtemplate on disk.
    let mut seed = Session::from_document(ProjectDocument::new("Original A").map_err(|e| e.to_string())?);
    seed.execute("sheet.createData", json!({ "sheet_id": "d1", "title": "Sentinel Data", "rows": 2, "cols": 1 })).map_err(|e| e.to_string())?;
    seed.execute("table.setCells", json!({ "sheet_id": "d1", "edits": [{ "row": 0, "col": 0, "value": 123.456 }] })).map_err(|e| e.to_string())?;
    let saved_a = seed.execute("project.save", json!({ "path": path_a.to_string_lossy(), "return_bytes": true })).map_err(|e| e.to_string())?;
    let bytes_b64 = saved_a.payload.get("bytes_base64").and_then(Value::as_str).ok_or_else(|| String::from("missing bytes_base64"))?.to_owned();
    seed.execute("project.saveTemplate", json!({ "path": path_tpl.to_string_lossy() })).map_err(|e| e.to_string())?;

    let mut seed_b = Session::from_document(ProjectDocument::new("Original B").map_err(|e| e.to_string())?);
    seed_b.execute("project.save", json!({ "path": path_b.to_string_lossy() })).map_err(|e| e.to_string())?;

    let original_a_bytes = fs::read(&path_a).map_err(|e| format!("read A.curvebook: {e}"))?;

    let mut state = McpServerState::new();

    // 1. Open A.curvebook -> project.new -> save_file {} must refuse and preserve A.curvebook.
    call_json_str(&mut state, "open_file", json!({ "path": path_a.to_string_lossy() }))?;
    let inspected_before = call_json_str(&mut state, "doc_inspect", json!({}))?;
    if inspected_before.get("file_path").and_then(Value::as_str).is_none() {
        return Err(format!("expected file_path set after open_file, got {inspected_before}"));
    }

    call_json_str(&mut state, "command_run", json!({ "command_id": "project.new", "params": { "title": "Brand New" } }))?;
    let inspected_after = call_json_str(&mut state, "doc_inspect", json!({}))?;
    if inspected_after.get("file_path") != Some(&Value::Null) {
        return Err(format!("expected file_path null after project.new, got {inspected_after}"));
    }

    match call_tool(&mut state, "save_file", json!({})) {
        Err(ToolCallError::InvalidParams(_)) => {}
        other => return Err(format!("expected InvalidParams from save_file after project.new, got {other:?}")),
    }
    let after_new_bytes = fs::read(&path_a).map_err(|e| format!("re-read A.curvebook: {e}"))?;
    if after_new_bytes != original_a_bytes {
        return Err(String::from("A.curvebook was modified after project.new + save_file {}"));
    }

    // 2. Open A.curvebook -> command_batch ending in project.new -> save_file {} refuses.
    call_json_str(&mut state, "open_file", json!({ "path": path_a.to_string_lossy() }))?;
    call_json_str(
        &mut state,
        "command_batch",
        json!({
            "commands": [
                { "command_id": "sheet.createData", "params": { "sheet_id": "extra" } },
                { "command_id": "project.new", "params": { "title": "Cleared In Batch" } }
            ]
        }),
    )?;
    match call_tool(&mut state, "save_file", json!({})) {
        Err(ToolCallError::InvalidParams(_)) => {}
        other => return Err(format!("expected InvalidParams from save_file after batch project.new, got {other:?}")),
    }
    if fs::read(&path_a).map_err(|e| e.to_string())? != original_a_bytes {
        return Err(String::from("A.curvebook was modified after command_batch project.new"));
    }

    // 3. Open A.curvebook -> project.instantiateTemplate / project.open(bytes_base64) -> save_file {} refuses.
    for (cmd_id, params) in [
        ("project.instantiateTemplate", json!({ "path": path_tpl.to_string_lossy(), "title": "Tpl" })),
        ("project.open", json!({ "bytes_base64": bytes_b64 })),
    ] {
        call_json_str(&mut state, "open_file", json!({ "path": path_a.to_string_lossy() }))?;
        call_json_str(&mut state, "command_run", json!({ "command_id": cmd_id, "params": params }))?;
        match call_tool(&mut state, "save_file", json!({})) {
            Err(ToolCallError::InvalidParams(_)) => {}
            other => return Err(format!("expected InvalidParams from save_file after {cmd_id}, got {other:?}")),
        }
        if fs::read(&path_a).map_err(|e| e.to_string())? != original_a_bytes {
            return Err(format!("A.curvebook modified after {cmd_id}"));
        }
    }

    // 4. Open A.curvebook -> command_run project.open { path: B } -> save_file {} writes B, never A.
    call_json_str(&mut state, "open_file", json!({ "path": path_a.to_string_lossy() }))?;
    call_json_str(&mut state, "command_run", json!({ "command_id": "project.open", "params": { "path": path_b.to_string_lossy() } }))?;
    let inspected_b = call_json_str(&mut state, "doc_inspect", json!({}))?;
    let expected_b = path_b.to_string_lossy().to_string();
    if inspected_b.get("file_path").and_then(Value::as_str) != Some(expected_b.as_str()) {
        return Err(format!("expected file_path `{expected_b}`, got {inspected_b}"));
    }
    call_json_str(&mut state, "save_file", json!({}))?;
    if fs::read(&path_a).map_err(|e| e.to_string())? != original_a_bytes {
        return Err(String::from("A.curvebook modified after switching to B.curvebook via project.open"));
    }

    Ok(())
}
