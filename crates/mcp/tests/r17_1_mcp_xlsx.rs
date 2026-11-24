#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::{Cell, TableArchetype};
use curvebook_format::zip::ZipWriter;
use curvebook_mcp::{ContentBlock, McpServerState, ToolCallResult, call_tool};
use serde_json::{Value, json};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDir(PathBuf);

impl TempDir {
    fn new(prefix: &str) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("{prefix}_{}_{seq}", std::process::id()));
        fs::create_dir_all(&dir).map_err(|e| format!("mkdir: {e}"))?;
        Ok(Self(fs::canonicalize(&dir).unwrap_or(dir)))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn call_json(state: &mut McpServerState, name: &str, args: Value) -> Result<Value, String> {
    let result: ToolCallResult = call_tool(state, name, args).map_err(|err| format!("{name}: {err:?}"))?;
    if result.is_error {
        return Err(format!("{name} failed: {:?}", result.structured_content));
    }
    if let Some(sc) = result.structured_content {
        return Ok(sc);
    }
    let text = result
        .content
        .iter()
        .find_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            ContentBlock::Image { .. } => None,
        })
        .ok_or("missing text block")?;
    serde_json::from_str(text).map_err(|e| format!("parse json: {e}"))
}

fn build_xlsx_zip(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut zw = ZipWriter::new();
    for (name, xml) in entries {
        zw.add_file(name, xml.as_bytes().to_vec()).expect("add zip entry");
    }
    zw.finish().expect("finish zip")
}

#[test]
fn mcp_open_file_and_table_import_xlsx_enumerate_and_select_sheets() -> Result<(), String> {
    let dir = TempDir::new("cb_r17_1_mcp_xlsx")?;
    let xlsx_path = dir.0.join("study.xlsx");

    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="PlateA" sheetId="1" r:id="rId1"/>
    <sheet name="PlateB" sheetId="2" r:id="rId2"/>
  </sheets>
</workbook>"#;
    let wb_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
</Relationships>"#;
    let s1 = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
  <row r="1"><c r="A1" t="inlineStr"><is><t>Dose</t></is></c><c r="B1" t="inlineStr"><is><t>Val</t></is></c></row>
  <row r="2"><c r="A2"><v>1</v></c><c r="B2"><v>10</v></c></row>
</sheetData></worksheet>"#;
    let s2 = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
  <row r="1"><c r="A1" t="inlineStr"><is><t>Ctrl</t></is></c><c r="B1" t="inlineStr"><is><t>Treated</t></is></c></row>
  <row r="2"><c r="A2"><v>3.5</v></c><c r="B2"><v>7.5</v></c></row>
</sheetData></worksheet>"#;

    let bytes = build_xlsx_zip(&[
        ("xl/workbook.xml", workbook),
        ("xl/_rels/workbook.xml.rels", wb_rels),
        ("xl/worksheets/sheet1.xml", s1),
        ("xl/worksheets/sheet2.xml", s2),
    ]);
    fs::write(&xlsx_path, &bytes).map_err(|e| format!("write xlsx: {e}"))?;

    let mut state = McpServerState::new();
    let list_res = call_json(&mut state, "open_file", json!({ "path": xlsx_path.display().to_string(), "list_sheets": true }))?;
    assert_eq!(list_res["sheet_count"], 2);
    assert_eq!(list_res["sheets"][0]["name"], "PlateA");
    assert_eq!(list_res["sheets"][1]["name"], "PlateB");

    let open_res = call_json(
        &mut state,
        "open_file",
        json!({
            "path": xlsx_path.display().to_string(),
            "sheets": ["PlateB"],
        }),
    )?;
    assert_eq!(open_res["opened"], true);
    assert_eq!(open_res["format"], "xlsx");
    assert_eq!(state.session().document().data_sheets.len(), 1);
    assert_eq!(state.session().document().data_sheets[0].title, "PlateB");
    let TableArchetype::Xy(xy) = &state.session().document().data_sheets[0].archetype else {
        return Err(String::from("expected XY archetype from PlateB"));
    };
    assert_eq!(xy.x_cells[0][0], Cell::number(3.5));
    assert_eq!(xy.data_sets[0].cells[0][0], Cell::number(7.5));

    // Also test direct engine-mirrored tool `table_import_xlsx` appending sheet index 0
    let _ = call_json(&mut state, "enable_toolset", json!({ "group": "table" }))?;
    let imp_res = call_json(
        &mut state,
        "table_import_xlsx",
        json!({
            "path": xlsx_path.display().to_string(),
            "sheet_indices": [0],
            "merge": true,
        }),
    )?;
    assert_eq!(imp_res["command_id"], "table.importXlsx");
    assert_eq!(imp_res["mutated"], true);
    assert_eq!(state.session().document().data_sheets.len(), 2);
    Ok(())
}
