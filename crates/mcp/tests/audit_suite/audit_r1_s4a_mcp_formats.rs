//! Regression tests for Stage 4A `R1-S12-05` MCP file-open dirty tracking and undo preservation.

use std::fs;
use std::path::Path;

use curvebook_doc::{CellCoord, CellValue, SheetId};
use curvebook_format::{ZipWriter, save_project_file, save_project_unpacked_dir, save_template_file};
use curvebook_mcp::{McpServerState, call_tool};
use serde_json::{Value, json};

use super::common::{TempDir, parse_result_json};

type TestResult<T = ()> = Result<T, String>;

fn call_json(state: &mut McpServerState, name: &str, args: Value) -> TestResult<Value> {
    let result = call_tool(state, name, args).map_err(|err| format!("{name} protocol failed: {err:?}"))?;
    if result.is_error {
        return Err(format!("expected {name} to succeed, got error: {:?}", result.structured_content));
    }
    parse_result_json(&result)
}

fn read_y_cell_number(state: &McpServerState, sheet_id: &str, row: usize, col: usize) -> Option<f64> {
    let sheet = state.session().document().data_sheet(&SheetId::new(sheet_id))?;
    let cell = sheet.archetype.data_cell(CellCoord::new(row, col, 0))?;
    match cell.value {
        CellValue::Number(v) => Some(v),
        _ => None,
    }
}

#[test]
fn r1_s12_05_non_append_csv_and_tsv_mark_dirty_with_no_file_path() -> TestResult {
    let dir = TempDir::new_canonical("cb_r1_s12_05_csv_tsv")?;
    let csv_path = dir.child("alpha.csv");
    let tsv_path = dir.child("beta.tsv");
    fs::write(&csv_path, "X,Y\n1,10\n2,20\n").map_err(|err| format!("write csv: {err}"))?;
    fs::write(&tsv_path, "Value\n123\n456\n").map_err(|err| format!("write tsv: {err}"))?;

    let mut state = McpServerState::new();
    let csv_res = call_json(&mut state, "open_file", json!({ "path": csv_path.display().to_string() }))?;
    assert_eq!(csv_res["opened"], true);
    assert_eq!(csv_res["format"], "csv");
    assert_eq!(csv_res["sheet_id"], "alpha");
    assert_eq!(csv_res["rows"], 2);
    assert_eq!(csv_res["columns"], 1);
    assert_eq!(read_y_cell_number(&state, "alpha", 0, 0), Some(10.0));

    let inspect_csv = call_json(&mut state, "doc_inspect", json!({}))?;
    assert_eq!(inspect_csv["dirty"], true);
    assert_eq!(inspect_csv["file_path"], Value::Null);
    assert_eq!(inspect_csv["total_sheet_count"], 1);
    assert!(state.session().is_dirty());
    assert!(state.session().file_path().is_none());

    let tsv_res = call_json(
        &mut state,
        "open_file",
        json!({
            "path": tsv_path.display().to_string(),
            "target_kind": "multiple_variables"
        }),
    )?;
    assert_eq!(tsv_res["opened"], true);
    assert_eq!(tsv_res["format"], "tsv");
    assert_eq!(tsv_res["sheet_id"], "beta");
    assert_eq!(tsv_res["rows"], 2);
    assert_eq!(tsv_res["columns"], 1);

    let inspect_tsv = call_json(&mut state, "doc_inspect", json!({}))?;
    assert_eq!(inspect_tsv["dirty"], true);
    assert_eq!(inspect_tsv["file_path"], Value::Null);
    assert_eq!(inspect_tsv["total_sheet_count"], 1);
    assert!(state.session().is_dirty());
    assert!(state.session().file_path().is_none());
    Ok(())
}

#[test]
fn r1_s12_05_tsv_with_commas_keeps_tab_delimiter_on_open_and_append() -> TestResult {
    let dir = TempDir::new_canonical("cb_r1_s12_05_tsv_commas")?;
    let first_csv = dir.child("first.csv");
    let comma_tsv = dir.child("labels.tsv");
    fs::write(&first_csv, "X,Y\n1,2\n").map_err(|err| format!("write csv: {err}"))?;
    fs::write(&comma_tsv, "Label\n\"1,234\"\n5,678\n").map_err(|err| format!("write tsv: {err}"))?;

    let mut state = McpServerState::new();
    call_json(&mut state, "open_file", json!({ "path": first_csv.display().to_string() }))?;
    let appended = call_json(
        &mut state,
        "open_file",
        json!({
            "path": comma_tsv.display().to_string(),
            "append": true,
            "sheet_id": "labels",
            "target_kind": "multiple_variables"
        }),
    )?;
    assert_eq!(appended["format"], "tsv");
    assert_eq!(appended["columns"], 1);
    assert_eq!(appended["rows"], 2);
    assert!(state.session().is_dirty());
    assert!(state.session().file_path().is_none());
    Ok(())
}

#[test]
fn r1_s12_05_append_csv_preserves_prior_undo_and_marks_dirty() -> TestResult {
    let dir = TempDir::new_canonical("cb_r1_s12_05_append_undo")?;
    let a_path = dir.child("a.csv");
    let b_path = dir.child("b.csv");
    fs::write(&a_path, "X,Y\n1,2\n3,4\n").map_err(|err| format!("write a.csv: {err}"))?;
    fs::write(&b_path, "X,Y\n10,20\n").map_err(|err| format!("write b.csv: {err}"))?;

    let mut state = McpServerState::new();
    call_json(&mut state, "enable_toolset", json!({ "groups": ["table", "edit"] }))?;
    call_json(&mut state, "open_file", json!({ "path": a_path.display().to_string() }))?;
    assert_eq!(read_y_cell_number(&state, "a", 0, 0), Some(2.0));

    call_json(
        &mut state,
        "table_set_cell",
        json!({
            "sheet_id": "a",
            "row": 0,
            "col": 0,
            "value": 99.0
        }),
    )?;
    assert_eq!(read_y_cell_number(&state, "a", 0, 0), Some(99.0));

    let appended = call_json(
        &mut state,
        "open_file",
        json!({
            "path": b_path.display().to_string(),
            "append": true,
            "sheet_id": "b"
        }),
    )?;
    assert_eq!(appended["opened"], true);
    assert_eq!(appended["sheet_id"], "b");

    let inspect = call_json(&mut state, "doc_inspect", json!({}))?;
    assert_eq!(inspect["dirty"], true);
    assert_eq!(inspect["file_path"], Value::Null);
    assert_eq!(inspect["total_sheet_count"], 2);
    assert!(state.session().document().data_sheet(&SheetId::new("b")).is_some());
    assert_eq!(read_y_cell_number(&state, "a", 0, 0), Some(99.0));
    assert_eq!(read_y_cell_number(&state, "b", 0, 0), Some(20.0));

    // First undo removes the appended CSV sheet while preserving the prior table.setCell edit.
    call_json(&mut state, "edit_undo", json!({}))?;
    let after_first_undo = call_json(&mut state, "doc_inspect", json!({}))?;
    assert_eq!(after_first_undo["total_sheet_count"], 1);
    assert_eq!(after_first_undo["dirty"], true);
    assert!(state.session().document().data_sheet(&SheetId::new("b")).is_none());
    assert_eq!(read_y_cell_number(&state, "a", 0, 0), Some(99.0));

    // Second undo restores the original pre-setCell CSV Y cell value.
    call_json(&mut state, "edit_undo", json!({}))?;
    assert_eq!(read_y_cell_number(&state, "a", 0, 0), Some(2.0));

    // Redo restores both the cell edit and the appended CSV sheet.
    call_json(&mut state, "edit_redo", json!({}))?;
    assert_eq!(read_y_cell_number(&state, "a", 0, 0), Some(99.0));
    call_json(&mut state, "edit_redo", json!({}))?;
    assert!(state.session().document().data_sheet(&SheetId::new("b")).is_some());
    assert_eq!(read_y_cell_number(&state, "b", 0, 0), Some(20.0));
    Ok(())
}

#[test]
fn r1_s12_05_append_into_saved_project_preserves_path_marks_dirty_and_undoes() -> TestResult {
    let dir = TempDir::new_canonical("cb_r1_s12_05_append_proj")?;
    let seed_csv = dir.child("base.csv");
    let proj_path = dir.child("study.curvebook");
    let append_tsv = dir.child("extra.tsv");
    fs::write(&seed_csv, "X,Y\n1,5\n").map_err(|err| format!("write seed csv: {err}"))?;
    fs::write(&append_tsv, "X\tY\n7\t8\n").map_err(|err| format!("write extra tsv: {err}"))?;

    let mut seed_state = McpServerState::new();
    call_json(&mut seed_state, "open_file", json!({ "path": seed_csv.display().to_string() }))?;
    save_project_file(&proj_path, seed_state.session().document()).map_err(|err| format!("save project: {err}"))?;

    let mut state = McpServerState::new();
    call_json(&mut state, "enable_toolset", json!({ "groups": ["edit"] }))?;
    call_json(&mut state, "open_file", json!({ "path": proj_path.display().to_string() }))?;
    let clean_inspect = call_json(&mut state, "doc_inspect", json!({}))?;
    assert_eq!(clean_inspect["dirty"], false);
    assert_eq!(clean_inspect["file_path"], proj_path.display().to_string());

    call_json(
        &mut state,
        "open_file",
        json!({
            "path": append_tsv.display().to_string(),
            "append": true,
            "sheet_id": "extra"
        }),
    )?;

    let dirty_inspect = call_json(&mut state, "doc_inspect", json!({}))?;
    assert_eq!(dirty_inspect["dirty"], true);
    assert_eq!(dirty_inspect["file_path"], proj_path.display().to_string());
    assert_eq!(dirty_inspect["total_sheet_count"], 2);
    assert_eq!(read_y_cell_number(&state, "extra", 0, 0), Some(8.0));

    call_json(&mut state, "edit_undo", json!({}))?;
    assert_eq!(call_json(&mut state, "doc_inspect", json!({}))?["total_sheet_count"], 1);
    assert!(state.session().document().data_sheet(&SheetId::new("extra")).is_none());
    assert_eq!(state.session().file_path(), Some(proj_path.as_path()));
    Ok(())
}

#[test]
fn r1_s12_05_append_duplicate_sheet_id_errors_without_overwriting_or_losing_undo() -> TestResult {
    let dir = TempDir::new_canonical("cb_r1_s12_05_dup_sheet")?;
    let a_path = dir.child("a.csv");
    let b_path = dir.child("b.csv");
    fs::write(&a_path, "X,Y\n1,2\n").map_err(|err| format!("write a.csv: {err}"))?;
    fs::write(&b_path, "X,Y\n50,60\n").map_err(|err| format!("write b.csv: {err}"))?;

    let mut state = McpServerState::new();
    call_json(&mut state, "enable_toolset", json!({ "groups": ["table", "edit"] }))?;
    call_json(&mut state, "open_file", json!({ "path": a_path.display().to_string() }))?;
    call_json(
        &mut state,
        "table_set_cell",
        json!({
            "sheet_id": "a",
            "row": 0,
            "col": 0,
            "value": 42.0
        }),
    )?;

    let dup_res = call_tool(
        &mut state,
        "open_file",
        json!({
            "path": b_path.display().to_string(),
            "append": true,
            "sheet_id": "a"
        }),
    )
    .map_err(|err| format!("duplicate append protocol failed: {err:?}"))?;
    assert!(dup_res.is_error, "expected duplicate sheet_id append to fail");
    let err_json = parse_result_json(&dup_res)?;
    assert_eq!(err_json["code"], "already_exists");

    assert_eq!(call_json(&mut state, "doc_inspect", json!({}))?["total_sheet_count"], 1);
    assert_eq!(read_y_cell_number(&state, "a", 0, 0), Some(42.0));
    call_json(&mut state, "edit_undo", json!({}))?;
    assert_eq!(read_y_cell_number(&state, "a", 0, 0), Some(2.0));
    Ok(())
}

#[test]
fn r1_s12_05_pzfx_and_prism_imports_mark_dirty_with_null_file_path() -> TestResult {
    let dir = TempDir::new_canonical("cb_r1_s12_05_pzfx_prism")?;
    let pzfx_path = dir.child("curve.pzfx");
    let prism_path = dir.child("bundle.prism");

    let pzfx_xml = r#"<PrismFile>
  <Title>PZFX Fixture</Title>
  <Table ID="tbl1" TableType="XY">
    <Title>Dose Curve</Title>
    <XColumn><Title>X</Title><Subcolumn><d>1</d><d>2</d></Subcolumn></XColumn>
    <YColumn><Title>Y</Title><Subcolumn><d>10</d><d>20</d></Subcolumn></YColumn>
  </Table>
</PrismFile>"#;
    fs::write(&pzfx_path, pzfx_xml).map_err(|err| format!("write pzfx: {err}"))?;

    let mut zip = ZipWriter::new();
    zip.add_file("data.csv", b"X,Y\n1,10\n2,20\n".to_vec()).map_err(|err| format!("add csv to prism zip: {err}"))?;
    let zip_bytes = zip.finish().map_err(|err| format!("finish prism zip: {err}"))?;
    fs::write(&prism_path, zip_bytes).map_err(|err| format!("write prism: {err}"))?;

    let mut state = McpServerState::new();
    let pzfx_res = call_json(&mut state, "open_file", json!({ "path": pzfx_path.display().to_string() }))?;
    assert_eq!(pzfx_res["opened"], true);
    assert_eq!(pzfx_res["format"], "pzfx");
    assert!(pzfx_res["warnings"].is_array());
    assert_eq!(pzfx_res["total_sheet_count"], 1);

    let pzfx_inspect = call_json(&mut state, "doc_inspect", json!({}))?;
    assert_eq!(pzfx_inspect["dirty"], true);
    assert_eq!(pzfx_inspect["file_path"], Value::Null);
    assert!(state.session().is_dirty());
    assert!(state.session().file_path().is_none());

    let prism_res = call_json(&mut state, "open_file", json!({ "path": prism_path.display().to_string() }))?;
    assert_eq!(prism_res["opened"], true);
    assert_eq!(prism_res["format"], "prism");
    assert!(prism_res["warnings"].is_array());
    assert_eq!(prism_res["total_sheet_count"], 1);

    let prism_inspect = call_json(&mut state, "doc_inspect", json!({}))?;
    assert_eq!(prism_inspect["dirty"], true);
    assert_eq!(prism_inspect["file_path"], Value::Null);
    assert!(state.session().is_dirty());
    assert!(state.session().file_path().is_none());
    Ok(())
}

#[test]
fn r1_s12_05_curvebook_unpacked_and_template_semantics_stay_accurate() -> TestResult {
    let dir = TempDir::new_canonical("cb_r1_s12_05_proj_tpl")?;
    let seed_csv = dir.child("seed.csv");
    let proj_path = dir.child("saved.curvebook");
    let unpacked_dir = dir.child("saved.curvebook.d");
    let tpl_path = dir.child("starter.cbtemplate");
    fs::write(&seed_csv, "X,Y\n1,2\n").map_err(|err| format!("write seed csv: {err}"))?;

    let mut seed_state = McpServerState::new();
    call_json(&mut seed_state, "open_file", json!({ "path": seed_csv.display().to_string() }))?;
    save_project_file(&proj_path, seed_state.session().document()).map_err(|err| format!("save project: {err}"))?;
    save_project_unpacked_dir(seed_state.session().document(), &unpacked_dir).map_err(|err| format!("save unpacked: {err}"))?;
    save_template_file(&tpl_path, seed_state.session().document()).map_err(|err| format!("save template: {err}"))?;

    let mut state = McpServerState::new();
    call_json(&mut state, "open_file", json!({ "path": proj_path.display().to_string() }))?;
    let proj_inspect = call_json(&mut state, "doc_inspect", json!({}))?;
    assert_eq!(proj_inspect["dirty"], false);
    assert_eq!(proj_inspect["file_path"], proj_path.display().to_string());

    let unpacked_res = call_json(&mut state, "open_file", json!({ "path": unpacked_dir.display().to_string() }))?;
    assert_eq!(unpacked_res["format"], "curvebook.d");
    let unpacked_inspect = call_json(&mut state, "doc_inspect", json!({}))?;
    assert_eq!(unpacked_inspect["dirty"], false);
    let unpacked_file_path = unpacked_inspect["file_path"].as_str().ok_or_else(|| String::from("missing file_path"))?;
    assert_eq!(Path::new(unpacked_file_path), unpacked_dir.as_path());

    let tpl_res = call_json(&mut state, "open_file", json!({ "path": tpl_path.display().to_string(), "title": "Instantiated Study" }))?;
    assert_eq!(tpl_res["format"], "cbtemplate");
    assert_eq!(tpl_res["title"], "Instantiated Study");
    let tpl_inspect = call_json(&mut state, "doc_inspect", json!({}))?;
    assert_eq!(tpl_inspect["dirty"], false);
    assert_eq!(tpl_inspect["file_path"], Value::Null);
    Ok(())
}

#[test]
fn r1_s12_05_workdir_guard_allows_open_and_append_while_keeping_dirty_and_undo() -> TestResult {
    let dir = TempDir::new_canonical("cb_r1_s12_05_workdir")?;
    let a_path = dir.child("a.csv");
    let b_path = dir.child("b.csv");
    let outside_dir = TempDir::new_canonical("cb_r1_s12_05_outside")?;
    let outside_csv = outside_dir.child("outside.csv");
    fs::write(&a_path, "X,Y\n1,2\n").map_err(|err| format!("write a.csv: {err}"))?;
    fs::write(&b_path, "X,Y\n3,4\n").map_err(|err| format!("write b.csv: {err}"))?;
    fs::write(&outside_csv, "X,Y\n9,9\n").map_err(|err| format!("write outside.csv: {err}"))?;

    let mut state = McpServerState::with_workdir(&dir.path).map_err(|err| format!("workdir guard: {err}"))?;
    call_json(&mut state, "enable_toolset", json!({ "groups": ["edit"] }))?;

    call_json(&mut state, "open_file", json!({ "path": "a.csv" }))?;
    assert_eq!(call_json(&mut state, "doc_inspect", json!({}))?["dirty"], true);
    assert!(state.session().file_path().is_none());

    call_json(&mut state, "open_file", json!({ "path": "b.csv", "append": true, "sheet_id": "b" }))?;
    assert_eq!(call_json(&mut state, "doc_inspect", json!({}))?["total_sheet_count"], 2);
    assert_eq!(call_json(&mut state, "doc_inspect", json!({}))?["dirty"], true);

    let blocked = call_tool(&mut state, "open_file", json!({ "path": outside_csv.display().to_string() }))
        .map_err(|err| format!("blocked protocol failed: {err:?}"))?;
    assert!(blocked.is_error, "outside path must still be blocked");
    assert_eq!(call_json(&mut state, "doc_inspect", json!({}))?["total_sheet_count"], 2);

    call_json(&mut state, "edit_undo", json!({}))?;
    assert_eq!(call_json(&mut state, "doc_inspect", json!({}))?["total_sheet_count"], 1);
    Ok(())
}
