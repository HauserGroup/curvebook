#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

use curvebook_doc::{Cell, CellValue, TableArchetype};
use curvebook_engine::Session;
use curvebook_format::zip::ZipWriter;
use serde_json::json;

use super::common::{TempDir, assert_bad_input, ds, encode_base64, ok_cmd};

fn multi_sheet_xlsx_bytes() -> Vec<u8> {
    let entries = [
        (
            "xl/workbook.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Alpha" sheetId="1" r:id="rId1"/>
    <sheet name="Beta" sheetId="2" r:id="rId2"/>
    <sheet name="Secret" sheetId="3" state="hidden" r:id="rId3"/>
  </sheets>
</workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet3.xml"/>
</Relationships>"#,
        ),
        (
            "xl/worksheets/sheet1.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
  <row r="1"><c r="A1" t="inlineStr"><is><t>Dose</t></is></c><c r="B1" t="inlineStr"><is><t>Control</t></is></c></row>
  <row r="2"><c r="A2" t="inlineStr"><is><t>uM</t></is></c><c r="B2" t="inlineStr"><is><t>Mean</t></is></c></row>
  <row r="3"><c r="A3"><v>1</v></c><c r="B3"><v>10.5</v></c></row>
  <row r="4"><c r="A4"><v>10</v></c><c r="B4"><v>55.25</v></c></row>
</sheetData></worksheet>"#,
        ),
        (
            "xl/worksheets/sheet2.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
  <row r="1"><c r="A1" t="inlineStr"><is><t>GroupA</t></is></c><c r="B1" t="inlineStr"><is><t>GroupB</t></is></c></row>
  <row r="2"><c r="A2"><v>4.5</v></c><c r="B2"><v>8.5</v></c></row>
</sheetData></worksheet>"#,
        ),
        (
            "xl/worksheets/sheet3.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
  <row r="1"><c r="A1" t="inlineStr"><is><t>HiddenCol</t></is></c></row>
  <row r="2"><c r="A2"><v>7</v></c></row>
</sheetData></worksheet>"#,
        ),
    ];
    let mut zw = ZipWriter::new();
    for (name, xml) in entries {
        zw.add_file(name, xml.as_bytes().to_vec()).expect("add zip entry");
    }
    zw.finish().expect("finish zip")
}

#[test]
fn engine_import_xlsx_lists_sheets_selects_subsets_and_handles_merge_and_file_path_detachment() {
    let b64 = encode_base64(&multi_sheet_xlsx_bytes());
    let mut session = Session::new();

    // 1. Enumerate worksheets without mutating session.
    let list_out = ok_cmd(&mut session, "table.importXlsx", json!({ "bytes_base64": b64, "list_sheets": true }));
    assert!(!list_out.mutated && session.document().data_sheets.is_empty());
    assert_eq!(
        (&list_out.payload["sheet_count"], &list_out.payload["sheets"][0]["name"], &list_out.payload["sheets"][2]["state"]),
        (&json!(3), &json!("Alpha"), &json!("hidden"))
    );

    // 2. Selective multi-row header import on Alpha only (`merge: false` detaches prior archive path).
    session.set_file_path(Some(std::path::PathBuf::from("/tmp/previous.curvebook")), true);
    let alpha_out = ok_cmd(&mut session, "table.importXlsx", json!({ "bytes_base64": b64, "sheets": ["Alpha"], "header_rows": 2, "merge": false }));
    assert!(alpha_out.mutated && session.file_path().is_none() && session.document().data_sheets.len() == 1);
    let alpha = &session.document().data_sheets[0];
    assert_eq!(alpha.title, "Alpha");
    let TableArchetype::Xy(xy) = &alpha.archetype else { panic!("expected XY archetype") };
    assert_eq!(
        (xy.x_title.as_str(), xy.data_sets[0].title.as_str(), &xy.data_sets[0].cells[1][0]),
        ("Dose - uM", "Control - Mean", &Cell::number(55.25))
    );

    // 3. Merge Beta + Secret by sheet_indices without discarding Alpha.
    let merge_out = ok_cmd(&mut session, "table.importXlsx", json!({ "bytes_base64": b64, "sheet_indices": [1, 2], "merge": true }));
    assert!(merge_out.mutated);
    let titles: Vec<&str> = session.document().data_sheets.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, vec!["Alpha", "Beta", "Secret"]);

    // 4. Reject unknown field cleanly with BadInput; AUD-13-14: honor explicit title override & reject blank title.
    assert_bad_input(&mut session, "table.importXlsx", json!({ "bytes_base64": b64, "unknown_flag": true }));
    let title_out = ok_cmd(&mut session, "table.importXlsx", json!({ "bytes_base64": b64, "title": "Custom Workbook Title" }));
    assert_eq!((&title_out.payload["title"], session.document().title.as_str()), (&json!("Custom Workbook Title"), "Custom Workbook Title"));
    assert_bad_input(&mut session, "table.importXlsx", json!({ "bytes_base64": b64, "title": "   " }));
}

#[test]
fn engine_import_csv_and_paste_harden_utf16_bom_thousands_decimal_comma_and_quoted_newlines() {
    let mut session = Session::new();
    let dir = TempDir::new("r17-1-eng");
    let utf16_path = dir.child("eu_data.tsv");

    let text = "Treatment\tDose\nControl\t1.234,56\n\"Drug A\nLine2\"\t2,50*\nOnlyCol0\n";
    let mut raw_le = vec![0xFF, 0xFE];
    for u in text.encode_utf16() {
        raw_le.extend_from_slice(&u.to_le_bytes());
    }
    std::fs::write(&utf16_path, &raw_le).expect("write utf16 tsv");

    let out = ok_cmd(&mut session, "table.importCsv", json!({ "path": utf16_path.to_string_lossy(), "archetype": "MultipleVariables" }));
    let sid = out.payload["sheet_id"].as_str().expect("sheet_id").to_owned();
    let TableArchetype::MultipleVariables(mv) = &ds(&session, &sid).archetype else { panic!("expected MV") };
    assert_eq!(mv.variables[1].cells[0], Cell::number(1234.56));
    assert_eq!(mv.variables[0].cells[1], Cell::text("Drug A\nLine2"));
    assert_eq!(mv.variables[1].cells[1], Cell { value: CellValue::Number(2.5), excluded: true });
    assert_eq!(mv.variables[1].cells[2], Cell::empty());

    // Clipboard paste with US thousands + comma decimal TSV auto-detection
    ok_cmd(&mut session, "table.paste", json!({ "sheet_id": sid, "text": "1,234.56\t\"Line1\nLine2\"\n", "start_row": 0, "start_col": 0 }));
    let TableArchetype::MultipleVariables(mv_after) = &ds(&session, &sid).archetype else { panic!("expected MV") };
    assert_eq!(mv_after.variables[1].cells[0], Cell::text("Line1\nLine2"));
}
