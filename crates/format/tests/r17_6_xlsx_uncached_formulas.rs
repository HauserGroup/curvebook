//! `R17.6`: a formula cell saved without a stored result must be reported, not passed off as a
//! blank cell or as an imported cached value.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_format::zip::ZipWriter;
use curvebook_format::{ImportWarning, ImportWarningKind, import_xlsx_bytes};

const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;
const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Assay" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#;
const WB_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;

/// Workbook whose single worksheet has a header row and the given `<row>` elements.
fn workbook(rows: &str) -> Vec<u8> {
    let sheet = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>Dose</t></is></c><c r="B1" t="inlineStr"><is><t>Response</t></is></c></row>
    {rows}
  </sheetData>
</worksheet>"#
    );
    let mut zw = ZipWriter::new();
    for (name, xml) in [("_rels/.rels", RELS), ("xl/workbook.xml", WORKBOOK), ("xl/_rels/workbook.xml.rels", WB_RELS)] {
        zw.add_file(name, xml.as_bytes().to_vec()).unwrap();
    }
    zw.add_file("xl/worksheets/sheet1.xml", sheet.into_bytes()).unwrap();
    zw.finish().unwrap()
}

fn of_kind(warnings: &[ImportWarning], kind: ImportWarningKind) -> Vec<&ImportWarning> {
    warnings.iter().filter(|w| w.kind == kind).collect()
}

#[test]
fn cached_and_uncached_formulas_are_reported_separately() {
    // B2 and B5 hold stored formula results; B3 (plain formula) and B4 (shared formula, the form
    // scripts and spreadsheet applications write for filled-down formulas) have none.
    let rows = r#"
    <row r="2"><c r="A2"><v>1</v></c><c r="B2"><f>A2*10</f><v>10</v></c></row>
    <row r="3"><c r="A3"><v>2</v></c><c r="B3"><f>A3*10</f></c></row>
    <row r="4"><c r="A4"><v>3</v></c><c r="B4"><f t="shared" si="0"/></c></row>
    <row r="5"><c r="A5"><v>4</v></c><c r="B5"><f>A5*10</f><v>40</v></c></row>"#;
    let report = import_xlsx_bytes(&workbook(rows)).unwrap();

    let cached = of_kind(&report.warnings, ImportWarningKind::CoercedValue);
    assert!(
        cached.iter().any(|w| w.message == "worksheet `Assay` contained 2 formula cell(s); imported their stored results"),
        "cached formula warning: {:?}",
        report.warnings
    );

    let missing = of_kind(&report.warnings, ImportWarningKind::MissingCachedValue);
    assert_eq!(missing.len(), 1, "{:?}", report.warnings);
    let msg = &missing[0].message;
    assert_eq!(missing[0].source, "Assay");
    assert!(msg.contains("2 formula cell(s) without a stored result (first: B3, B4)"), "{msg}");
    assert!(msg.contains("imported as empty cells") && msg.contains("recalculate"), "{msg}");
}

#[test]
fn all_cached_formulas_produce_no_missing_value_warning() {
    let rows = r#"<row r="2"><c r="A2"><v>1</v></c><c r="B2"><f>A2*10</f><v>10</v></c></row>"#;
    let report = import_xlsx_bytes(&workbook(rows)).unwrap();
    assert!(of_kind(&report.warnings, ImportWarningKind::MissingCachedValue).is_empty(), "{:?}", report.warnings);
}

#[test]
fn many_uncached_formulas_list_a_bounded_set_of_examples() {
    let rows: String = (2..=9).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c><c r="B{r}"><f>A{r}*10</f></c></row>"#)).collect();
    let report = import_xlsx_bytes(&workbook(&rows)).unwrap();
    assert!(of_kind(&report.warnings, ImportWarningKind::CoercedValue).iter().all(|w| !w.message.contains("formula")), "{:?}", report.warnings);
    let missing = of_kind(&report.warnings, ImportWarningKind::MissingCachedValue);
    assert_eq!(missing.len(), 1);
    assert!(missing[0].message.contains("8 formula cell(s) without a stored result (first: B2, B3, B4, B5, B6, ...)"), "{}", missing[0].message);
}
