//! Regression tests covering Wave 1 audit fixes for `curvebook-format`:
//! #26 (.pzfx verbatim titles/labels and strict XML Excluded), #27 (.prism Info ID remapping),
//! #28 (.prism bounded JSON DOM parsing), #29 (.pzfx/.prism SheetId::try_new validation & remapping),
//! #71 (failed ResultSheet CSV export refusal), #72 (CSV formula-injection neutralization),
//! and #73 (quote-aware CSV round-trip and `rsplit_once(':')` multi-subcolumn headers).

use std::collections::BTreeMap;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CellValue, ChecklistItem, ChecklistStatus, ColumnDesignHint, ColumnTable, DataSheet, ErrorCode,
    FittedCurveOverlay, GraphSheet, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoSheet, InternalLinkKind, LayoutItem, LayoutSheet,
    LayoutSpec, LinkSource, LinkedDataRegion, OneWayAnovaSpec, PairwiseComparisonRecord, PartsOfWholeTable, ProjectDocument, RectPt, ResultCell,
    ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetCellRange, SheetId, SubcolumnMode, TableArchetype, TableKind,
    XColumnMode, XyTable,
};
use curvebook_format::export::{ensure_result_sheet_csv_exportable, try_export_result_sheet_tidy_csv, try_export_result_tab_csv};
use curvebook_format::{
    CsvOptions, FormatError, ImportWarningKind, ZipWriter, export_datasheet_to_csv, export_result_sheet_tidy_csv, export_result_tab_csv,
    import_csv_to_datasheet, import_prism_bytes, import_pzfx_str, parse_delimited_records, save_project_bytes, validate_project_document,
};

fn build_stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let name_bytes = name.as_bytes();
        let crc = crc32fast::hash(data);
        assert!(data.len() <= usize::try_from(u32::MAX).unwrap_or(usize::MAX));
        let size = u32::try_from(data.len()).unwrap_or(0);
        let name_len = u16::try_from(name_bytes.len()).unwrap_or(0);
        let local_offset = u32::try_from(out.len()).unwrap_or(0);

        // Local file header (method = 0 stored)
        out.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        out.extend_from_slice(&20_u16.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(data);

        // Central directory header
        central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        central.extend_from_slice(&20_u16.to_le_bytes());
        central.extend_from_slice(&20_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&name_len.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u32.to_le_bytes());
        central.extend_from_slice(&local_offset.to_le_bytes());
        central.extend_from_slice(name_bytes);
    }
    let cd_offset = u32::try_from(out.len()).unwrap_or(0);
    let cd_size = u32::try_from(central.len()).unwrap_or(0);
    let count = u16::try_from(entries.len()).unwrap_or(0);
    out.extend_from_slice(&central);

    out.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out
}

#[test]
fn audit_26_pzfx_verbatim_titles_labels_sentinels_and_strict_xml_exclusions() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile>
  <Title>  Project Padded  </Title>
  <InfoSequence>
    <Info ID="info-1"><Title>  Metadata Padded  </Title></Info>
  </InfoSequence>
  <TableSequence>
    <Table ID="xy-1" TableType="XY">
      <Title>  Dose Response  </Title>
      <RowTitlesColumn>
        <Subcolumn>
          <d>NA</d><d>NaN</d><d>NULL</d><d>.</d><d>007</d><d>1.50</d><d>Control*</d><d>  Padded Row  </d>
        </Subcolumn>
      </RowTitlesColumn>
      <XColumn>
        <Title>  Time (hr)  </Title>
        <Subcolumn><d>1</d><d>2</d><d>3</d><d>4</d><d>5</d><d>6</d><d>7</d><d>8</d></Subcolumn>
      </XColumn>
      <YColumn>
        <Title>  Drug A  </Title>
        <Subcolumn>
          <Title>  Rep One  </Title>
          <d>NA</d>
          <d>NaN</d>
          <d>NULL</d>
          <d>.</d>
          <d>1.2*</d>
          <d>*2.4</d>
          <d>*</d>
          <d Excluded="1">9.5</d>
        </Subcolumn>
      </YColumn>
    </Table>
    <Table ID="mv-1" TableType="MultipleVariables">
      <Title>MV Sentinels</Title>
      <YColumn>
        <Title>Element</Title>
        <Subcolumn><d>Na</d><d>Ca*</d><d>  keep spaces  </d><d Excluded="true"/></Subcolumn>
      </YColumn>
    </Table>
    <Table ID="pow-1" TableType="PartsOfWhole">
      <Title>Slice Labels</Title>
      <XColumn>
        <Subcolumn><d>NA</d><d>007</d><d>1.50</d><d>Slice*</d><d>  Padded Slice  </d></Subcolumn>
      </XColumn>
      <YColumn><Title>Pct</Title><Subcolumn><d>10</d><d>20</d><d>30</d><d>25</d><d>15</d></Subcolumn></YColumn>
    </Table>
  </TableSequence>
</PrismFile>"#;

    let report = import_pzfx_str(xml).expect("import verbatim pzfx");
    let doc = &report.document;
    assert_eq!(doc.title, "  Project Padded  ");
    assert_eq!(doc.info_sheets[0].title, "  Metadata Padded  ");

    let TableArchetype::Xy(ref xy) = doc.data_sheets[0].archetype else {
        panic!("expected XY");
    };
    assert_eq!(doc.data_sheets[0].title, "  Dose Response  ");
    assert_eq!(xy.x_title, "  Time (hr)  ");
    assert_eq!(xy.data_sets[0].title, "  Drug A  ");
    assert_eq!(xy.row_titles, vec!["NA", "NaN", "NULL", ".", "007", "1.50", "Control*", "  Padded Row  "]);
    let expected_cells = [
        Cell::text("NA"),
        Cell::text("NaN"),
        Cell::text("NULL"),
        Cell::text("."),
        Cell::text("1.2*"),
        Cell::text("*2.4"),
        Cell::text("*"),
        Cell::excluded_number(9.5),
    ];
    for (r, exp) in expected_cells.iter().enumerate() {
        assert_eq!(xy.data_sets[0].cell(r, 0), Some(exp), "row {r}");
    }

    let TableArchetype::MultipleVariables(ref mv) = doc.data_sheets[1].archetype else {
        panic!("expected MultipleVariables");
    };
    assert_eq!(mv.variables[0].cells[0], Cell::text("Na"));
    assert_eq!(mv.variables[0].cells[1], Cell::text("Ca*"));
    assert_eq!(mv.variables[0].cells[2], Cell::text("  keep spaces  "));
    assert_eq!(mv.variables[0].cells[3], Cell { value: CellValue::Empty, excluded: true });

    let TableArchetype::PartsOfWhole(ref pow) = doc.data_sheets[2].archetype else {
        panic!("expected PartsOfWhole");
    };
    assert_eq!(pow.slice_labels, vec!["NA", "007", "1.50", "Slice*", "  Padded Slice  "]);
}

#[test]
fn audit_27_prism_merge_embedded_pzfx_remaps_clashing_info_sheet_ids() {
    let pzfx_a = r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile>
  <InfoSequence><Info ID="info-1"><Title>Info A</Title><Constant><Name>Source</Name><Value>FileA</Value></Constant></Info></InfoSequence>
  <TableSequence><Table ID="tbl" TableType="Column"><Title>Table A</Title><YColumn><Subcolumn><d>1</d></Subcolumn></YColumn></Table></TableSequence>
</PrismFile>"#;
    let pzfx_b = r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile>
  <InfoSequence><Info ID="info-1"><Title>Info B</Title><Constant><Name>Source</Name><Value>FileB</Value></Constant></Info></InfoSequence>
  <TableSequence><Table ID="tbl" TableType="Column"><Title>Table B</Title><YColumn><Subcolumn><d>2</d></Subcolumn></YColumn></Table></TableSequence>
</PrismFile>"#;

    let mut zip = ZipWriter::new();
    zip.add_file("info/base.json", br#"{"id":"info-1","title":"Base Info","constants":{"Source":"Base"}}"#.to_vec()).unwrap();
    zip.add_file("embedded_a.pzfx", pzfx_a.as_bytes().to_vec()).unwrap();
    zip.add_file("embedded_b.pzfx", pzfx_b.as_bytes().to_vec()).unwrap();
    let bytes = zip.finish().unwrap();

    let report = import_prism_bytes(&bytes).expect("import prism with two clashing embedded pzfx files");
    let doc = &report.document;
    assert_eq!(doc.info_sheets.len(), 3);
    assert_eq!(doc.data_sheets.len(), 2);

    let info_by_id: BTreeMap<&str, &InfoSheet> = doc.info_sheets.iter().map(|s| (s.id.as_str(), s)).collect();
    assert_eq!(info_by_id.len(), 3);

    for ds in &doc.data_sheets {
        let linked_id = ds.info_sheet_id.as_ref().expect("merged sheet keeps its info_sheet_id");
        let linked = info_by_id.get(linked_id.as_str()).expect("linked info sheet exists");
        match ds.title.as_str() {
            "Table A" => assert_eq!(linked.title, "Info A"),
            "Table B" => assert_eq!(linked.title, "Info B"),
            other => panic!("unexpected table title {other}"),
        }
    }
}

#[test]
fn audit_28_prism_bounds_embedded_document_and_manifest_json() {
    // > 2,000,000 empty arrays exceeds MAX_MIGRATION_JSON_NODES safely inside ParseBoundedJsonOutcome.
    let mut array_bomb = Vec::with_capacity(6_100_000);
    array_bomb.push(b'[');
    for i in 0..2_010_000 {
        if i > 0 {
            array_bomb.push(b',');
        }
        array_bomb.extend_from_slice(b"[]");
    }
    array_bomb.push(b']');

    let doc_bomb_zip = build_stored_zip(&[("document.json", &array_bomb)]);
    assert!(matches!(import_prism_bytes(&doc_bomb_zip), Err(FormatError::Json(_))));

    let manifest_bomb_zip = build_stored_zip(&[("manifest.json", &array_bomb), ("sheets/a.csv", b"A\n1\n")]);
    assert!(matches!(import_prism_bytes(&manifest_bomb_zip), Err(FormatError::Json(_))));

    let info_bomb_zip = build_stored_zip(&[("info/info.json", &array_bomb)]);
    assert!(matches!(import_prism_bytes(&info_bomb_zip), Err(FormatError::Json(_))));
}

#[test]
fn audit_29_pzfx_and_prism_validate_and_sanitize_invalid_sheet_ids() {
    // 1. PZFX invalid IDs (spaces, `:`, `{{`, `}}`, whitespace-only, >256 bytes)
    let long_valid_id = "s".repeat(270);
    let pzfx_xml = format!(
        r#"<?xml version="1.0"?>
<PrismFile>
  <InfoSequence><Info ID="bad info:1"><Title>Info</Title></Info></InfoSequence>
  <TableSequence>
    <Table ID="tbl 1:dose {{{{x}}}}" TableType="Column"><Title>T1</Title><YColumn><Subcolumn><d>1</d></Subcolumn></YColumn></Table>
    <Table ID="   " TableType="Column"><Title>T2</Title><YColumn><Subcolumn><d>2</d></Subcolumn></YColumn></Table>
    <Table ID="{long_valid_id}" TableType="Column"><Title>T3</Title><YColumn><Subcolumn><d>3</d></Subcolumn></YColumn></Table>
  </TableSequence>
</PrismFile>"#
    );
    let pzfx_rep = import_pzfx_str(&pzfx_xml).expect("pzfx with sanitized invalid IDs");
    for s in &pzfx_rep.document.info_sheets {
        assert!(SheetId::try_new(s.id.as_str()).is_ok(), "invalid info id: {:?}", s.id);
    }
    for ds in &pzfx_rep.document.data_sheets {
        assert!(SheetId::try_new(ds.id.as_str()).is_ok(), "invalid data id: {:?}", ds.id);
        assert_eq!(ds.info_sheet_id.as_ref(), Some(&pzfx_rep.document.info_sheets[0].id));
    }
    assert!(pzfx_rep.warnings.iter().filter(|w| w.kind == ImportWarningKind::CoercedValue).count() >= 4);

    // 2. .prism manifest + CSV invalid IDs
    let mut archive_zip = ZipWriter::new();
    archive_zip
        .add_file(
            "manifest.json",
            br#"{"title":"Archive Invalid IDs","info_sheets":[{"id":"info : bad","title":"Info"}],"sheets":[{"id":"data {{1}}","kind":"Column","csv_path":"sheets/a.csv"}]}"#.to_vec(),
        )
        .unwrap();
    archive_zip.add_file("sheets/a.csv", b"A\n1\n".to_vec()).unwrap();
    let archive_rep = import_prism_bytes(&archive_zip.finish().unwrap()).expect("archive with sanitized manifest/csv IDs");
    assert!(SheetId::try_new(archive_rep.document.info_sheets[0].id.as_str()).is_ok());
    assert!(SheetId::try_new(archive_rep.document.data_sheets[0].id.as_str()).is_ok());
    assert_eq!(archive_rep.document.data_sheets[0].info_sheet_id.as_ref(), Some(&archive_rep.document.info_sheets[0].id));
    assert!(archive_rep.warnings.iter().filter(|w| w.kind == ImportWarningKind::CoercedValue).count() >= 2);

    // 3. Embedded document.json with invalid SheetIds and cross-sheet references remaps cleanly
    let bad_info_id = SheetId::new("info bad:id");
    let bad_up_id = SheetId::new("up stream:0");
    let bad_data_id = SheetId::new("data {{1}}");
    let bad_res_id = SheetId::new("res 1\tbad");
    let bad_graph_id = SheetId::new("graph:bad");
    let bad_layout_id = SheetId::new("layout}}bad");

    let mut info = InfoSheet::new_preseeded(bad_info_id.as_str().to_owned(), "Info").unwrap();
    info.linked_data_sheets.push(bad_data_id.clone());

    let mut up_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).unwrap();
    up_col.data_sets[0].cells[0][0] = Cell::number(9.0);
    let up_ds = DataSheet::new(bad_up_id.clone(), "Upstream", TableArchetype::Column(up_col)).unwrap();

    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap();
    col.data_sets[0].cells[0][0] = Cell::number(1.0);
    col.data_sets[0].cells[1][0] = Cell::number(2.0);
    let mut ds = DataSheet::new(bad_data_id.clone(), "Data", TableArchetype::Column(col)).unwrap();
    ds.info_sheet_id = Some(bad_info_id.clone());
    ds.linked_regions.push(LinkedDataRegion {
        id: String::from("link-1"),
        spec: ImportLinkSpec {
            source: LinkSource::InternalSheet {
                sheet_id: bad_up_id.clone(),
                kind: InternalLinkKind::DataSheet,
                tab_id: None,
                range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
            },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
        provenance_summary: String::from("upstream link"),
    });

    let spec = AnalysisSpec::new(bad_res_id.clone(), bad_data_id.clone(), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).unwrap();
    let res = ResultSheet::new(spec).unwrap();
    let gs = GraphSheet::new_uninitialized_for_data(bad_graph_id.clone(), &ds).unwrap();
    let mut ls_spec = LayoutSpec::default();
    ls_spec.items.push(LayoutItem::LiveGraph {
        id: String::from("panel-1"),
        graph_sheet_id: bad_graph_id.clone(),
        bounds_pt: RectPt::new(10.0, 10.0, 100.0, 80.0),
        scale_percent: 100.0,
        badge_override: None,
    });
    let ls = LayoutSheet::new(bad_layout_id.clone(), "Layout", ls_spec).unwrap();

    let mut doc = ProjectDocument::new("Embedded Bad IDs").unwrap();
    doc.info_sheets.push(info);
    doc.data_sheets.push(up_ds);
    doc.data_sheets.push(ds);
    doc.result_sheets.push(res);
    doc.graph_sheets.push(gs);
    doc.layout_sheets.push(ls);
    doc.active_sheet_id = Some(bad_graph_id);
    doc.family_anchor_sheet_id = Some(bad_data_id);

    let mut doc_zip = ZipWriter::new();
    doc_zip.add_file("document.json", serde_json::to_vec(&doc).unwrap()).unwrap();
    let loaded = import_prism_bytes(&doc_zip.finish().unwrap()).expect("sanitize embedded document.json IDs");

    let clean = &loaded.document;
    validate_project_document(clean).expect("sanitized document validates");
    assert_eq!(loaded.warnings.iter().filter(|w| w.kind == ImportWarningKind::CoercedValue).count(), 6);
    assert!(SheetId::try_new(clean.info_sheets[0].id.as_str()).is_ok());
    assert!(SheetId::try_new(clean.data_sheets[0].id.as_str()).is_ok());
    assert!(SheetId::try_new(clean.data_sheets[1].id.as_str()).is_ok());
    assert!(SheetId::try_new(clean.result_sheets[0].id.as_str()).is_ok());
    assert!(SheetId::try_new(clean.graph_sheets[0].id.as_str()).is_ok());
    assert!(SheetId::try_new(clean.layout_sheets[0].id.as_str()).is_ok());

    let clean_up_id = &clean.data_sheets[0].id;
    let clean_ds = &clean.data_sheets[1];
    assert_eq!(clean.info_sheets[0].linked_data_sheets, vec![clean_ds.id.clone()]);
    assert_eq!(clean_ds.info_sheet_id.as_ref(), Some(&clean.info_sheets[0].id));
    assert_eq!(clean_ds.linked_regions[0].spec.source.upstream_sheet_id(), Some(clean_up_id));
    assert_eq!(clean.result_sheets[0].analysis.id, clean.result_sheets[0].id);
    assert_eq!(clean.result_sheets[0].analysis.source_sheet_id, clean_ds.id);
    assert_eq!(clean.graph_sheets[0].spec.primary_source_sheet_id, clean_ds.id);
    assert_eq!(clean.active_sheet_id.as_ref(), Some(&clean.graph_sheets[0].id));
    assert_eq!(clean.family_anchor_sheet_id.as_ref(), Some(&clean_ds.id));
}

#[test]
fn audit_71_failed_result_sheet_csv_export_refused() {
    let spec = AnalysisSpec::new("res-failed-anova", "data-1", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).unwrap();
    let mut sheet = ResultSheet::new(spec).unwrap();
    sheet.tabs.push(ResultTab {
        id: String::from("tab-1"),
        title: String::from("Summary"),
        kind: ResultTabKind::AnovaTable,
        column_headers: vec![String::from("Val")],
        row_labels: vec![String::from("R0")],
        rows: vec![vec![ResultCell::Number { value: 1.0, prefix: None, units: None }]],
        footnotes: vec![],
    });
    let opts = CsvOptions::csv();

    // Dirty and UpToDate sheets export normally.
    assert!(ensure_result_sheet_csv_exportable(&sheet).is_ok());
    assert!(try_export_result_sheet_tidy_csv(&sheet, &opts).is_ok());
    assert!(try_export_result_tab_csv(&sheet, &sheet.tabs[0], &opts).is_ok());
    sheet.status = ResultStatus::UpToDate;
    assert!(try_export_result_sheet_tidy_csv(&sheet, &opts).is_ok());

    // Failed result sheet must be rejected instead of writing a header-only CSV.
    sheet.status = ResultStatus::Failed { code: ErrorCode::InsufficientData, message: String::from("at least 2 valid groups are required") };
    let err_tidy = try_export_result_sheet_tidy_csv(&sheet, &opts).unwrap_err();
    let msg_tidy = err_tidy.to_string();
    assert!(msg_tidy.contains("res-failed-anova"), "error must name failed sheet: {msg_tidy}");
    assert!(msg_tidy.contains("insufficient_data"), "error must include code: {msg_tidy}");
    assert!(msg_tidy.contains("at least 2 valid groups are required"), "error must include cause: {msg_tidy}");

    let err_tab = try_export_result_tab_csv(&sheet, &sheet.tabs[0], &opts).unwrap_err();
    assert!(err_tab.to_string().contains("res-failed-anova"));
}

#[test]
fn audit_72_csv_formula_injection_neutralized_while_numbers_untouched() {
    // 1. DataSheet CSV export neutralizes `=`, `+`, `-`, `@`, `\t`, `\r` in text/headers/labels while keeping numeric `-1.5` untouched.
    let mut tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 8).unwrap();
    tbl.row_titles_expanded = true;
    tbl.row_titles = vec![
        String::from("=Row0"),
        String::from("+Row1"),
        String::from("-Row2"),
        String::from("@Row3"),
        String::from("\tRow4"),
        String::from("\rRow5"),
        String::from("SafeRow6"),
        String::from("SafeRow7"),
    ];
    tbl.data_sets[0].title = String::from("=ColTitle");
    tbl.data_sets[1].title = String::from("NumericCol");

    let text_payloads = ["=SUM(A1)", "+1+1", "-cmd", "@A1", "\tcmd", "\rcmd", "-1.5", "ok"];
    for (r, txt) in text_payloads.iter().enumerate() {
        tbl.data_sets[0].cells[r][0] = Cell::text(*txt);
    }
    tbl.data_sets[1].cells[0][0] = Cell::number(-1.5);
    tbl.data_sets[1].cells[1][0] = Cell::excluded_number(-2.5);

    let sheet = DataSheet::new("formula-ds", "Formula Sheet", TableArchetype::Column(tbl)).unwrap();
    let opts = CsvOptions::csv();
    let exported = export_datasheet_to_csv(&sheet, &opts).unwrap();
    let parsed = parse_delimited_records(&exported, &opts).unwrap();

    assert_eq!(parsed[0], vec!["Row Title", "'=ColTitle", "NumericCol"]);
    assert_eq!(parsed[1][0], "'=Row0");
    assert_eq!(parsed[2][0], "'+Row1");
    assert_eq!(parsed[3][0], "'-Row2");
    assert_eq!(parsed[4][0], "'@Row3");
    assert_eq!(parsed[5][0], "'\tRow4");
    assert_eq!(parsed[6][0], "'\rRow5");
    assert_eq!(parsed[1][1], "'=SUM(A1)");
    assert_eq!(parsed[2][1], "'+1+1");
    assert_eq!(parsed[3][1], "'-cmd");
    assert_eq!(parsed[4][1], "'@A1");
    assert_eq!(parsed[5][1], "'\tcmd");
    assert_eq!(parsed[6][1], "'\rcmd");
    assert_eq!(parsed[7][1], "'-1.5");
    assert_eq!(parsed[8][1], "ok");
    // Finite numeric cells remain unquoted and unprefixed (`-1.5`, `-2.5*`).
    assert_eq!(parsed[1][2], "-1.5");
    assert_eq!(parsed[2][2], "-2.5*");

    let reimported = import_csv_to_datasheet(
        "formula-ds-re",
        "Formula Sheet",
        &exported,
        TableKind::Column,
        SubcolumnMode::Single,
        &CsvOptions { has_row_titles: true, ..opts.clone() },
    )
    .unwrap();
    let TableArchetype::Column(ref re_col) = reimported.archetype else {
        panic!("expected Column");
    };
    assert_eq!(re_col.data_sets[1].cells[0][0], Cell::number(-1.5));
    assert_eq!(re_col.data_sets[1].cells[1][0], Cell::excluded_number(-2.5));
    assert_eq!(re_col.data_sets[0].cells[4][0], Cell::text("'\tcmd"));
    assert_eq!(re_col.data_sets[0].cells[5][0], Cell::text("'\rcmd"));

    // 2. PartsOfWhole slice labels also neutralized.
    let mut pow = PartsOfWholeTable::new(1, 2).unwrap();
    pow.slice_labels = vec![String::from("=SliceA"), String::from("-SliceB")];
    pow.data_sets[0].cells[0][0] = Cell::number(40.0);
    pow.data_sets[0].cells[1][0] = Cell::number(60.0);
    let pow_csv = export_datasheet_to_csv(&DataSheet::new("pow", "PoW", TableArchetype::PartsOfWhole(pow)).unwrap(), &opts).unwrap();
    let pow_rows = parse_delimited_records(&pow_csv, &opts).unwrap();
    assert_eq!(pow_rows[1][0], "'=SliceA");
    assert_eq!(pow_rows[2][0], "'-SliceB");
}

#[test]
fn audit_72_result_sheet_csv_formula_injection_neutralized_while_numbers_untouched() {
    let opts = CsvOptions::csv();
    // 3. ResultSheet wide tab CSV and tidy CSV neutralize all non-numeric strings while leaving finite numbers untouched.
    let spec = AnalysisSpec::new("=res-id", "data-1", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).unwrap();
    let mut res = ResultSheet::new(spec).unwrap();
    res.analysis.custom_title = Some(String::from("+BadTitle"));
    res.status = ResultStatus::UpToDate;
    res.scalars.push(ResultScalarEntry { key: String::from("@Key"), data_set_index: Some(0), value: -3.25, units: Some(String::from("=uM")) });
    res.tabs.push(ResultTab {
        id: String::from("-tab"),
        title: String::from("\tTabTitle"),
        kind: ResultTabKind::AnovaTable,
        column_headers: vec![String::from("=H1"), String::from("+H2"), String::from("-H3")],
        row_labels: vec![String::from("@R1")],
        rows: vec![vec![
            ResultCell::Number { value: -1.5, prefix: None, units: None },
            ResultCell::Text(String::from("=HYPERLINK(\"http://evil\")")),
            ResultCell::PValue { raw: 0.001, formatted: String::from("+0.001"), stars: String::from("-star") },
        ]],
        footnotes: vec![],
    });
    res.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: 0,
        group_b: 1,
        label_a: String::from("=A"),
        label_b: String::from("B"),
        estimate_diff: Some(-4.5),
        raw_p_or_q: 0.01,
        is_fdr_q: false,
        significant: true,
    });
    res.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("@eq"),
        converged: true,
        ambiguous: false,
        points: vec![curvebook_doc::CurveSamplePoint {
            x: -2.0,
            y: -3.0,
            ci_lower: Some(-4.5),
            ci_upper: Some(1.5),
            pred_lower: None,
            pred_upper: None,
        }],
    });
    res.checklist.push(ChecklistItem {
        assumption: String::from("-Assumption"),
        status: ChecklistStatus::Pass,
        diagnostic_metric: Some(String::from("=metric")),
        explanation: String::from("\texplanation"),
        remediation: None,
    });

    let tab_csv = export_result_tab_csv(&res.tabs[0], &opts);
    let tab_rows = parse_delimited_records(&tab_csv, &opts).unwrap();
    assert_eq!(tab_rows[0], vec!["Row", "'=H1", "'+H2", "'-H3"]);
    assert_eq!(tab_rows[1], vec!["'@R1", "-1.5", "'=HYPERLINK(\"http://evil\")", "'+0.001 (-star)"]);

    let tidy_csv = export_result_sheet_tidy_csv(&res, &opts);
    let tidy_rows = parse_delimited_records(&tidy_csv, &opts).unwrap();
    // tab_cell numeric stays `-1.5` in both `value_text` and `numeric_value`
    assert!(tidy_rows.iter().any(|r| r[2] == "tab_cell" && r[0] == "'=res-id" && r[1] == "'+BadTitle" && r[8] == "-1.5" && r[9] == "-1.5"));
    assert!(tidy_rows.iter().any(|r| r[2] == "scalar" && r[6] == "'@Key" && r[8] == "-3.25" && r[11] == "'=uM"));
    assert!(tidy_rows.iter().any(|r| r[2] == "pairwise_comparison" && r[6] == "'=A vs B" && r[9] == "-4.5"));
    assert!(tidy_rows.iter().any(|r| r[2] == "curve_point" && r[3] == "'@eq" && r[6] == "-2" && r[9] == "-3" && r[14] == "-4.5"));
    assert!(tidy_rows.iter().any(|r| r[2] == "checklist" && r[6] == "'-Assumption" && r[8] == "'\texplanation" && r[18] == "'=metric"));
}

#[test]
fn audit_73_quote_aware_csv_round_trip_and_colon_subcolumn_headers() {
    // 1. Subcolumn header with colon (`"Drug: high"` in MeanSdN) round-trips without truncating to `"Drug"`.
    let mut xy = XyTable::new_with_x_mode(XColumnMode::default(), SubcolumnMode::MeanSdN, 1, 2).unwrap();
    xy.data_sets[0].title = String::from("Drug: high");
    xy.x_cells[0][0] = Cell::number(1.0);
    xy.x_cells[1][0] = Cell::number(2.0);
    xy.data_sets[0].cells[0] = vec![Cell::number(10.0), Cell::number(1.0), Cell::number(3.0)];
    xy.data_sets[0].cells[1] = vec![Cell::number(20.0), Cell::number(2.0), Cell::number(3.0)];

    let opts = CsvOptions::csv();
    let xy_csv = export_datasheet_to_csv(&DataSheet::new("xy-colon", "Colon Test", TableArchetype::Xy(xy)).unwrap(), &opts).unwrap();
    assert!(xy_csv.lines().next().unwrap().contains("Drug: high:Mean,Drug: high:SD,Drug: high:N"));
    let xy_re = import_csv_to_datasheet("xy-colon-re", "Colon Test", &xy_csv, TableKind::Xy, SubcolumnMode::MeanSdN, &opts).unwrap();
    let TableArchetype::Xy(ref re_xy) = xy_re.archetype else {
        panic!("expected XY");
    };
    assert_eq!(re_xy.data_sets[0].title, "Drug: high");

    // 2. Quote-aware round-trip of ambiguous text tokens vs empty vs numbers vs exclusions.
    let cases = [
        Cell::text("NA"),
        Cell::text("."),
        Cell::text("NULL"),
        Cell::text("NaN"),
        Cell::text("abc*"),
        Cell::text("*abc"),
        Cell::text("*"),
        Cell::text("007"),
        Cell::text("1e3"),
        Cell::text("  lead_and_trail  "),
        Cell::text(""),
        Cell::empty(),
        Cell { value: CellValue::Empty, excluded: true },
        Cell::number(-1.5),
        Cell::excluded_number(3.5),
    ];
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, cases.len()).unwrap();
    col.data_sets[0].title = String::from("  Padded Title  ");
    for (r, cell) in cases.iter().enumerate() {
        col.data_sets[0].cells[r][0] = cell.clone();
    }
    let orig_sheet = DataSheet::new("quote-rt", "Quote Round-Trip", TableArchetype::Column(col)).unwrap();
    let csv_out = export_datasheet_to_csv(&orig_sheet, &opts).unwrap();

    // Verify ambiguous values were explicitly quoted on export while empty/number/-1.5 stayed unquoted.
    let lines: Vec<&str> = csv_out.lines().collect();
    assert_eq!(lines[0], "\"  Padded Title  \"");
    assert_eq!(lines[1], "\"NA\"");
    assert_eq!(lines[2], "\".\"");
    assert_eq!(lines[3], "\"NULL\"");
    assert_eq!(lines[4], "\"NaN\"");
    assert_eq!(lines[5], "\"abc*\"");
    assert_eq!(lines[6], "\"*abc\"");
    assert_eq!(lines[7], "\"*\"");
    assert_eq!(lines[8], "\"007\"");
    assert_eq!(lines[9], "\"1e3\"");
    assert_eq!(lines[10], "\"  lead_and_trail  \"");
    assert_eq!(lines[11], "\"\"");
    assert_eq!(lines[12], "");
    assert_eq!(lines[13], "*");
    assert_eq!(lines[14], "-1.5");
    assert_eq!(lines[15], "3.5*");

    let rt = import_csv_to_datasheet("quote-rt-re", "Quote Round-Trip", &csv_out, TableKind::Column, SubcolumnMode::Single, &opts).unwrap();
    let TableArchetype::Column(ref rt_col) = rt.archetype else {
        panic!("expected Column");
    };
    assert_eq!(rt_col.data_sets[0].title, "  Padded Title  ");
    for (r, expected) in cases.iter().enumerate() {
        assert_eq!(&rt_col.data_sets[0].cells[r][0], expected, "round-trip mismatch at row {r}");
    }

    // 3. European CSV preserves Cell::text("1,5") as quoted verbatim text distinct from Cell::number(1.5).
    let euro_opts = CsvOptions::european_csv();
    let mut euro = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap();
    euro.data_sets[0].title = String::from("Euro");
    euro.data_sets[0].cells[0][0] = Cell::number(1.5);
    euro.data_sets[0].cells[1][0] = Cell::text("1,5");
    let euro_csv = export_datasheet_to_csv(&DataSheet::new("euro", "Euro", TableArchetype::Column(euro)).unwrap(), &euro_opts).unwrap();
    assert_eq!(euro_csv, "Euro\n1,5\n\"1,5\"");
    let euro_re = import_csv_to_datasheet("euro-re", "Euro", &euro_csv, TableKind::Column, SubcolumnMode::Single, &euro_opts).unwrap();
    let TableArchetype::Column(ref re_euro) = euro_re.archetype else {
        panic!("expected Column");
    };
    assert_eq!(re_euro.data_sets[0].cells[0][0], Cell::number(1.5));
    assert_eq!(re_euro.data_sets[0].cells[1][0], Cell::text("1,5"));

    // 4. `save_project_bytes` round-trip succeeds cleanly on imported quoted project.
    let mut doc = ProjectDocument::new("RT").unwrap();
    doc.add_data_sheet(rt).unwrap();
    let saved = save_project_bytes(&doc).unwrap();
    assert!(!saved.is_empty());
}
