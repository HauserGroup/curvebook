//! Targeted regression tests for AUD-01 (`AUD-01-01`, `02`, `05`, `06`, `07`, `08`, `09`, `11`,
//! `12`, `14`, `15`, `17`, `18`, `20`) and format-half AUD-13 (`AUD-13-08`, `19`, `24`) findings.

use std::fs;
use std::time::Instant;

use curvebook_doc::{
    AnalysisKind, BaselineOperation, Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, CurveSamplePoint, DataSheet, DemingErrorSpec,
    DocError, FittedCurveOverlay, FloatingNote, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoConstant, InfoSheet, LinearRegressionSpec,
    LinkSource, MAX_TITLE_BYTES, NoteId, PowerAnalysisSpec, RectPt, ResultStatus, SheetId, SourceCellId, SubcolumnMode, TableArchetype, TableKind,
    XColumnMode, XyTable,
};
use curvebook_format::recovery::{CrashRecoveryJournal, JournalBackend, JournalOpRecord, MemoryJournalBackend};
use curvebook_format::xml::decode_xml_entities;
use curvebook_format::{
    CsvOptions, FormatError, ImportBaselineAdjustment, XlsxImportOptions, ZipArchive, apply_csv_import_link, export_datasheet_to_csv,
    export_result_sheet_json, export_result_sheet_tidy_csv, filter_delimited_text, import_csv_to_datasheet, import_csv_to_datasheet_with_x_mode,
    import_pzfx_str, import_xlsx_bytes, import_xlsx_bytes_with_options, open_project_bytes, parse_delimited_records, parse_xml,
    rearrange_and_baseline_grid, save_project_bytes, save_project_unpacked_dir,
};
use serde_json::{Value, json};

use crate::common::{
    RawZipEntry, TempDirGuard, build_raw_zip, build_zip_str_entries, col_sheet, make_result_sheet, pzfx_tables, sample_column_project,
    xlsx_single_sheet_zip,
};

fn xlsx_with_parts(shared_strings_xml: Option<&str>, worksheet_xml: &str) -> Vec<u8> {
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let wb = r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let wb_rels = if shared_strings_xml.is_some() {
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#
    } else {
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#
    };
    let mut entries =
        vec![("_rels/.rels", rels), ("xl/workbook.xml", wb), ("xl/_rels/workbook.xml.rels", wb_rels), ("xl/worksheets/sheet1.xml", worksheet_xml)];
    if let Some(sst) = shared_strings_xml {
        entries.push(("xl/sharedStrings.xml", sst));
    }
    build_zip_str_entries(&entries)
}

#[test]
fn aud_01_01_xlsx_bounds_shared_string_and_merged_header_text_amplification() {
    // 1. Shared-string amplification: one 1 MiB shared string referenced 80 times exceeds 64 MiB text budget.
    let big = "A".repeat(1_048_576);
    let sst = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>{big}</t></si></sst>"#
    );
    let mut rows = String::new();
    for r in 1..=80 {
        rows.push_str(&format!(r#"<row r="{r}"><c r="A{r}" t="s"><v>0</v></c></row>"#));
    }
    let ws = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{rows}</sheetData></worksheet>"#
    );
    let bytes = xlsx_with_parts(Some(&sst), &ws);
    assert!(matches!(import_xlsx_bytes(&bytes), Err(FormatError::ZipBomb { .. })));

    // 2. Merged-header amplification: a 100 KiB inline header merged across 20,000 cols (20,000 * 4 KiB > 64 MiB)
    //    must be bounded by MAX_TITLE_BYTES per slot and rejected by MAX_IMPORT_TEXT_BYTES.
    let big_hdr = "H".repeat(100_000);
    // Column "ACYJ" -> 20,000th column (within max_cols = 524,292 and total_grid_cells = 20,000).
    let merge_ws = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>{big_hdr}</t></is></c><c r="ACYJ1"><v>1</v></c></row></sheetData><mergeCells count="1"><mergeCell ref="A1:ACYJ1"/></mergeCells></worksheet>"#
    );
    let merge_bytes = xlsx_with_parts(None, &merge_ws);
    assert!(matches!(import_xlsx_bytes(&merge_bytes), Err(FormatError::ZipBomb { .. })));

    // 3. Normal wide header text merged over 3 columns clamps safely to MAX_TITLE_BYTES and imports cleanly.
    let ok_hdr = "B".repeat(MAX_TITLE_BYTES + 500);
    let ok_ws = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>{ok_hdr}</t></is></c></row><row r="2"><c r="A2"><v>1</v></c><c r="B2"><v>2</v></c><c r="C2"><v>3</v></c></row></sheetData><mergeCells count="1"><mergeCell ref="A1:C1"/></mergeCells></worksheet>"#
    );
    let ok_bytes = xlsx_with_parts(None, &ok_ws);
    let report = import_xlsx_bytes(&ok_bytes).unwrap();
    let TableArchetype::Xy(ref xy) = report.document.data_sheets[0].archetype else { panic!("expected Xy") };
    assert!(xy.x_title.len() <= MAX_TITLE_BYTES);
    assert!(xy.data_sets[0].title.len() <= MAX_TITLE_BYTES);
}

#[test]
fn aud_01_02_xlsx_deduplicates_and_bounds_overlapping_merge_cells() {
    // 1. 10,000 duplicate mergeCell tags complete rapidly without quadratic hang.
    let dup_merges = r#"<mergeCell ref="A1:D1"/>"#.repeat(10_000);
    let dup_ws = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>H</t></is></c><c r="D1"><v>1</v></c></row><row r="2"><c r="A2"><v>10</v></c><c r="B2"><v>20</v></c><c r="C2"><v>30</v></c><c r="D2"><v>40</v></c></row></sheetData><mergeCells>{dup_merges}</mergeCells></worksheet>"#
    );
    let dup_bytes = xlsx_with_parts(None, &dup_ws);
    let start = Instant::now();
    let report = import_xlsx_bytes(&dup_bytes).unwrap();
    assert!(start.elapsed().as_secs() < 2, "duplicate mergeCells took too long: {:?}", start.elapsed());
    assert_eq!(report.document.data_sheets.len(), 1);

    // 2. Distinct overlapping header mergeCell ranges exceeding header grid area are rejected with ZipBomb.
    let overlap_ws = r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>H</t></is></c><c r="Z1"><v>1</v></c></row></sheetData><mergeCells><mergeCell ref="A1:Z1"/><mergeCell ref="A1:Y1"/></mergeCells></worksheet>"#;
    let overlap_bytes = xlsx_with_parts(None, overlap_ws);
    assert!(matches!(import_xlsx_bytes(&overlap_bytes), Err(FormatError::ZipBomb { .. })));
}

fn excel_col_name(mut col_one_based: usize) -> String {
    let mut chars = Vec::new();
    while col_one_based > 0 {
        let rem = (col_one_based - 1) % 26;
        chars.push((b'A' + rem as u8) as char);
        col_one_based = (col_one_based - 1) / 26;
    }
    chars.into_iter().rev().collect()
}

#[test]
fn aud_01_01_and_02_xlsx_bounds_multisheet_shared_strings_and_whitespace_merges() {
    // 1. Multi-sheet shared-string amplification: 100 sheets each referencing one 1 MiB shared string
    //    pass per-sheet limits individually (1 MiB <= 64 MiB) but exceed the 64 MiB workbook-wide budget.
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let mut sheet_tags = String::new();
    for i in 1..=100 {
        sheet_tags.push_str(&format!(r#"<sheet name="S{i}" sheetId="{i}" r:id="rId1"/>"#));
    }
    let wb = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>{sheet_tags}</sheets></workbook>"#
    );
    let wb_rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#;
    let big = "S".repeat(1_048_576);
    let sst = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>{big}</t></si></sst>"#
    );
    let tiny_ws = r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c></row></sheetData></worksheet>"#;
    let multi_bytes = build_zip_str_entries(&[
        ("_rels/.rels", rels),
        ("xl/workbook.xml", &wb),
        ("xl/_rels/workbook.xml.rels", wb_rels),
        ("xl/sharedStrings.xml", &sst),
        ("xl/worksheets/sheet1.xml", tiny_ws),
    ]);
    assert!(matches!(import_xlsx_bytes(&multi_bytes), Err(FormatError::ZipBomb { .. })));

    // 2. Large whitespace anchor at A1 with 1,000 distinct overlapping mergeCell ranges must fail fast
    //    without cloning the full whitespace string per merge or bypassing merge_cells_visited.
    let spaces = " ".repeat(2_097_152);
    let mut merges = String::new();
    for c in 2..=1_001 {
        let end_col = excel_col_name(c);
        merges.push_str(&format!(r#"<mergeCell ref="A1:{end_col}1"/>"#));
    }
    let last_col = excel_col_name(1_001);
    let ws_spaces = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>{spaces}</t></is></c><c r="{last_col}1"><v>1</v></c></row></sheetData><mergeCells>{merges}</mergeCells></worksheet>"#
    );
    let spaces_bytes = xlsx_with_parts(None, &ws_spaces);
    let start = Instant::now();
    assert!(matches!(import_xlsx_bytes(&spaces_bytes), Err(FormatError::ZipBomb { .. })));
    assert!(start.elapsed().as_secs() < 2, "whitespace-anchor overlapping merges took too long: {:?}", start.elapsed());
}

#[test]
fn aud_01_05_xlsx_rejects_truncated_shared_strings_and_worksheet_xml() {
    // Truncated worksheet XML mid-row/cell must fail instead of silently dropping trailing rows.
    let trunc_ws = r#"<?xml version="1.0" encoding="UTF-8"?><worksheet><sheetData><row r="1"><c r="A1"><v>1</v></c></row><row r="2"><c r="A2"><v>2"#;
    assert!(matches!(import_xlsx_bytes(&xlsx_with_parts(None, trunc_ws)), Err(FormatError::XmlParse { .. })));

    // Unclosed <worksheet> after a complete row must also fail.
    let unclosed_ws = r#"<?xml version="1.0" encoding="UTF-8"?><worksheet><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData>"#;
    assert!(matches!(import_xlsx_bytes(&xlsx_with_parts(None, unclosed_ws)), Err(FormatError::XmlParse { .. })));

    // Truncated sharedStrings.xml mid-<si> must fail.
    let ok_ws =
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c></row></sheetData></worksheet>"#;
    let trunc_sst = r#"<?xml version="1.0" encoding="UTF-8"?><sst><si><t>Hello</t></si><si><t>Trunc"#;
    assert!(matches!(import_xlsx_bytes(&xlsx_with_parts(Some(trunc_sst), ok_ws)), Err(FormatError::XmlParse { .. })));

    // Empty worksheet XML must fail, while self-closing <worksheet/> and <sst/> succeed.
    assert!(matches!(import_xlsx_bytes(&xlsx_with_parts(None, "")), Err(FormatError::XmlParse { .. })));
    let empty_ok = xlsx_with_parts(Some(r#"<?xml version="1.0"?><sst/>"#), r#"<?xml version="1.0"?><worksheet/>"#);
    assert!(import_xlsx_bytes(&empty_ok).is_ok());
}

#[test]
fn aud_01_06_apply_baseline_clears_zero_missing_and_overflow_to_empty() {
    // Zero baseline in col 0 under PercentOfFirstRow must clear col 0 to Empty, never keep raw [0.0, 5.0].
    let grid = vec![vec![Cell::number(0.0), Cell::number(10.0)], vec![Cell::number(5.0), Cell::number(20.0)]];
    let pct = rearrange_and_baseline_grid(&grid, &ImportPlacementSpec::default(), ImportBaselineAdjustment::PercentOfFirstRow);
    assert_eq!(pct[0][0], Cell::empty());
    assert_eq!(pct[1][0], Cell::empty());
    assert_eq!(pct[0][1], Cell::number(100.0));
    assert_eq!(pct[1][1], Cell::number(200.0));

    // Overflow in PercentOfFirstRow clears to Empty.
    let ovf_pct = rearrange_and_baseline_grid(
        &[vec![Cell::number(1e-308)], vec![Cell::number(1e5)]],
        &ImportPlacementSpec::default(),
        ImportBaselineAdjustment::PercentOfFirstRow,
    );
    assert_eq!(ovf_pct[1][0], Cell::empty());

    // Missing/excluded baseline and subtraction overflow in SubtractFirstRow clear to Empty.
    let sub_row = rearrange_and_baseline_grid(
        &[vec![Cell::excluded_number(10.0), Cell::number(-f64::MAX)], vec![Cell::number(42.0), Cell::number(f64::MAX)]],
        &ImportPlacementSpec::default(),
        ImportBaselineAdjustment::SubtractFirstRow,
    );
    assert_eq!(sub_row[1][0], Cell::empty());
    assert_eq!(sub_row[1][1], Cell::empty());

    // Missing/excluded baseline cell in SubtractColumn clears numeric cells in that row to Empty.
    let sub_col = rearrange_and_baseline_grid(
        &[vec![Cell::empty(), Cell::number(99.0)], vec![Cell::number(10.0), Cell::number(25.0)]],
        &ImportPlacementSpec::default(),
        ImportBaselineAdjustment::SubtractColumn(0),
    );
    assert_eq!(sub_col[0][1], Cell::empty());
    assert_eq!(sub_col[1][0], Cell::number(0.0));
    assert_eq!(sub_col[1][1], Cell::number(15.0));
}

#[test]
fn aud_01_07_unpacked_tx_reconcile_rejects_arbitrary_non_stage_paths() {
    let dir = TempDirGuard::new("aud-01-07-unpacked").unwrap();
    let root = dir.child("study.curvebook.d");
    let doc = sample_column_project("Unpacked Security");
    save_project_unpacked_dir(&doc, &root).unwrap();

    // Seed .git/config and user files inside the unpacked directory.
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(root.join(".git/config"), b"[core]\n\trepositoryformatversion = 0\n").unwrap();
    fs::write(root.join("README.md"), b"# My Lab Notes\n").unwrap();
    fs::write(root.join("analysis.py"), b"print('keep me')\n").unwrap();

    // Craft malicious .curvebook.tx/COMMITTING listing non-stage files.
    let tx_dir = root.join(".curvebook.tx");
    fs::create_dir_all(&tx_dir).unwrap();
    let bad_marker = json!({
        "format": "curvebook-unpacked-tx",
        "version": 1,
        "stage_paths": [".git/config", "README.md", "analysis.py"],
        "prior_csv_paths": [],
        "new_csv_paths": []
    });
    fs::write(tx_dir.join("COMMITTING"), serde_json::to_vec(&bad_marker).unwrap()).unwrap();

    let err = save_project_unpacked_dir(&doc, &root).unwrap_err();
    assert!(matches!(err, FormatError::UnsafePath { .. }), "expected UnsafePath, got {err:?}");
    assert!(root.join(".git/config").exists(), ".git/config must not be deleted");
    assert!(root.join("README.md").exists(), "README.md must not be deleted");
    assert!(root.join("analysis.py").exists(), "analysis.py must not be deleted");

    // Even with a syntactically valid stage filename, stage cleanup must not run before snapshot validation.
    let dummy_stage = root.join(".document.json.stage.1234.9");
    fs::write(&dummy_stage, b"staged").unwrap();
    let missing_snap_marker = json!({
        "format": "curvebook-unpacked-tx",
        "version": 1,
        "stage_paths": [".document.json.stage.1234.9"],
        "prior_csv_paths": [],
        "new_csv_paths": []
    });
    fs::write(tx_dir.join("COMMITTING"), serde_json::to_vec(&missing_snap_marker).unwrap()).unwrap();
    let err2 = save_project_unpacked_dir(&doc, &root).unwrap_err();
    assert!(matches!(err2, FormatError::MissingArchiveEntry { .. }), "expected MissingArchiveEntry, got {err2:?}");
    assert!(dummy_stage.exists(), "stage file must not be deleted before snapshot recovery succeeds");
}

#[test]
fn aud_01_08_xlsx_rejects_max_row_and_col_indices_without_overflow_panic() {
    let cases = [
        r#"<row r="18446744073709551615"/><row><c><v>1</v></c></row>"#,
        r#"<row r="4294967295"/><row><c><v>1</v></c></row>"#,
        r#"<row r="1"><c r="GKGWBYLWRXTLPO1"/><c><v>1</v></c></row>"#,
        r#"<row r="1"><c r="MWLQKWU1"/><c><v>1</v></c></row>"#,
    ];
    for rows_xml in cases {
        let bytes = xlsx_single_sheet_zip("Overflow", rows_xml);
        let err = import_xlsx_bytes(&bytes).unwrap_err();
        match err {
            FormatError::Doc(DocError::TooManyRows { count, .. }) | FormatError::Doc(DocError::TooManyDataSets { count, .. }) => {
                assert!(count > 0, "index wrapped around to 0 for `{rows_xml}`");
            }
            other => panic!("expected TooManyRows/TooManyDataSets for `{rows_xml}`, got {other:?}"),
        }
    }
}

#[test]
fn aud_01_09_save_project_bytes_matches_open_validation_for_all_sheet_types() {
    // (a) DataSheet with negative FloatingNote width.
    let mut doc_a = sample_column_project("Bad Data Note");
    doc_a.data_sheets[0].floating_notes.push(FloatingNote {
        id: NoteId::new("n1"),
        fill_color: FloatingNote::DEFAULT_FILL,
        bounds_pt: RectPt { x: 0.0, y: 0.0, width: -10.0, height: 20.0 },
        minimized: false,
        body_markdown: String::from("bad"),
    });
    assert!(save_project_bytes(&doc_a).is_err(), "save must reject negative DataSheet FloatingNote width");

    // (b) InfoSheet with whitespace-only InfoConstant.name.
    let mut doc_b = sample_column_project("Bad Info Const");
    let mut info_b = InfoSheet::new_preseeded("info-1", "Info").unwrap();
    info_b.constants.push(InfoConstant { name: String::from("   "), value: curvebook_doc::ConstantValue::Empty, units: None });
    doc_b.info_sheets.push(info_b);
    assert!(save_project_bytes(&doc_b).is_err(), "save must reject whitespace InfoConstant.name");

    // (c) InfoSheet with empty FloatingNote.id.
    let mut doc_c = sample_column_project("Bad Info Note");
    let mut info_c = InfoSheet::new_preseeded("info-1", "Info").unwrap();
    info_c.floating_notes.push(FloatingNote::new("  ", RectPt::new(0.0, 0.0, 50.0, 50.0), "note"));
    doc_c.info_sheets.push(info_c);
    assert!(save_project_bytes(&doc_c).is_err(), "save must reject empty InfoSheet FloatingNote.id");

    // (d) ResultSheet with out-of-domain PowerAnalysisSpec (alpha = 1.5).
    let mut doc_d = sample_column_project("Bad Power Spec");
    let mut rs_d = make_result_sheet("res-1", "data-1", ResultStatus::UpToDate);
    let bad_power = PowerAnalysisSpec { alpha: 1.5, ..Default::default() };
    rs_d.analysis.kind = AnalysisKind::PowerAnalysis(bad_power);
    doc_d.result_sheets.push(rs_d);
    assert!(save_project_bytes(&doc_d).is_err(), "save must reject invalid PowerAnalysisSpec domain");

    // (e) ResultSheet with non-finite LinearRegressionSpec fixed_point (AUD-01-04 save/reopen guard).
    let mut doc_e = sample_column_project("Bad Linear FixedPoint");
    let mut rs_e = make_result_sheet("res-1", "data-1", ResultStatus::UpToDate);
    rs_e.analysis.kind = AnalysisKind::LinearRegression(LinearRegressionSpec {
        fixed_point: Some((f64::NAN, 0.0)),
        deming_error: DemingErrorSpec::VarianceRatio { lambda: 1.0 },
        ..LinearRegressionSpec::default()
    });
    doc_e.result_sheets.push(rs_e);
    assert!(save_project_bytes(&doc_e).is_err(), "save must reject non-finite LinearRegressionSpec::fixed_point");

    // Valid project round-trips cleanly.
    let valid_bytes = save_project_bytes(&sample_column_project("Valid")).unwrap();
    assert!(open_project_bytes(&valid_bytes).is_ok());
}

#[test]
fn aud_01_11_xlsx_default_single_subcolumn_normalizes_for_grouped_and_nested() {
    let rows = r#"
        <row r="1"><c r="A1" t="inlineStr"><is><t>Ctrl</t></is></c><c r="B1" t="inlineStr"><is><t>Drug</t></is></c></row>
        <row r="2"><c r="A2"><v>10</v></c><c r="B2"><v>20</v></c></row>
    "#;
    let bytes = xlsx_single_sheet_zip("Sheet1", rows);

    let grp_report = import_xlsx_bytes_with_options(&bytes, &XlsxImportOptions { table_kind: Some(TableKind::Grouped), ..Default::default() })
        .expect("Grouped import with default Single subcolumn_mode must succeed");
    let TableArchetype::Grouped(ref grp) = grp_report.document.data_sheets[0].archetype else { panic!("expected Grouped") };
    assert_eq!(grp.subcolumn_mode, SubcolumnMode::RawReplicates(1));
    assert_eq!(grp.data_sets.len(), 2);

    let nested_report = import_xlsx_bytes_with_options(&bytes, &XlsxImportOptions { table_kind: Some(TableKind::Nested), ..Default::default() })
        .expect("Nested import with default Single subcolumn_mode must succeed");
    let TableArchetype::Nested(ref nested) = nested_report.document.data_sheets[0].archetype else { panic!("expected Nested") };
    assert_eq!(nested.groups.len(), 2);
    assert_eq!(nested.groups[0].biological_unit_count(), 1);

    // Genuinely incompatible modes are still rejected.
    let bad = import_xlsx_bytes_with_options(
        &bytes,
        &XlsxImportOptions { table_kind: Some(TableKind::Nested), subcolumn_mode: SubcolumnMode::MeanSdN, ..Default::default() },
    );
    assert!(matches!(bad, Err(FormatError::Doc(DocError::IncompatibleSubcolumnMode { archetype: TableKind::Nested, .. }))));
}

#[test]
fn aud_01_12_filter_delimited_text_and_link_honor_baseline_thousands_and_reject_predicate() {
    // 1. filter.baseline executes Difference and Ratio (clearing div-by-zero to Empty).
    let diff_filter = ImportFilterSpec { baseline: Some((0, BaselineOperation::Difference)), ..Default::default() };
    let diff = filter_delimited_text("Base,Resp\n10,25\n20,50\n", ',', '.', true, &diff_filter).unwrap();
    assert_eq!(diff.cells, vec![vec![Cell::number(0.0), Cell::number(15.0)], vec![Cell::number(0.0), Cell::number(30.0)]]);

    let ratio_filter = ImportFilterSpec { baseline: Some((0, BaselineOperation::Ratio)), ..Default::default() };
    let ratio = filter_delimited_text("Base,Resp\n0,25\n5,20\n", ',', '.', true, &ratio_filter).unwrap();
    assert_eq!(ratio.cells[0], vec![Cell::empty(), Cell::empty()]);
    assert_eq!(ratio.cells[1], vec![Cell::number(1.0), Cell::number(4.0)]);

    // 2. Thousands grouping inferred cleanly in filter_delimited_text.
    let tsv = filter_delimited_text("Dose\tSignal\n1,234.56\t2,500.00\n", '\t', '.', true, &ImportFilterSpec::default()).unwrap();
    assert_eq!(tsv.cells, vec![vec![Cell::number(1234.56), Cell::number(2500.0)]]);

    // 3. exclude_row_predicate is explicitly rejected rather than silently ignored and persisted.
    let pred_filter = ImportFilterSpec { exclude_row_predicate: Some(String::from("B > 10")), ..Default::default() };
    assert!(matches!(filter_delimited_text("A,B\n1,20\n", ',', '.', true, &pred_filter), Err(FormatError::InvalidPipeline { .. })));

    let mut target = col_sheet("c1", 2, 2);
    let link_spec = ImportLinkSpec {
        source: LinkSource::ExternalDelimitedFile { path: String::from("in.csv"), delimiter: ',', decimal_separator: '.' },
        filter: pred_filter,
        placement: ImportPlacementSpec::default(),
    };
    assert!(matches!(
        apply_csv_import_link(&mut target, "A,B\n1,20\n", &link_spec, true, ImportBaselineAdjustment::None, "lnk-1", "test"),
        Err(FormatError::InvalidPipeline { .. })
    ));
    assert!(target.linked_regions.is_empty(), "rejected link must not be persisted");
}

#[test]
fn aud_01_14_recovery_journal_rejects_seq_overflow_before_appending() {
    let mut backend = MemoryJournalBackend::new();
    let rec = JournalOpRecord::new(u64::MAX - 1, json!({"command_id": "noop", "params": {}})).unwrap();
    backend.append_journal_line(&rec.to_ndjson_line().unwrap()).unwrap();

    let (mut journal, report) = CrashRecoveryJournal::open_or_recover(backend, "owner-1", 60_000, 1_000).unwrap();
    assert_eq!(report.recovered_seqs, vec![u64::MAX - 1]);

    // First command reaches u64::MAX cleanly.
    let seq_max = journal.record_command(json!({"command_id": "last", "params": {}})).unwrap();
    assert_eq!(seq_max, u64::MAX);

    // Second command must fail BEFORE appending a duplicate seq = u64::MAX line.
    let before_bytes = journal.backend().read_journal_bytes().unwrap();
    let err = journal.record_command(json!({"command_id": "overflow", "params": {}})).unwrap_err();
    assert!(matches!(err, FormatError::Io(ref msg) if msg.contains("overflow")), "got {err:?}");
    let after_bytes = journal.backend().read_journal_bytes().unwrap();
    assert_eq!(before_bytes, after_bytes, "overflow error must not append duplicate u64::MAX record");

    // Re-opening from the journal ending at u64::MAX must recover cleanly and still refuse new appends.
    let abandoned = journal.abandon();
    let (mut reopened, report2) = CrashRecoveryJournal::open_or_recover(abandoned, "owner-2", 60_000, 100_000).unwrap();
    assert_eq!(report2.recovered_seqs, vec![u64::MAX - 1, u64::MAX]);
    let before_reopen_bytes = reopened.backend().read_journal_bytes().unwrap();
    assert!(reopened.record_command(json!({"command_id": "overflow2", "params": {}})).is_err());
    assert_eq!(before_reopen_bytes, reopened.backend().read_journal_bytes().unwrap());
}

#[test]
fn aud_01_15_csv_active_quoted_asterisk_text_round_trips_without_collision() {
    let cells = vec![
        Cell::text("\"hello\"*"),
        Cell::text("\"\"*"),
        Cell::text("[excluded]:literal"),
        Cell::text("[excluded]\\:literal"),
        Cell::excluded_text("\"hello\"*"),
        Cell::excluded_text("[excluded]:nested"),
        Cell::excluded_text("a,b"),
        Cell::excluded_text("NA"),
    ];
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, cells.len()).unwrap();
    col.data_sets[0].title = String::from("Labels");
    col.data_sets[0].cells = cells.iter().map(|c| vec![c.clone()]).collect();
    let sheet = DataSheet::new("s-quote", "Quote Collision", TableArchetype::Column(col)).unwrap();

    let opts = CsvOptions::csv();
    let exported = export_datasheet_to_csv(&sheet, &opts).unwrap();
    parse_delimited_records(&exported, &opts).expect("exported CSV must be valid RFC-4180");

    let re = import_csv_to_datasheet("s-quote-re", "Re", &exported, TableKind::Column, SubcolumnMode::Single, &opts).unwrap();
    let TableArchetype::Column(ref re_col) = re.archetype else { panic!("expected Column") };
    let actual: Vec<Cell> = re_col.data_sets[0].cells.iter().map(|r| r[0].clone()).collect();
    assert_eq!(actual, cells);

    // External CSV containing quoted """label""*" must import as active text `"label"*`.
    let external = import_csv_to_datasheet("ext", "Ext", "Col\n\"\"\"label\"\"*\"\n", TableKind::Column, SubcolumnMode::Single, &opts).unwrap();
    assert_eq!(external.cell(CellCoord::new(0, 0, 0)), Some(&Cell::text("\"label\"*")));
}

#[test]
fn aud_01_17_csv_detect_stops_early_on_large_input() {
    // 1. Tall input (~1.5 MiB, 100,000 rows).
    let mut tall = String::from("X;Y\n");
    for i in 0..100_000 {
        tall.push_str(&format!("{i},5;{},25\n", i + 1));
    }
    let start = Instant::now();
    let det = CsvOptions::detect(&tall);
    assert_eq!((det.delimiter, det.decimal_separator), (';', ','));
    assert!(start.elapsed().as_millis() < 500, "CsvOptions::detect took {:?}", start.elapsed());

    // 2. Pathological wide single row (50,000 columns on row 0).
    let wide = (0..50_000).map(|i| format!("c{i}")).collect::<Vec<_>>().join(",");
    let start_w = Instant::now();
    let det_w = CsvOptions::detect(&wide);
    assert_eq!(det_w.delimiter, ',');
    assert!(start_w.elapsed().as_millis() < 500, "wide CsvOptions::detect took {:?}", start_w.elapsed());
}

#[test]
fn aud_01_18_zip_locate_eocd_skips_false_signature_in_archive_comment() {
    // Construct a 22-byte false EOCD (disk = 1) embedded inside a valid ZIP's trailing comment.
    let mut fake_eocd = Vec::with_capacity(32);
    fake_eocd.extend_from_slice(b"note:");
    fake_eocd.extend_from_slice(&0x0605_4b50_u32.to_le_bytes()); // EOCD_SIG
    fake_eocd.extend_from_slice(&1_u16.to_le_bytes()); // disk = 1 (invalid multi-disk)
    fake_eocd.extend_from_slice(&0_u16.to_le_bytes());
    fake_eocd.extend_from_slice(&1_u16.to_le_bytes());
    fake_eocd.extend_from_slice(&1_u16.to_le_bytes());
    fake_eocd.extend_from_slice(&0_u32.to_le_bytes());
    fake_eocd.extend_from_slice(&0_u32.to_le_bytes());
    fake_eocd.extend_from_slice(&0_u16.to_le_bytes()); // comment_len = 0 -> matches EOF!

    let zip = build_raw_zip(&[RawZipEntry::stored(b"hello.txt", b"world")], &fake_eocd);
    let archive = ZipArchive::from_bytes(&zip).expect("must skip false EOCD in comment and find genuine EOCD");
    assert_eq!(archive.get("hello.txt"), Some(b"world".as_slice()));

    // A ZIP with ONLY a multi-disk EOCD (no valid EOCD earlier) still fails with InvalidZip.
    let mut only_bad = vec![0_u8; 22];
    only_bad[0..4].copy_from_slice(&0x0605_4b50_u32.to_le_bytes());
    only_bad[4..6].copy_from_slice(&2_u16.to_le_bytes());
    assert!(matches!(ZipArchive::from_bytes(&only_bad), Err(FormatError::InvalidZip { .. })));
}

#[test]
fn aud_01_20_xml_entities_reject_nul_and_forbidden_c0_controls() {
    for bad in ["&#0;", "&#x0;", "&#x00;", "&#1;", "&#8;", "&#11;", "&#x0C;", "&#31;", "&#xFFFE;", "&#xFFFF;"] {
        assert!(matches!(decode_xml_entities(bad, 0), Err(FormatError::XmlParse { .. })), "decode_xml_entities must reject `{bad}`");
        let xml = format!("<Root>{bad}</Root>");
        assert!(matches!(parse_xml(&xml), Err(FormatError::XmlParse { .. })), "parse_xml must reject `{bad}`");
    }

    // PZFX and XLSX importers reject NUL entities in data cells.
    let bad_pzfx =
        pzfx_tables("<Table TableType=\"OneWay\"><Title>T</Title><YColumn><Title>A</Title><Subcolumn><d>Dose&#0;1</d></Subcolumn></YColumn></Table>");
    assert!(matches!(import_pzfx_str(&bad_pzfx), Err(FormatError::XmlParse { .. })));

    let bad_xlsx = xlsx_single_sheet_zip("Nul", r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Sample&#x0;A</t></is></c></row>"#);
    assert!(matches!(import_xlsx_bytes(&bad_xlsx), Err(FormatError::XmlParse { .. })));

    // Valid XML 1.0 whitespace references remain accepted.
    let ok = parse_xml("<Root>a&#9;b&#10;c&#13;d&#32;e</Root>").unwrap();
    assert_eq!(ok.plain_text(), "a\tb\nc\rd e");
}

#[test]
fn aud_13_08_multi_subcolumn_x_roundtrip_csv_and_xlsx() {
    // Part 1a: Multi-subcolumn X (MeanSdN) export + CSV re-import preserves X subcolumns without shifting Y.
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN }, SubcolumnMode::MeanSdN, 2, 2).unwrap();
    xy.x_title = String::from("Dose");
    xy.data_sets[0].title = String::from("DrugA");
    xy.data_sets[1].title = String::from("DrugB");
    xy.x_cells = vec![vec![Cell::number(1.0), Cell::number(0.1), Cell::number(3.0)], vec![Cell::number(2.0), Cell::number(0.2), Cell::number(3.0)]];
    xy.data_sets[0].cells =
        vec![vec![Cell::number(10.0), Cell::number(1.0), Cell::number(3.0)], vec![Cell::number(20.0), Cell::number(2.0), Cell::number(3.0)]];
    xy.data_sets[1].cells =
        vec![vec![Cell::number(30.0), Cell::number(3.0), Cell::number(4.0)], vec![Cell::number(40.0), Cell::number(4.0), Cell::number(4.0)]];
    let sheet = DataSheet::new("xy-multi-x", "Multi X", TableArchetype::Xy(xy)).unwrap();
    let csv = export_datasheet_to_csv(&sheet, &CsvOptions::csv()).unwrap();
    let re = import_csv_to_datasheet("xy-re", "Multi X", &csv, TableKind::Xy, SubcolumnMode::MeanSdN, &CsvOptions::csv()).unwrap();
    let TableArchetype::Xy(ref re_xy) = re.archetype else { panic!("expected Xy") };
    assert_eq!(re_xy.x_mode, XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN });
    assert_eq!(re_xy.x_title, "Dose");
    assert_eq!(re_xy.x_cells[0], vec![Cell::number(1.0), Cell::number(0.1), Cell::number(3.0)]);
    assert_eq!(re_xy.data_sets.len(), 2);
    assert_eq!(re_xy.data_sets[0].title, "DrugA");
    assert_eq!(re_xy.data_sets[0].cells[0], vec![Cell::number(10.0), Cell::number(1.0), Cell::number(3.0)]);
    assert_eq!(re_xy.data_sets[1].title, "DrugB");
    assert_eq!(re_xy.data_sets[1].cells[1], vec![Cell::number(40.0), Cell::number(4.0), Cell::number(4.0)]);

    // Part 1b: Legacy X,X:2,X:3 headers in CSV and XLSX preserve 3 X subcolumns and keep Y unshifted.
    let legacy_csv = "X,X:2,X:3,A:Mean,A:SD,A:N\n1,0.1,3,10,1,3\n";
    let re_legacy = import_csv_to_datasheet("xy-leg", "Legacy", legacy_csv, TableKind::Xy, SubcolumnMode::MeanSdN, &CsvOptions::csv()).unwrap();
    let TableArchetype::Xy(ref leg_xy) = re_legacy.archetype else { panic!("expected Xy") };
    assert_eq!(leg_xy.x_mode, XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(3) });
    assert_eq!(leg_xy.x_cells[0], vec![Cell::number(1.0), Cell::number(0.1), Cell::number(3.0)]);
    assert_eq!(leg_xy.data_sets.len(), 1);
    assert_eq!(leg_xy.data_sets[0].title, "A");
    assert_eq!(leg_xy.data_sets[0].cells[0], vec![Cell::number(10.0), Cell::number(1.0), Cell::number(3.0)]);

    let xlsx_rows = r#"
        <row r="1">
            <c r="A1" t="inlineStr"><is><t>Dose:Mean</t></is></c>
            <c r="B1" t="inlineStr"><is><t>Dose:SD</t></is></c>
            <c r="C1" t="inlineStr"><is><t>Dose:N</t></is></c>
            <c r="D1" t="inlineStr"><is><t>DrugA:Mean</t></is></c>
            <c r="E1" t="inlineStr"><is><t>DrugA:SD</t></is></c>
            <c r="F1" t="inlineStr"><is><t>DrugA:N</t></is></c>
        </row>
        <row r="2">
            <c r="A2"><v>1</v></c><c r="B2"><v>0.1</v></c><c r="C2"><v>3</v></c>
            <c r="D2"><v>10</v></c><c r="E2"><v>1</v></c><c r="F2"><v>3</v></c>
        </row>
    "#;
    let xlsx_rep = import_xlsx_bytes_with_options(
        &xlsx_single_sheet_zip("MultiX", xlsx_rows),
        &XlsxImportOptions { table_kind: Some(TableKind::Xy), subcolumn_mode: SubcolumnMode::MeanSdN, ..Default::default() },
    )
    .unwrap();
    let TableArchetype::Xy(ref x_xy) = xlsx_rep.document.data_sheets[0].archetype else { panic!("expected Xy") };
    assert_eq!(x_xy.x_mode, XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN });
    assert_eq!(x_xy.x_cells[0], vec![Cell::number(1.0), Cell::number(0.1), Cell::number(3.0)]);
    assert_eq!(x_xy.data_sets.len(), 1);
    assert_eq!(x_xy.data_sets[0].cells[0], vec![Cell::number(10.0), Cell::number(1.0), Cell::number(3.0)]);

    // Explicit headerless helper works too.
    let no_hdr_opts = CsvOptions { header_rows: 0, ..CsvOptions::csv() };
    let hdrless = import_csv_to_datasheet_with_x_mode(
        "xy-hdrless",
        "Hdrless",
        "1,0.1,3,10\n",
        TableKind::Xy,
        SubcolumnMode::Single,
        Some(SubcolumnMode::MeanSdN),
        &no_hdr_opts,
    )
    .unwrap();
    let TableArchetype::Xy(ref h_xy) = hdrless.archetype else { panic!("expected Xy") };
    assert_eq!(h_xy.x_cells[0], vec![Cell::number(1.0), Cell::number(0.1), Cell::number(3.0)]);
    assert_eq!(h_xy.data_sets[0].cells[0], vec![Cell::number(10.0)]);
}

#[test]
fn aud_13_08_quoted_comma_grouped_and_decimal_comma_numbers() {
    // Part 2: Quoted comma-grouped numbers & decimal-comma numbers parse as CellValue::Number.
    let us_csv = "X,Y\n10,950\n20,\"1,250.5\"\n30,\"2,500.0*\"\n40,\"a,b*\"\n";
    let us_opts = CsvOptions::detect(us_csv);
    assert_eq!((us_opts.delimiter, us_opts.decimal_separator, us_opts.thousands_separator), (',', '.', Some(',')));
    let us_sheet = import_csv_to_datasheet("us-q", "US Quoted", us_csv, TableKind::Xy, SubcolumnMode::Single, &us_opts).unwrap();
    assert_eq!(us_sheet.cell(CellCoord::new(0, 0, 0)), Some(&Cell::number(950.0)));
    assert_eq!(us_sheet.cell(CellCoord::new(1, 0, 0)), Some(&Cell::number(1250.5)));
    assert_eq!(us_sheet.cell(CellCoord::new(2, 0, 0)), Some(&Cell::excluded_number(2500.0)));
    assert_eq!(us_sheet.cell(CellCoord::new(3, 0, 0)), Some(&Cell::text("a,b*")));

    let int_thou_csv = "X,Y\n10,\"1,250\"\n20,\"2,500\"\n";
    let int_opts = CsvOptions::detect(int_thou_csv);
    assert_eq!(int_opts.thousands_separator, Some(','));
    let int_sheet = import_csv_to_datasheet("us-int", "US Int Thou", int_thou_csv, TableKind::Xy, SubcolumnMode::Single, &int_opts).unwrap();
    assert_eq!(int_sheet.cell(CellCoord::new(0, 0, 0)), Some(&Cell::number(1250.0)));
    assert_eq!(int_sheet.cell(CellCoord::new(1, 0, 0)), Some(&Cell::number(2500.0)));

    let eu_comma_csv = "X,Y\n10,\"1,25\"\n20,\"2,50\"\n";
    let eu_opts = CsvOptions::detect(eu_comma_csv);
    assert_eq!((eu_opts.delimiter, eu_opts.decimal_separator), (',', ','));
    let eu_sheet = import_csv_to_datasheet("eu-q", "EU Quoted", eu_comma_csv, TableKind::Xy, SubcolumnMode::Single, &eu_opts).unwrap();
    assert_eq!(eu_sheet.cell(CellCoord::new(0, 0, 0)), Some(&Cell::number(1.25)));
    assert_eq!(eu_sheet.cell(CellCoord::new(1, 0, 0)), Some(&Cell::number(2.50)));
}

#[test]
fn aud_13_19_result_exports_preserve_outliers_pred_bands_footnotes_and_analyzable_data() {
    let mut rs = make_result_sheet("fit-rout", "data-1", ResultStatus::UpToDate);
    rs.tabs[0].footnotes = vec![String::from("ROUT Q = 1%"), String::from("=formula_note")];
    rs.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("four_pl"),
        converged: true,
        ambiguous: false,
        points: vec![CurveSamplePoint {
            x: -7.0,
            y: 50.0,
            ci_lower: Some(45.0),
            ci_upper: Some(55.0),
            pred_lower: Some(35.0),
            pred_upper: Some(65.0),
        }],
    });
    rs.eliminated_outliers.push(SourceCellId { sheet_id: SheetId::new("data-1"), row: 2, col: 0, subcol: 1 });
    let mut cleaned = col_sheet("fit-rout-cleaned", 1, 1);
    cleaned.set_cell(CellCoord::new(0, 0, 0), Cell::number(42.0)).unwrap();
    rs.analyzable_data = Some(cleaned);

    let opts = CsvOptions::csv();
    let tidy = parse_delimited_records(&export_result_sheet_tidy_csv(&rs, &opts), &opts).unwrap();
    assert_eq!(tidy[0].len(), 21);
    assert_eq!((tidy[0][19].as_str(), tidy[0][20].as_str()), ("pred_lower", "pred_upper"));
    assert!(tidy.iter().all(|r| r.len() == 21), "every tidy CSV row must match 21-column header width");
    assert!(tidy.iter().any(|r| r[2] == "curve_point" && r[14] == "45" && r[15] == "55" && r[19] == "35" && r[20] == "65"));
    assert!(tidy.iter().any(|r| r[2] == "tab_footnote" && r[3] == "summary" && r[8] == "ROUT Q = 1%"));
    assert!(tidy.iter().any(|r| r[2] == "tab_footnote" && r[8] == "'=formula_note"));
    assert!(tidy.iter().any(|r| r[2] == "eliminated_outlier" && r[3] == "data-1" && r[6] == "row_2" && r[7] == "col_0_sub_1"));

    let json_val: Value = serde_json::from_str(&export_result_sheet_json(&rs, false).unwrap()).unwrap();
    assert_eq!(json_val["eliminated_outliers"][0]["sheet_id"], json!("data-1"));
    assert_eq!(json_val["eliminated_outliers"][0]["row"], json!(2));
    assert_eq!(json_val["eliminated_outliers"][0]["subcol"], json!(1));
    assert_eq!(json_val["analyzable_data"]["id"], json!("fit-rout-cleaned"));

    // Non-finite float inside analyzable_data must be rejected by export_result_sheet_json.
    if let Some(ref mut ad) = rs.analyzable_data
        && let TableArchetype::Column(ref mut ct) = ad.archetype
    {
        ct.data_sets[0].cells[0][0] = Cell { value: CellValue::Number(f64::NAN), excluded: false };
    }
    assert!(matches!(export_result_sheet_json(&rs, false), Err(FormatError::NonFiniteFloat { .. })));
}

#[test]
fn aud_13_24_xlsx_presummarized_column_sets_precomputed_error_hint() {
    let rows = r#"
        <row r="1">
            <c r="A1" t="inlineStr"><is><t>Ctrl:Mean</t></is></c>
            <c r="B1" t="inlineStr"><is><t>Ctrl:SD</t></is></c>
            <c r="C1" t="inlineStr"><is><t>Ctrl:N</t></is></c>
        </row>
        <row r="2"><c r="A2"><v>10.5</v></c><c r="B2"><v>1.2</v></c><c r="C2"><v>6</v></c></row>
    "#;
    let bytes = xlsx_single_sheet_zip("SummaryCol", rows);
    for mode in [SubcolumnMode::MeanSdN, SubcolumnMode::MeanSemN, SubcolumnMode::MeanCvN] {
        let rep = import_xlsx_bytes_with_options(
            &bytes,
            &XlsxImportOptions { table_kind: Some(TableKind::Column), subcolumn_mode: mode, ..Default::default() },
        )
        .unwrap();
        let TableArchetype::Column(ref col) = rep.document.data_sheets[0].archetype else { panic!("expected Column") };
        assert_eq!(col.design_hint, ColumnDesignHint::PrecomputedError, "expected PrecomputedError for {mode:?}");
    }

    let single_rep = import_xlsx_bytes_with_options(
        &bytes,
        &XlsxImportOptions { table_kind: Some(TableKind::Column), subcolumn_mode: SubcolumnMode::Single, ..Default::default() },
    )
    .unwrap();
    let TableArchetype::Column(ref col) = single_rep.document.data_sheets[0].archetype else { panic!("expected Column") };
    assert_eq!(col.design_hint, ColumnDesignHint::UnpairedReplicates);
}
