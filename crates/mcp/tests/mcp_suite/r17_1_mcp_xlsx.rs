#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

use std::fs;
use std::sync::Arc;

use curvebook_doc::{Cell, TableArchetype};
use curvebook_format::zip::ZipWriter;
use curvebook_mcp::{McpServerState, ToolCallError, ToolCallResult, call_tool, list_tools};
use serde_json::{Value, json};

use crate::common::{TempDir, parse_result_json};

fn call_json(state: &mut McpServerState, name: &str, args: Value) -> Result<Value, String> {
    let result: ToolCallResult = call_tool(state, name, args).map_err(|err| format!("{name}: {err:?}"))?;
    if result.is_error {
        return Err(format!("{name} failed: {:?}", result.structured_content));
    }
    parse_result_json(&result)
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
    let dir = TempDir::new_canonical("cb_r17_1_mcp_xlsx")?;
    let xlsx_path = dir.child("study.xlsx");

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

#[test]
fn mcp_table_import_xlsx_forwards_r17_7_options_and_preamble_info() -> Result<(), String> {
    let dir = TempDir::new_canonical("cb_r17_7_mcp_xlsx")?;
    let xlsx_path = dir.child("kinetic.xlsx");

    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Kinetic" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#;
    let wb_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;
    let s1 = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
  <row r="1"><c r="A1" t="inlineStr"><is><t>Assay: Dual Read</t></is></c></row>
  <row r="2"><c r="A2" t="inlineStr"><is><t>Channel</t></is></c><c r="B2" t="inlineStr"><is><t>Time</t></is></c><c r="C2" t="inlineStr"><is><t>A01</t></is></c></row>
  <row r="3"><c r="A3" t="inlineStr"><is><t>ChA</t></is></c><c r="B3"><v>0</v></c><c r="C3"><v>5</v></c></row>
  <row r="4"><c r="A4" t="inlineStr"><is><t>ChA</t></is></c><c r="B4"><v>10</v></c><c r="C4"><v>15</v></c></row>
  <row r="5"><c r="A5" t="inlineStr"><is><t>ChB</t></is></c><c r="B5"><v>0</v></c><c r="C5"><v>2</v></c></row>
  <row r="6"><c r="A6" t="inlineStr"><is><t>ChB</t></is></c><c r="B6"><v>10</v></c><c r="C6"><v>4</v></c></row>
</sheetData></worksheet>"#;

    let bytes = build_xlsx_zip(&[("xl/workbook.xml", workbook), ("xl/_rels/workbook.xml.rels", wb_rels), ("xl/worksheets/sheet1.xml", s1)]);
    fs::write(&xlsx_path, &bytes).map_err(|e| format!("write xlsx: {e}"))?;

    let mut state = McpServerState::new();
    let _ = call_json(&mut state, "enable_toolset", json!({ "group": "table" }))?;
    let imp_res = call_json(
        &mut state,
        "table_import_xlsx",
        json!({
            "path": xlsx_path.display().to_string(),
            "skip_rows": 1,
            "split_by_column": 0,
            "extract_preamble_info": true,
        }),
    )?;
    assert_eq!(imp_res["command_id"], "table.importXlsx");
    assert_eq!(imp_res["mutated"], true);
    assert_eq!(state.session().document().data_sheets.len(), 2);
    assert_eq!(state.session().document().info_sheets.len(), 1);
    assert!(imp_res["payload"]["info_sheet_id"].as_str().is_some());
    Ok(())
}

#[test]
fn open_file_subcolumn_schema_xlsx_title_append_and_live_csv_escape_fallback() -> Result<(), String> {
    let tools = list_tools();
    let desc = tools.iter().find(|t| t.name == "open_file").ok_or("missing open_file")?.input_schema["properties"]["subcolumn_mode"]["description"]
        .as_str()
        .ok_or("missing desc")?;
    assert!(desc.contains("single") && desc.contains("upper_lower_limits") && desc.contains("'mean_sd'") && desc.contains("'mean_sem'"));

    let dir = TempDir::new("open_file_rem")?;
    let csv_path = dir.child("data.csv");
    fs::write(&csv_path, "X,Y\n1,10\n2,20\n").map_err(|e| e.to_string())?;
    let mut state = McpServerState::new();
    assert!(
        !call_tool(&mut state, "open_file", json!({ "path": csv_path.display().to_string(), "subcolumn_mode": "none" }))
            .map_err(|e| e.to_string())?
            .is_error
    );
    let csv_msd = dir.child("data_msd.csv");
    fs::write(&csv_msd, "X,Y:Mean,Y:SD\n1,10,1.2\n2,20,2.3\n").map_err(|e| e.to_string())?;
    assert!(
        !call_tool(&mut state, "open_file", json!({ "path": csv_msd.display().to_string(), "subcolumn_mode": "mean_sd" }))
            .map_err(|e| e.to_string())?
            .is_error
    );
    assert!(matches!(
        call_tool(&mut state, "open_file", json!({ "path": csv_path.display().to_string(), "subcolumn_mode": "mean_invalid" })),
        Err(ToolCallError::InvalidParams(ref m)) if m.contains("mean_sd_n")
    ));

    let xlsx_path = dir.child("dose_study.xlsx");
    let xlsx = build_xlsx_zip(&[
        (
            "xl/workbook.xml",
            r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Assay" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
        ),
        (
            "xl/worksheets/sheet1.xml",
            r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Dose</t></is></c><c r="B1" t="inlineStr"><is><t>Response</t></is></c></row><row r="2"><c r="A2"><v>1</v></c><c r="B2"><v>10</v></c></row></sheetData></worksheet>"#,
        ),
    ]);
    fs::write(&xlsx_path, xlsx).map_err(|e| e.to_string())?;

    let _ = call_json(&mut state, "open_file", json!({ "path": xlsx_path.display().to_string() }))?;
    assert_eq!(state.session().document().title, "dose_study");
    let _ = call_json(&mut state, "open_file", json!({ "path": xlsx_path.display().to_string(), "title": "Explicit Study Title" }))?;
    assert_eq!(state.session().document().title, "Explicit Study Title");
    let _ = call_json(&mut state, "open_file", json!({ "path": xlsx_path.display().to_string(), "append": true }))?;
    assert!(state.session().document().data_sheets.len() >= 2);

    // Live-attach: ~3.5 MiB control-char TSV whose JSON-escaped payload exceeds 16 MiB falls back to `path` mode,
    // and `.xlsx` with explicit `title` sends `file.new {title}` + `table.importXlsx {merge: true}`.
    let seen = Arc::new(curvebook_platform::sync::Mutex::named("test.open_file_live", Vec::<Value>::new()));
    let seen_c = Arc::clone(&seen);
    let mut live = McpServerState::with_control_port(49814).map_err(|e| e.to_string())?.with_control_handler(move |method, params| match method {
        "command.batch" => {
            if let Ok(mut g) = seen_c.lock() {
                g.push(params.clone());
            }
            Ok(json!({ "results": [{ "ok": true }, { "ok": true }] }))
        }
        "doc.inspect" => Ok(json!({ "title": "Live Title", "counts": { "total": 1 }, "kind": "xy", "rows": 2, "cols": 2 })),
        _ => Ok(json!({})),
    });
    let mut heavy = String::from("X\tY\n");
    let chunk = "\u{000b}".repeat(1024);
    for _ in 0..3_200 {
        heavy.push_str("1\t");
        heavy.push_str(&chunk);
        heavy.push('\n');
    }
    assert!(heavy.len() < 8 * 1024 * 1024);
    let heavy_path = dir.child("heavy_escaped.tsv");
    fs::write(&heavy_path, &heavy).map_err(|e| e.to_string())?;
    let _ = call_json(&mut live, "open_file", json!({ "path": heavy_path.display().to_string() }))?;
    let _ = call_json(&mut live, "open_file", json!({ "path": xlsx_path.display().to_string(), "title": "Live Custom Title" }))?;

    let rec = seen.lock().map_err(|e| e.to_string())?;
    assert_eq!(rec.len(), 2);
    assert!(rec[0]["commands"][1]["params"].get("path").is_some() && rec[0]["commands"][1]["params"].get("csv").is_none());
    assert_eq!((&rec[1]["commands"][0]["params"]["title"], &rec[1]["commands"][1]["params"]["merge"]), (&json!("Live Custom Title"), &json!(true)));
    Ok(())
}
