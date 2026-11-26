#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp, clippy::panic)]

use super::common;

use std::path::Path;

use common::build_zip_str_entries;
use curvebook_doc::{Cell, CellValue, SubcolumnMode, TableArchetype, TableKind};
use curvebook_format::{
    CsvOptions, FormatKind, FormatRegistry, XlsxImportOptions, decode_delimited_text_bytes, import_csv_to_datasheet, import_xlsx_bytes,
    import_xlsx_bytes_with_options, list_xlsx_sheets,
};

#[test]
fn xlsx_registry_multi_sheet_selection_sparse_cells_and_merged_headers() {
    let bytes = build_zip_str_entries(&[
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        ),
        (
            "xl/workbook.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="DoseResponse" sheetId="1" r:id="rId1"/><sheet name="HiddenQC" sheetId="2" state="hidden" r:id="rId2"/><sheet name="Summary" sheetId="3" r:id="rId3"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet3.xml"/></Relationships>"#,
        ),
        (
            "xl/sharedStrings.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="4" uniqueCount="4"><si><t>Dose</t></si><si><t>Treatment</t></si><si><r><t xml:space="preserve">Rep </t></r><r><t>1</t></r></si><si><t>Rep 2</t></si></sst>"#,
        ),
        (
            "xl/worksheets/sheet1.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row><row r="2"><c r="B2" t="s"><v>2</v></c><c r="C2" t="s"><v>3</v></c></row><row r="3"><c r="A3"><v>0.1</v></c><c r="B3"><v>12.5</v></c><c r="C3"><f>B3*2</f><v>25</v></c></row><row r="5"><c r="A5"><v>1.0</v></c><c r="B5" t="inlineStr"><is><t>Note</t></is></c><c r="D5" t="b"><v>1</v></c></row><row r="6"><c r="B6" t="str"><f>"OK"</f><v>OK</v></c><c r="C6" t="e"><v>#DIV/0!</v></c></row></sheetData><mergeCells count="1"><mergeCell ref="B1:C1"/></mergeCells></worksheet>"#,
        ),
        (
            "xl/worksheets/sheet2.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>QC</t></is></c></row><row r="2"><c r="A2"><v>99.5</v></c></row></sheetData></worksheet>"#,
        ),
        (
            "xl/worksheets/sheet3.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>X</t></is></c><c r="B1" t="inlineStr"><is><t>Y</t></is></c></row><row r="2"><c r="A2"><v>1</v></c><c r="B2"><v>42</v></c></row></sheetData></worksheet>"#,
        ),
    ]);

    let reg = FormatRegistry::default_registry();
    let importer = reg.importer_for_path(Path::new("plate_run.xlsx")).unwrap();
    assert_eq!((importer.descriptor().id, importer.descriptor().kind), ("xlsx", FormatKind::ForeignProject));

    let listed = list_xlsx_sheets(&bytes).unwrap();
    assert_eq!((listed.len(), listed[1].state.as_deref()), (3, Some("hidden")));

    let default_titles: Vec<String> = import_xlsx_bytes(&bytes).unwrap().document.data_sheets.into_iter().map(|s| s.title).collect();
    assert_eq!(default_titles, vec!["DoseResponse", "Summary"]);

    let multi = import_xlsx_bytes_with_options(
        &bytes,
        &XlsxImportOptions { sheet_names: Some(vec![String::from("DoseResponse"), String::from("HiddenQC")]), header_rows: 2, ..Default::default() },
    )
    .unwrap();
    assert_eq!(multi.document.data_sheets.len(), 2);
    assert!(multi.warnings.iter().any(|w| w.message.contains("formula cell(s)")));
    assert!(multi.warnings.iter().any(|w| w.message.contains("#DIV/0!")));

    let TableArchetype::MultipleVariables(mv) = &multi.document.data_sheets[0].archetype else { panic!("expected MultipleVariables") };
    assert_eq!(
        [mv.variables[0].title.as_str(), mv.variables[1].title.as_str(), mv.variables[2].title.as_str()],
        ["Dose", "Treatment - Rep 1", "Treatment - Rep 2"]
    );
    for (col, row, expected) in [
        (0, 0, Cell::number(0.1)),
        (2, 0, Cell::number(25.0)),
        (0, 1, Cell::empty()),
        (1, 2, Cell::text("Note")),
        (3, 2, Cell::text("TRUE")),
        (1, 3, Cell::text("OK")),
        (2, 3, Cell::text("#DIV/0!")),
    ] {
        assert_eq!(mv.variables[col].cells[row], expected, "col={col} row={row}");
    }

    let by_idx = import_xlsx_bytes_with_options(&bytes, &XlsxImportOptions { sheet_indices: Some(vec![2]), ..Default::default() }).unwrap();
    assert_eq!((by_idx.document.data_sheets.len(), by_idx.document.data_sheets[0].title.as_str()), (1, "Summary"));
}

#[test]
fn delimited_hardening_covers_utf16_boms_thousands_locales_multiline_headers_and_ragged_rows() {
    let encode_utf16 = |s: &str, le: bool| -> Vec<u8> {
        let mut buf = if le { vec![0xFF, 0xFE] } else { vec![0xFE, 0xFF] };
        for u in s.encode_utf16() {
            buf.extend_from_slice(&if le { u.to_le_bytes() } else { u.to_be_bytes() });
        }
        buf
    };

    let src = "Conc (µM)\tResp\n\"Line 1\nLine 2\"\t12,34\n10\n";
    let mut utf8_bom = vec![0xEF, 0xBB, 0xBF];
    utf8_bom.extend_from_slice(src.as_bytes());
    for (label, bytes) in [("utf8", utf8_bom), ("utf16le", encode_utf16(src, true)), ("utf16be", encode_utf16(src, false))] {
        assert_eq!(decode_delimited_text_bytes(&bytes, label).unwrap(), src, "{label}");
    }

    let messy_eu_tsv = "Group A\t\tGroup B\nRep 1\tRep 2\tRep 1\n1.234,56\t2.345,67\t3,5*\n\"row 2\ncontinuation\"\t4,25\n";
    let decoded_messy = decode_delimited_text_bytes(&encode_utf16(messy_eu_tsv, true), "tsv").unwrap();
    let eu_ds = import_csv_to_datasheet(
        "eu-tsv",
        "EU Assay",
        &decoded_messy,
        TableKind::Column,
        SubcolumnMode::Single,
        &CsvOptions { header_rows: 2, ..CsvOptions::detect_tsv(&decoded_messy) },
    )
    .unwrap();
    let TableArchetype::Column(col) = &eu_ds.archetype else { panic!("expected Column") };
    assert_eq!(col.data_sets.iter().map(|d| d.title.as_str()).collect::<Vec<_>>(), vec!["Group A - Rep 1", "Group A - Rep 2", "Group B - Rep 1"]);
    assert_eq!(
        [&col.data_sets[0].cells[0][0], &col.data_sets[1].cells[0][0], &col.data_sets[2].cells[0][0]],
        [&Cell::number(1234.56), &Cell::number(2345.67), &Cell { value: CellValue::Number(3.5), excluded: true }]
    );
    assert_eq!(
        [&col.data_sets[0].cells[1][0], &col.data_sets[1].cells[1][0], &col.data_sets[2].cells[1][0]],
        [&Cell::text("row 2\ncontinuation"), &Cell::number(4.25), &Cell::empty()]
    );

    let us_tsv = "Dose\tSignal\n1,234.56\t2,500.75\n";
    let TableArchetype::Xy(xy) =
        import_csv_to_datasheet("us-tsv", "US Thousands", us_tsv, TableKind::Xy, SubcolumnMode::Single, &CsvOptions::detect_tsv(us_tsv))
            .unwrap()
            .archetype
    else {
        panic!("expected XY");
    };
    assert_eq!((&xy.x_cells[0][0], &xy.data_sets[0].cells[0][0]), (&Cell::number(1234.56), &Cell::number(2500.75)));

    let detect_cases: [(&str, &str, char, char, Option<char>); 5] = [
        ("eu_tab", "1,5\t2,5*\n3,5\t4,5", '\t', ',', None),
        ("eu_semi", "1,5;2,5\r3,5;4,5", ';', ',', None),
        ("us_semi", "1,234.56;7,890.12\n2,345.67;8,901.23", ';', '.', Some(',')),
        ("quoted_tab_csv", "\"note\t1\",2,3\n\"note\t2\",4,5", ',', '.', None),
        ("plain_int_csv", "1,2\n3,4", ',', '.', None),
    ];
    for (label, text, delim, dec, thou) in detect_cases {
        let got = CsvOptions::detect(text);
        assert_eq!((got.delimiter, got.decimal_separator), (delim, dec), "{label}");
        if thou.is_some() {
            assert_eq!(got.thousands_separator, thou, "{label}");
        }
    }
}
