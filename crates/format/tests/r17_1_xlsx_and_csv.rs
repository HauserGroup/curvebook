#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

use std::path::Path;

use curvebook_doc::{Cell, SubcolumnMode, TableArchetype, TableKind};
use curvebook_format::zip::ZipWriter;
use curvebook_format::{
    CsvOptions, FormatKind, FormatRegistry, XlsxImportOptions, decode_delimited_text_bytes, import_csv_to_datasheet, import_xlsx_bytes,
    import_xlsx_bytes_with_options, list_xlsx_sheets,
};

fn build_xlsx_zip(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut zw = ZipWriter::new();
    for (name, xml) in entries {
        zw.add_file(name, xml.as_bytes().to_vec()).expect("add zip entry");
    }
    zw.finish().expect("finish zip")
}

#[test]
fn xlsx_registry_multi_sheet_selection_sparse_cells_and_merged_headers() {
    let rels_root = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="DoseResponse" sheetId="1" r:id="rId1"/>
    <sheet name="HiddenQC" sheetId="2" state="hidden" r:id="rId2"/>
    <sheet name="Summary" sheetId="3" r:id="rId3"/>
  </sheets>
</workbook>"#;
    let wb_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet3.xml"/>
</Relationships>"#;
    let shared = r#"<?xml version="1.0" encoding="UTF-8"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="4" uniqueCount="4">
  <si><t>Dose</t></si>
  <si><t>Treatment</t></si>
  <si><r><t xml:space="preserve">Rep </t></r><r><t>1</t></r></si>
  <si><t>Rep 2</t></si>
</sst>"#;
    let sheet1 = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row>
    <row r="2"><c r="B2" t="s"><v>2</v></c><c r="C2" t="s"><v>3</v></c></row>
    <row r="3"><c r="A3"><v>0.1</v></c><c r="B3"><v>12.5</v></c><c r="C3"><f>B3*2</f><v>25</v></c></row>
    <row r="5"><c r="A5"><v>1.0</v></c><c r="B5" t="inlineStr"><is><t>Note</t></is></c><c r="D5" t="b"><v>1</v></c></row>
    <row r="6"><c r="B6" t="str"><f>"OK"</f><v>OK</v></c><c r="C6" t="e"><v>#DIV/0!</v></c></row>
  </sheetData>
  <mergeCells count="1"><mergeCell ref="B1:C1"/></mergeCells>
</worksheet>"#;
    let sheet2 = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>QC</t></is></c></row>
    <row r="2"><c r="A2"><v>99.5</v></c></row>
  </sheetData>
</worksheet>"#;
    let sheet3 = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>X</t></is></c><c r="B1" t="inlineStr"><is><t>Y</t></is></c></row>
    <row r="2"><c r="A2"><v>1</v></c><c r="B2"><v>42</v></c></row>
  </sheetData>
</worksheet>"#;

    let bytes = build_xlsx_zip(&[
        ("_rels/.rels", rels_root),
        ("xl/workbook.xml", workbook),
        ("xl/_rels/workbook.xml.rels", wb_rels),
        ("xl/sharedStrings.xml", shared),
        ("xl/worksheets/sheet1.xml", sheet1),
        ("xl/worksheets/sheet2.xml", sheet2),
        ("xl/worksheets/sheet3.xml", sheet3),
    ]);

    let reg = FormatRegistry::default_registry();
    let importer = reg.importer_for_path(Path::new("plate_run.xlsx")).expect("xlsx importer registered");
    assert_eq!(importer.descriptor().id, "xlsx");
    assert_eq!(importer.descriptor().kind, FormatKind::ForeignProject);

    let listed = list_xlsx_sheets(&bytes).expect("list sheets");
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[1].state.as_deref(), Some("hidden"));

    // Default skips hidden worksheet
    let default_rep = import_xlsx_bytes(&bytes).expect("default import");
    let default_titles: Vec<&str> = default_rep.document.data_sheets.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(default_titles, vec!["DoseResponse", "Summary"]);

    // Explicit multi-sheet selection including merged headers + formula/boolean/sparse/error cells
    let multi = import_xlsx_bytes_with_options(
        &bytes,
        &XlsxImportOptions {
            sheet_names: Some(vec![String::from("DoseResponse"), String::from("HiddenQC")]),
            header_rows: 2,
            ..XlsxImportOptions::default()
        },
    )
    .expect("multi-sheet selection");
    assert_eq!(multi.document.data_sheets.len(), 2);
    assert!(multi.warnings.iter().any(|w| w.message.contains("formula cell(s)")));
    assert!(multi.warnings.iter().any(|w| w.message.contains("#DIV/0!")));

    let s1 = &multi.document.data_sheets[0];
    let TableArchetype::MultipleVariables(mv) = &s1.archetype else {
        panic!("expected MultipleVariables due to mixed text/bool cells");
    };
    assert_eq!(mv.variables[0].title, "Dose");
    assert_eq!(mv.variables[1].title, "Treatment - Rep 1");
    assert_eq!(mv.variables[2].title, "Treatment - Rep 2");
    assert_eq!(mv.variables[0].cells[0], Cell::number(0.1));
    assert_eq!(mv.variables[2].cells[0], Cell::number(25.0));
    assert_eq!(mv.variables[0].cells[1], Cell::empty()); // sparse empty row 4 in Excel
    assert_eq!(mv.variables[1].cells[2], Cell::text("Note"));
    assert_eq!(mv.variables[3].cells[2], Cell::text("TRUE"));
    assert_eq!(mv.variables[1].cells[3], Cell::text("OK"));
    assert_eq!(mv.variables[2].cells[3], Cell::text("#DIV/0!"));

    // Sheet index selection
    let by_idx = import_xlsx_bytes_with_options(&bytes, &XlsxImportOptions { sheet_indices: Some(vec![2]), ..XlsxImportOptions::default() })
        .expect("import by sheet index");
    assert_eq!(by_idx.document.data_sheets.len(), 1);
    assert_eq!(by_idx.document.data_sheets[0].title, "Summary");
}

#[test]
fn delimited_hardening_covers_utf16_boms_thousands_locales_multiline_headers_and_ragged_rows() {
    // 1. UTF-8 BOM + UTF-16 LE BOM + UTF-16 BE BOM with non-ASCII 'µ'
    let src = "Conc (µM)\tResp\n\"Line 1\nLine 2\"\t12,34\n10\n";
    let mut utf8_bom = vec![0xEF, 0xBB, 0xBF];
    utf8_bom.extend_from_slice(src.as_bytes());
    assert_eq!(decode_delimited_text_bytes(&utf8_bom, "utf8").expect("utf8 bom"), src);

    let mut utf16_le = vec![0xFF, 0xFE];
    for u in src.encode_utf16() {
        utf16_le.extend_from_slice(&u.to_le_bytes());
    }
    let decoded_le = decode_delimited_text_bytes(&utf16_le, "utf16le").expect("utf16le bom");
    assert_eq!(decoded_le, src);

    let mut utf16_be = vec![0xFE, 0xFF];
    for u in src.encode_utf16() {
        utf16_be.extend_from_slice(&u.to_be_bytes());
    }
    assert_eq!(decode_delimited_text_bytes(&utf16_be, "utf16be").expect("utf16be bom"), src);

    // 2. Quoted multiline cell + ragged rows + EU decimal comma + multi-row headers via UTF-16 LE bytes
    let messy_eu_tsv = concat!("Group A\t\tGroup B\n", "Rep 1\tRep 2\tRep 1\n", "1.234,56\t2.345,67\t3,5*\n", "\"row 2\ncontinuation\"\t4,25\n",);
    let mut eu_utf16_le = vec![0xFF, 0xFE];
    for u in messy_eu_tsv.encode_utf16() {
        eu_utf16_le.extend_from_slice(&u.to_le_bytes());
    }
    let decoded_messy = decode_delimited_text_bytes(&eu_utf16_le, "tsv").unwrap();
    let eu_ds = import_csv_to_datasheet(
        "eu-tsv",
        "EU Assay",
        &decoded_messy,
        TableKind::Column,
        SubcolumnMode::Single,
        &CsvOptions { header_rows: 2, ..CsvOptions::detect_tsv(&decoded_messy) },
    )
    .expect("import messy EU UTF-16LE TSV");
    let TableArchetype::Column(col) = &eu_ds.archetype else {
        panic!("expected Column table");
    };
    assert_eq!(col.data_sets.len(), 3);
    assert_eq!(col.data_sets[0].title, "Group A - Rep 1");
    assert_eq!(col.data_sets[1].title, "Group A - Rep 2");
    assert_eq!(col.data_sets[2].title, "Group B - Rep 1");
    assert_eq!(col.data_sets[0].cells[0][0], Cell::number(1234.56));
    assert_eq!(col.data_sets[1].cells[0][0], Cell::number(2345.67));
    assert_eq!(col.data_sets[2].cells[0][0], Cell { value: curvebook_doc::CellValue::Number(3.5), excluded: true });
    assert_eq!(col.data_sets[0].cells[1][0], Cell::text("row 2\ncontinuation"));
    assert_eq!(col.data_sets[1].cells[1][0], Cell::number(4.25));
    assert_eq!(col.data_sets[2].cells[1][0], Cell::empty()); // ragged row padded cleanly

    // 3. US thousands grouping (1,234.56) in TSV vs locale decimal comma (1,23)
    let us_tsv = "Dose\tSignal\n1,234.56\t2,500.75\n";
    let us_ds = import_csv_to_datasheet("us-tsv", "US Thousands", us_tsv, TableKind::Xy, SubcolumnMode::Single, &CsvOptions::detect_tsv(us_tsv))
        .expect("US thousands");
    let TableArchetype::Xy(xy) = &us_ds.archetype else {
        panic!("expected XY table");
    };
    assert_eq!(xy.x_cells[0][0], Cell::number(1234.56));
    assert_eq!(xy.data_sets[0].cells[0][0], Cell::number(2500.75));

    // 4. Delimiter auto-detection when commas appear inside EU decimals or US thousands
    let eu_tab_paste = CsvOptions::detect("1,5\t2,5*\n3,5\t4,5");
    assert_eq!(eu_tab_paste.delimiter, '\t');
    assert_eq!(eu_tab_paste.decimal_separator, ',');

    let eu_semi = CsvOptions::detect("1,5;2,5\r3,5;4,5");
    assert_eq!(eu_semi.delimiter, ';');
    assert_eq!(eu_semi.decimal_separator, ',');

    let us_semi = CsvOptions::detect("1,234.56;7,890.12\n2,345.67;8,901.23");
    assert_eq!(us_semi.delimiter, ';');
    assert_eq!(us_semi.decimal_separator, '.');
    assert_eq!(us_semi.thousands_separator, Some(','));

    let quoted_tab_csv = CsvOptions::detect("\"note\t1\",2,3\n\"note\t2\",4,5");
    assert_eq!(quoted_tab_csv.delimiter, ',');
    assert_eq!(quoted_tab_csv.decimal_separator, '.');

    let plain_int_csv = CsvOptions::detect("1,2\n3,4");
    assert_eq!(plain_int_csv.delimiter, ',');
}
