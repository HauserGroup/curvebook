//! `R17.6`: a formula cell saved without a stored result must be reported, not passed off as a
//! blank cell or as an imported cached value.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::xlsx_single_sheet_zip;
use curvebook_format::{ImportWarning, ImportWarningKind, import_xlsx_bytes};

fn workbook(rows: &str) -> Vec<u8> {
    let header = r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Dose</t></is></c><c r="B1" t="inlineStr"><is><t>Response</t></is></c></row>"#;
    xlsx_single_sheet_zip("Assay", &format!("{header}{rows}"))
}

fn of_kind(warnings: &[ImportWarning], kind: ImportWarningKind) -> Vec<&ImportWarning> {
    warnings.iter().filter(|w| w.kind == kind).collect()
}

#[test]
fn cached_and_uncached_formulas_are_reported_separately() {
    let rows = r#"
    <row r="2"><c r="A2"><v>1</v></c><c r="B2"><f>A2*10</f><v>10</v></c></row>
    <row r="3"><c r="A3"><v>2</v></c><c r="B3"><f>A3*10</f></c></row>
    <row r="4"><c r="A4"><v>3</v></c><c r="B4"><f t="shared" si="0"/></c></row>
    <row r="5"><c r="A5"><v>4</v></c><c r="B5"><f>A5*10</f><v>40</v></c></row>"#;
    let report = import_xlsx_bytes(&workbook(rows)).unwrap();

    assert!(
        of_kind(&report.warnings, ImportWarningKind::CoercedValue)
            .iter()
            .any(|w| w.message == "worksheet `Assay` contained 2 formula cell(s); imported their stored results"),
        "cached formula warning: {:?}",
        report.warnings
    );

    let missing = of_kind(&report.warnings, ImportWarningKind::MissingCachedValue);
    assert_eq!((missing.len(), missing[0].source.as_str()), (1, "Assay"));
    let msg = &missing[0].message;
    assert!(
        msg.contains("2 formula cell(s) without a stored result (first: B3, B4)")
            && msg.contains("imported as empty cells")
            && msg.contains("recalculate"),
        "{msg}"
    );
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
