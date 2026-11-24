//! Integration tests for `curvebook-cli` Stage 3B S1 fixes:
//! - `R1-S12-02`: `load_project_json` validates JSON projects against `validate_project_document` and cell budgets.
//! - `R1-S12-03`: MCP subprocess `save_file {}` without `path` after `project.new` preserves the previously opened file.

use std::fs;
use std::io::BufReader;
use std::process::{Child, Command, Output, Stdio};

use curvebook_doc::{
    Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, SubcolumnMode, TableArchetype, XColumnMode, XyTable,
};
use curvebook_format::save_project_file;
use serde_json::{Value, json};

use super::common::{CliHarness, TempDir, read_framed_rpc, write_framed_rpc};

type TestResult = Result<(), String>;

fn run_convert(args: &[&str]) -> Result<Output, String> {
    CliHarness::run(args)
}

fn assert_clean_failure(output: &Output, context: &str) -> TestResult {
    let code = output.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !(1..=3).contains(&code) {
        return Err(format!("{context}: expected exit code 1..=3, got {code}; stdout={stdout}; stderr={stderr}"));
    }
    if stderr.contains("panicked at") || stdout.contains("panicked at") {
        return Err(format!("{context}: unexpected panic output: stderr={stderr}"));
    }
    Ok(())
}

fn send_mcp(stdin: &mut impl std::io::Write, msg: &Value) -> TestResult {
    write_framed_rpc(stdin, msg)
}

fn recv_mcp(reader: &mut BufReader<impl std::io::Read>) -> Result<Value, String> {
    read_framed_rpc(reader)
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn r1_s12_02_convert_json_validates_replicates_schema_and_accepts_valid_json() -> TestResult {
    let dir = TempDir::new("json-validate")?;
    let mut base_doc = ProjectDocument::new("JSON Baseline").map_err(|e| e.to_string())?;
    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2).map_err(|e| e.to_string())?;
    let mut ds = DataSheet::new("d1", "Control vs Treated", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
    ds.set_cell(CellCoord::new(0, 0, 0), Cell::number(1.5)).map_err(|e| e.to_string())?;
    ds.set_cell(CellCoord::new(0, 1, 0), Cell::number(2.5)).map_err(|e| e.to_string())?;
    base_doc.add_data_sheet(ds).map_err(|e| e.to_string())?;

    // 1. Huge `RawReplicates` in JSON must return clean CLI exit 1..=3 with zero panic.
    let mut huge_doc = serde_json::to_value(&base_doc).map_err(|e| e.to_string())?;
    huge_doc["data_sheets"][0]["archetype"]["Column"]["subcolumn_mode"] = json!({ "RawReplicates": 400_000_000_000_000_000_u64 });
    let evil_json = dir.path().join("evil_replicates.json");
    let evil_out = dir.path().join("evil_out.csv");
    fs::write(&evil_json, serde_json::to_vec(&huge_doc).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let huge_res = run_convert(&["convert", &evil_json.to_string_lossy(), &evil_out.to_string_lossy()])?;
    assert_clean_failure(&huge_res, "huge RawReplicates JSON")?;
    if evil_out.exists() {
        return Err(String::from("output CSV must not exist when evil_replicates.json is rejected"));
    }

    // 1b. Huge `RawReplicates` in `XyTable` `y_subcolumn_mode` and `x_mode` must fail cleanly without panic.
    let mut xy_doc = ProjectDocument::new("XY Replicates").map_err(|e| e.to_string())?;
    let xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, 1)
        .map_err(|e| e.to_string())?;
    xy_doc.add_data_sheet(DataSheet::new("xy1", "XY", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut xy_huge_y = serde_json::to_value(&xy_doc).map_err(|e| e.to_string())?;
    xy_huge_y["data_sheets"][0]["archetype"]["Xy"]["y_subcolumn_mode"] = json!({ "RawReplicates": 400_000_000_000_000_000_u64 });
    let xy_y_json = dir.path().join("evil_xy_y.json");
    fs::write(&xy_y_json, serde_json::to_vec(&xy_huge_y).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    assert_clean_failure(&run_convert(&["convert", &xy_y_json.to_string_lossy(), &evil_out.to_string_lossy()])?, "huge XY y RawReplicates JSON")?;

    let mut xy_huge_x = serde_json::to_value(&xy_doc).map_err(|e| e.to_string())?;
    xy_huge_x["data_sheets"][0]["archetype"]["Xy"]["x_mode"] =
        json!({ "Numbers": { "subcolumn_mode": { "RawReplicates": 400_000_000_000_000_000_u64 } } });
    let xy_x_json = dir.path().join("evil_xy_x.json");
    fs::write(&xy_x_json, serde_json::to_vec(&xy_huge_x).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    assert_clean_failure(&run_convert(&["convert", &xy_x_json.to_string_lossy(), &evil_out.to_string_lossy()])?, "huge XY x RawReplicates JSON")?;

    // 2. Zero `RawReplicates` must fail cleanly without panic.
    let mut zero_doc = serde_json::to_value(&base_doc).map_err(|e| e.to_string())?;
    zero_doc["data_sheets"][0]["archetype"]["Column"]["subcolumn_mode"] = json!({ "RawReplicates": 0 });
    let zero_json = dir.path().join("zero_replicates.json");
    fs::write(&zero_json, serde_json::to_vec(&zero_doc).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let zero_res = run_convert(&["convert", &zero_json.to_string_lossy(), &evil_out.to_string_lossy()])?;
    assert_clean_failure(&zero_res, "zero RawReplicates JSON")?;

    // 3. Future `schema_version` must fail cleanly without panic.
    let mut future_doc = serde_json::to_value(&base_doc).map_err(|e| e.to_string())?;
    future_doc["schema_version"] = json!(999);
    let future_json = dir.path().join("future_schema.json");
    fs::write(&future_json, serde_json::to_vec(&future_doc).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let future_res = run_convert(&["convert", &future_json.to_string_lossy(), &evil_out.to_string_lossy()])?;
    assert_clean_failure(&future_res, "future schema_version JSON")?;

    // 4. Top-level array JSON must fail cleanly without panic.
    let array_json = dir.path().join("array.json");
    fs::write(&array_json, b"[]").map_err(|e| e.to_string())?;
    let array_res = run_convert(&["convert", &array_json.to_string_lossy(), &evil_out.to_string_lossy()])?;
    assert_clean_failure(&array_res, "top-level array JSON")?;

    // 5. Valid `.json` project still converts cleanly to `.csv`.
    let valid_json = dir.path().join("valid.json");
    let valid_out = dir.path().join("valid.csv");
    fs::write(&valid_json, serde_json::to_vec_pretty(&base_doc).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let ok_res = run_convert(&["convert", &valid_json.to_string_lossy(), &valid_out.to_string_lossy()])?;
    if !ok_res.status.success() {
        return Err(format!("valid.json convert failed (code {:?}): stderr={}", ok_res.status.code(), String::from_utf8_lossy(&ok_res.stderr)));
    }
    let csv_body = fs::read_to_string(&valid_out).map_err(|e| format!("read valid.csv: {e}"))?;
    if !csv_body.contains("1.5") || !csv_body.contains("2.5") {
        return Err(format!("expected converted CSV to include data values, got:\n{csv_body}"));
    }

    Ok(())
}

#[test]
fn r1_s12_03_mcp_cli_subprocess_save_without_path_after_project_new_preserves_file() -> TestResult {
    let dir = TempDir::new("mcp-save-guard")?;
    let path_a = dir.path().join("A.curvebook");

    let mut doc = ProjectDocument::new("Sentinel Archive").map_err(|e| e.to_string())?;
    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).map_err(|e| e.to_string())?;
    let mut ds = DataSheet::new("d1", "A Data", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
    ds.set_cell(CellCoord::new(0, 0, 0), Cell::number(77.0)).map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    save_project_file(&path_a, &doc).map_err(|e| e.to_string())?;
    let original_bytes = fs::read(&path_a).map_err(|e| e.to_string())?;

    let mut child = ChildGuard(
        Command::new(CliHarness::bin())
            .args(["mcp", "--workdir", &dir.path().to_string_lossy()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn mcp: {e}"))?,
    );

    let mut stdin = child.0.stdin.take().ok_or_else(|| String::from("missing mcp stdin"))?;
    let stdout = child.0.stdout.take().ok_or_else(|| String::from("missing mcp stdout"))?;
    let mut reader = BufReader::new(stdout);

    send_mcp(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": { "name": "test", "version": "1.0" } }
        }),
    )?;
    let _ = recv_mcp(&mut reader)?;
    send_mcp(&mut stdin, &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))?;

    send_mcp(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "open_file", "arguments": { "path": "A.curvebook" } } }),
    )?;
    let open_resp = recv_mcp(&mut reader)?;
    if open_resp.get("error").is_some() || open_resp["result"]["isError"] == Value::Bool(true) {
        return Err(format!("open_file failed unexpectedly: {open_resp}"));
    }

    send_mcp(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "command_run", "arguments": { "command_id": "project.new", "params": { "title": "Fresh Untitled" } } }
        }),
    )?;
    let new_resp = recv_mcp(&mut reader)?;
    if new_resp.get("error").is_some() || new_resp["result"]["isError"] == Value::Bool(true) {
        return Err(format!("project.new failed unexpectedly: {new_resp}"));
    }

    send_mcp(&mut stdin, &json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "save_file", "arguments": {} } }))?;
    let save_resp = recv_mcp(&mut reader)?;
    let is_rejected = save_resp.get("error").is_some() || save_resp["result"]["isError"] == Value::Bool(true);
    if !is_rejected {
        return Err(format!("expected save_file {{}} after project.new to fail, got {save_resp}"));
    }

    let after_bytes = fs::read(&path_a).map_err(|e| e.to_string())?;
    if after_bytes != original_bytes {
        return Err(String::from("A.curvebook was overwritten after project.new + save_file {}"));
    }

    Ok(())
}
