#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

use std::time::{SystemTime, UNIX_EPOCH};

use curvebook_doc::{Cell, TableArchetype};
use curvebook_engine::{ErrorCode, Session};
use curvebook_format::zip::ZipWriter;
use serde_json::{Value, json};

fn encode_base64(bytes: &[u8]) -> String {
    curvebook_platform::base64::encode(bytes)
}

fn build_xlsx_zip(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut zw = ZipWriter::new();
    for (name, xml) in entries {
        zw.add_file(name, xml.as_bytes().to_vec()).expect("add zip entry");
    }
    zw.finish().expect("finish zip")
}

fn multi_sheet_xlsx_bytes() -> Vec<u8> {
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Alpha" sheetId="1" r:id="rId1"/>
    <sheet name="Beta" sheetId="2" r:id="rId2"/>
    <sheet name="Secret" sheetId="3" state="hidden" r:id="rId3"/>
  </sheets>
</workbook>"#;
    let wb_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet3.xml"/>
</Relationships>"#;
    let sheet1 = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>Dose</t></is></c><c r="B1" t="inlineStr"><is><t>Control</t></is></c></row>
    <row r="2"><c r="A2" t="inlineStr"><is><t>uM</t></is></c><c r="B2" t="inlineStr"><is><t>Mean</t></is></c></row>
    <row r="3"><c r="A3"><v>1</v></c><c r="B3"><v>10.5</v></c></row>
    <row r="4"><c r="A4"><v>10</v></c><c r="B4"><v>55.25</v></c></row>
  </sheetData>
</worksheet>"#;
    let sheet2 = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>GroupA</t></is></c><c r="B1" t="inlineStr"><is><t>GroupB</t></is></c></row>
    <row r="2"><c r="A2"><v>4.5</v></c><c r="B2"><v>8.5</v></c></row>
  </sheetData>
</worksheet>"#;
    let sheet3 = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>HiddenCol</t></is></c></row>
    <row r="2"><c r="A2"><v>7</v></c></row>
  </sheetData>
</worksheet>"#;

    build_xlsx_zip(&[
        ("xl/workbook.xml", workbook),
        ("xl/_rels/workbook.xml.rels", wb_rels),
        ("xl/worksheets/sheet1.xml", sheet1),
        ("xl/worksheets/sheet2.xml", sheet2),
        ("xl/worksheets/sheet3.xml", sheet3),
    ])
}

#[test]
fn engine_import_xlsx_lists_sheets_selects_subsets_and_handles_merge_and_file_path_detachment() {
    let bytes = multi_sheet_xlsx_bytes();
    let b64 = encode_base64(&bytes);
    let mut session = Session::new();

    // 1. Enumerate worksheets without mutating session.
    let list_out = session.execute("table.importXlsx", json!({ "bytes_base64": b64, "list_sheets": true })).expect("list_sheets");
    assert!(!list_out.mutated);
    assert_eq!(list_out.payload["sheet_count"], 3);
    assert_eq!(list_out.payload["sheets"][0]["name"], "Alpha");
    assert_eq!(list_out.payload["sheets"][2]["state"], "hidden");
    assert_eq!(session.document().data_sheets.len(), 0);

    // 2. Selective multi-row header import on Alpha only (`merge: false` detaches prior archive path).
    session.set_file_path(Some(std::path::PathBuf::from("/tmp/previous.curvebook")), true);
    let alpha_out = session
        .execute(
            "table.importXlsx",
            json!({
                "bytes_base64": encode_base64(&bytes),
                "sheets": ["Alpha"],
                "header_rows": 2,
                "merge": false,
            }),
        )
        .expect("import Alpha");
    assert!(alpha_out.mutated);
    assert!(session.file_path().is_none(), "merge=false replace must clear prior archive path");
    assert_eq!(session.document().data_sheets.len(), 1);
    let alpha = &session.document().data_sheets[0];
    assert_eq!(alpha.title, "Alpha");
    let TableArchetype::Xy(xy) = &alpha.archetype else {
        panic!("expected XY archetype");
    };
    assert_eq!(xy.x_title, "Dose - uM");
    assert_eq!(xy.data_sets[0].title, "Control - Mean");
    assert_eq!(xy.data_sets[0].cells[1][0], Cell::number(55.25));

    // 3. Merge Beta + Secret by sheet_indices without discarding Alpha.
    let merge_out = session
        .execute(
            "table.importXlsx",
            json!({
                "bytes_base64": encode_base64(&bytes),
                "sheet_indices": [1, 2],
                "merge": true,
            }),
        )
        .expect("merge sheet_indices");
    assert!(merge_out.mutated);
    let titles: Vec<&str> = session.document().data_sheets.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, vec!["Alpha", "Beta", "Secret"]);

    // 4. Reject unknown field cleanly with BadInput.
    let err = session
        .execute("table.importXlsx", json!({ "bytes_base64": encode_base64(&bytes), "unknown_flag": true }))
        .expect_err("unknown field must fail");
    assert_eq!(err.code, ErrorCode::BadInput);
}

#[test]
fn engine_import_csv_and_paste_harden_utf16_bom_thousands_decimal_comma_and_quoted_newlines() {
    let mut session = Session::new();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("cb-r17-1-eng-{nonce}"));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let utf16_path = dir.join("eu_data.tsv");

    let text = "Treatment\tDose\nControl\t1.234,56\n\"Drug A\nLine2\"\t2,50*\nOnlyCol0\n";
    let mut raw_le = vec![0xFF, 0xFE];
    for u in text.encode_utf16() {
        raw_le.extend_from_slice(&u.to_le_bytes());
    }
    std::fs::write(&utf16_path, &raw_le).expect("write utf16 tsv");

    let out = session
        .execute(
            "table.importCsv",
            json!({
                "path": utf16_path.to_string_lossy(),
                "archetype": "MultipleVariables",
            }),
        )
        .expect("import utf16 tsv");
    let sid = out.payload["sheet_id"].as_str().expect("sheet_id").to_owned();
    let ds = session.document().data_sheet(&sid.as_str().into()).expect("sheet present");
    let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
        panic!("expected MV");
    };
    assert_eq!(mv.variables[1].cells[0], Cell::number(1234.56));
    assert_eq!(mv.variables[0].cells[1], Cell::text("Drug A\nLine2"));
    assert_eq!(mv.variables[1].cells[1], Cell { value: curvebook_doc::CellValue::Number(2.5), excluded: true });
    assert_eq!(mv.variables[1].cells[2], Cell::empty());

    // Clipboard paste with US thousands + comma decimal TSV auto-detection
    let _ = session
        .execute(
            "table.paste",
            json!({
                "sheet_id": sid,
                "text": "1,234.56\t\"Line1\nLine2\"\n",
                "start_row": 0,
                "start_col": 0,
            }),
        )
        .expect("paste messy tsv");
    let ds_after = session.document().data_sheet(&sid.as_str().into()).expect("sheet");
    let TableArchetype::MultipleVariables(mv_after) = &ds_after.archetype else {
        panic!("expected MV");
    };
    assert_eq!(mv_after.variables[1].cells[0], Cell::text("Line1\nLine2"));

    let _ = std::fs::remove_dir_all(&dir);
    let _: Value = out.payload;
}
