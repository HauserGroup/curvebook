//! Regression tests covering Wave 1 audit fixes for `curvebook-format`:
//! #26 (.pzfx verbatim titles/labels and strict XML Excluded), #27 (.prism Info ID remapping),
//! #28 (.prism bounded JSON DOM parsing), #29 (.pzfx/.prism SheetId::try_new validation & remapping),
//! #72 (CSV formula-injection neutralization), and #73 (quote-aware CSV round-trip and `rsplit_once(':')` headers).
//! (Failed ResultSheet CSV export #71 is verified in `audit_p3_sec04::fa_sec04_04_result_csv_reexports_and_registry_refuse_failed_status`.)

use std::collections::BTreeMap;

use crate::common::{RawZipEntry, build_raw_zip, build_zip_entries, live_graph_item, make_array_bomb_json, pzfx_doc, result_sheet_with_kind};
use curvebook_doc::{
    AnalysisKind, Cell, CellCoord, CellValue, ChecklistItem, ChecklistStatus, ColumnDesignHint, ColumnTable, CurveSamplePoint, DataSheet,
    FittedCurveOverlay, GraphSheet, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoSheet, InternalLinkKind, LayoutSheet, LayoutSpec,
    LinkSource, LinkedDataRegion, OneWayAnovaSpec, PairwiseComparisonRecord, PartsOfWholeTable, ProjectDocument, ResultCell, ResultScalarEntry,
    ResultStatus, ResultTab, ResultTabKind, SheetCellRange, SheetId, SubcolumnMode, TableArchetype, TableKind, XColumnMode, XyTable,
};
use curvebook_format::{
    CsvOptions, FormatError, ImportWarningKind, export_datasheet_to_csv, export_result_sheet_tidy_csv, export_result_tab_csv,
    import_csv_to_datasheet, import_prism_bytes, import_pzfx_str, parse_delimited_records, save_project_bytes, validate_project_document,
};

#[test]
fn audit_26_pzfx_verbatim_titles_labels_sentinels_and_strict_xml_exclusions() {
    let xml = pzfx_doc(
        r#"<Title>  Project Padded  </Title>
  <InfoSequence><Info ID="info-1"><Title>  Metadata Padded  </Title></Info></InfoSequence>
  <TableSequence>
    <Table ID="xy-1" TableType="XY"><Title>  Dose Response  </Title>
      <RowTitlesColumn><Subcolumn><d>NA</d><d>NaN</d><d>NULL</d><d>.</d><d>007</d><d>1.50</d><d>Control*</d><d>  Padded Row  </d></Subcolumn></RowTitlesColumn>
      <XColumn><Title>  Time (hr)  </Title><Subcolumn><d>1</d><d>2</d><d>3</d><d>4</d><d>5</d><d>6</d><d>7</d><d>8</d></Subcolumn></XColumn>
      <YColumn><Title>  Drug A  </Title><Subcolumn><Title>  Rep One  </Title><d>NA</d><d>NaN</d><d>NULL</d><d>.</d><d>1.2*</d><d>*2.4</d><d>*</d><d Excluded="1">9.5</d></Subcolumn></YColumn>
    </Table>
    <Table ID="mv-1" TableType="MultipleVariables"><Title>MV Sentinels</Title><YColumn><Title>Element</Title><Subcolumn><d>Na</d><d>Ca*</d><d>  keep spaces  </d><d Excluded="true"/></Subcolumn></YColumn></Table>
    <Table ID="pow-1" TableType="PartsOfWhole"><Title>Slice Labels</Title><XColumn><Subcolumn><d>NA</d><d>007</d><d>1.50</d><d>Slice*</d><d>  Padded Slice  </d></Subcolumn></XColumn><YColumn><Title>Pct</Title><Subcolumn><d>10</d><d>20</d><d>30</d><d>25</d><d>15</d></Subcolumn></YColumn></Table>
  </TableSequence>"#,
    );
    let doc = import_pzfx_str(&xml).unwrap().document;
    assert_eq!((doc.title.as_str(), doc.info_sheets[0].title.as_str()), ("  Project Padded  ", "  Metadata Padded  "));

    let TableArchetype::Xy(ref xy) = doc.data_sheets[0].archetype else { panic!("expected XY") };
    assert_eq!(
        (doc.data_sheets[0].title.as_str(), xy.x_title.as_str(), xy.data_sets[0].title.as_str()),
        ("  Dose Response  ", "  Time (hr)  ", "  Drug A  ")
    );
    assert_eq!(xy.row_titles, vec!["NA", "NaN", "NULL", ".", "007", "1.50", "Control*", "  Padded Row  "]);
    let expected = [
        Cell::text("NA"),
        Cell::text("NaN"),
        Cell::text("NULL"),
        Cell::text("."),
        Cell::text("1.2*"),
        Cell::text("*2.4"),
        Cell::text("*"),
        Cell::excluded_number(9.5),
    ];
    assert_eq!(xy.data_sets[0].cells.iter().map(|r| r[0].clone()).collect::<Vec<_>>(), expected);

    let TableArchetype::MultipleVariables(ref mv) = doc.data_sheets[1].archetype else { panic!("expected MV") };
    assert_eq!(
        mv.variables[0].cells,
        vec![Cell::text("Na"), Cell::text("Ca*"), Cell::text("  keep spaces  "), Cell { value: CellValue::Empty, excluded: true }]
    );
    let TableArchetype::PartsOfWhole(ref pow) = doc.data_sheets[2].archetype else { panic!("expected PoW") };
    assert_eq!(pow.slice_labels, vec!["NA", "007", "1.50", "Slice*", "  Padded Slice  "]);
}

#[test]
fn audit_27_prism_merge_embedded_pzfx_remaps_clashing_info_sheet_ids() {
    let make_pzfx = |s: &str, v: &str| {
        pzfx_doc(&format!(
            r#"<InfoSequence><Info ID="info-1"><Title>Info {s}</Title><Constant><Name>Source</Name><Value>File{s}</Value></Constant></Info></InfoSequence><TableSequence><Table ID="tbl" TableType="Column"><Title>Table {s}</Title><YColumn><Subcolumn><d>{v}</d></Subcolumn></YColumn></Table></TableSequence>"#
        ))
    };
    let (pzfx_a, pzfx_b) = (make_pzfx("A", "1"), make_pzfx("B", "2"));
    let doc = import_prism_bytes(&build_zip_entries(&[
        ("info/base.json", br#"{"id":"info-1","title":"Base Info","constants":{"Source":"Base"}}"#),
        ("embedded_a.pzfx", pzfx_a.as_bytes()),
        ("embedded_b.pzfx", pzfx_b.as_bytes()),
    ]))
    .unwrap()
    .document;
    assert_eq!((doc.info_sheets.len(), doc.data_sheets.len()), (3, 2));
    let info_by_id: BTreeMap<&str, &InfoSheet> = doc.info_sheets.iter().map(|s| (s.id.as_str(), s)).collect();
    assert_eq!(info_by_id.len(), 3);
    for ds in &doc.data_sheets {
        assert_eq!(info_by_id[ds.info_sheet_id.as_ref().unwrap().as_str()].title, if ds.title == "Table A" { "Info A" } else { "Info B" });
    }
}

#[test]
fn audit_28_prism_bounds_embedded_document_and_manifest_json() {
    let bomb = make_array_bomb_json(2_010_000);
    for (label, entries) in [
        ("document.json", vec![RawZipEntry::stored(b"document.json", &bomb)]),
        ("manifest.json", vec![RawZipEntry::stored(b"manifest.json", &bomb), RawZipEntry::stored(b"sheets/a.csv", b"A\n1\n")]),
        ("info/info.json", vec![RawZipEntry::stored(b"info/info.json", &bomb)]),
    ] {
        assert!(matches!(import_prism_bytes(&build_raw_zip(&entries, b"")), Err(FormatError::Json(_))), "{label}");
    }
}

#[test]
fn audit_29_pzfx_and_prism_validate_and_sanitize_invalid_sheet_ids() {
    let pzfx_xml = pzfx_doc(&format!(
        r#"<InfoSequence><Info ID="bad info:1"><Title>Info</Title></Info></InfoSequence><TableSequence>
  <Table ID="tbl 1:dose {{{{x}}}}" TableType="Column"><Title>T1</Title><YColumn><Subcolumn><d>1</d></Subcolumn></YColumn></Table>
  <Table ID="   " TableType="Column"><Title>T2</Title><YColumn><Subcolumn><d>2</d></Subcolumn></YColumn></Table>
  <Table ID="{}" TableType="Column"><Title>T3</Title><YColumn><Subcolumn><d>3</d></Subcolumn></YColumn></Table>
</TableSequence>"#,
        "s".repeat(270)
    ));
    let pzfx_rep = import_pzfx_str(&pzfx_xml).unwrap();
    assert!(pzfx_rep.document.info_sheets.iter().all(|s| SheetId::try_new(s.id.as_str()).is_ok()));
    assert!(
        pzfx_rep
            .document
            .data_sheets
            .iter()
            .all(|ds| SheetId::try_new(ds.id.as_str()).is_ok() && ds.info_sheet_id.as_ref() == Some(&pzfx_rep.document.info_sheets[0].id))
    );
    assert!(pzfx_rep.warnings.iter().filter(|w| w.kind == ImportWarningKind::CoercedValue).count() >= 4);

    let archive_rep = import_prism_bytes(&build_zip_entries(&[
        ("manifest.json", br#"{"title":"Archive Invalid IDs","info_sheets":[{"id":"info : bad","title":"Info"}],"sheets":[{"id":"data {{1}}","kind":"Column","csv_path":"sheets/a.csv"}]}"#),
        ("sheets/a.csv", b"A\n1\n"),
    ]))
    .unwrap();
    assert!(
        SheetId::try_new(archive_rep.document.info_sheets[0].id.as_str()).is_ok()
            && SheetId::try_new(archive_rep.document.data_sheets[0].id.as_str()).is_ok()
    );
    assert_eq!(archive_rep.document.data_sheets[0].info_sheet_id.as_ref(), Some(&archive_rep.document.info_sheets[0].id));
    assert!(archive_rep.warnings.iter().filter(|w| w.kind == ImportWarningKind::CoercedValue).count() >= 2);

    let (bad_info, bad_up, bad_data, bad_res, bad_graph, bad_layout) = (
        SheetId::new("info bad:id"),
        SheetId::new("up stream:0"),
        SheetId::new("data {{1}}"),
        SheetId::new("res 1\tbad"),
        SheetId::new("graph:bad"),
        SheetId::new("layout}}bad"),
    );
    let mut info = InfoSheet::new_preseeded(bad_info.as_str().to_owned(), "Info").unwrap();
    info.linked_data_sheets.push(bad_data.clone());
    let mut up_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).unwrap();
    up_col.data_sets[0].cells[0][0] = Cell::number(9.0);
    let up_ds = DataSheet::new(bad_up.clone(), "Upstream", TableArchetype::Column(up_col)).unwrap();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap();
    col.data_sets[0].cells = vec![vec![Cell::number(1.0)], vec![Cell::number(2.0)]];
    let mut ds = DataSheet::new(bad_data.clone(), "Data", TableArchetype::Column(col)).unwrap();
    ds.info_sheet_id = Some(bad_info);
    let zero_range = SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0));
    ds.linked_regions.push(LinkedDataRegion {
        id: String::from("link-1"),
        spec: ImportLinkSpec {
            source: LinkSource::InternalSheet { sheet_id: bad_up, kind: InternalLinkKind::DataSheet, tab_id: None, range: zero_range },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: zero_range,
        provenance_summary: String::from("upstream link"),
    });
    let res = result_sheet_with_kind(bad_res.as_str(), bad_data.as_str(), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()));
    let gs = GraphSheet::new_uninitialized_for_data(bad_graph.clone(), &ds).unwrap();
    let mut ls_spec = LayoutSpec::default();
    ls_spec.items.push(live_graph_item("panel-1", bad_graph.as_str()));
    let ls = LayoutSheet::new(bad_layout, "Layout", ls_spec).unwrap();

    let mut doc = ProjectDocument::new("Embedded Bad IDs").unwrap();
    doc.info_sheets.push(info);
    doc.data_sheets.extend([up_ds, ds]);
    doc.result_sheets.push(res);
    doc.graph_sheets.push(gs);
    doc.layout_sheets.push(ls);
    doc.active_sheet_id = Some(bad_graph);
    doc.family_anchor_sheet_id = Some(bad_data);

    let loaded = import_prism_bytes(&build_zip_entries(&[("document.json", &serde_json::to_vec(&doc).unwrap())])).unwrap();
    let clean = &loaded.document;
    validate_project_document(clean).unwrap();
    assert_eq!(loaded.warnings.iter().filter(|w| w.kind == ImportWarningKind::CoercedValue).count(), 6);
    for id in [
        clean.info_sheets[0].id.as_str(),
        clean.data_sheets[0].id.as_str(),
        clean.data_sheets[1].id.as_str(),
        clean.result_sheets[0].id.as_str(),
        clean.graph_sheets[0].id.as_str(),
        clean.layout_sheets[0].id.as_str(),
    ] {
        assert!(SheetId::try_new(id).is_ok(), "{id:?}");
    }
    let (clean_up_id, clean_ds) = (&clean.data_sheets[0].id, &clean.data_sheets[1]);
    assert_eq!(clean.info_sheets[0].linked_data_sheets, vec![clean_ds.id.clone()]);
    assert_eq!(clean_ds.info_sheet_id.as_ref(), Some(&clean.info_sheets[0].id));
    assert_eq!(clean_ds.linked_regions[0].spec.source.upstream_sheet_id(), Some(clean_up_id));
    assert_eq!((&clean.result_sheets[0].analysis.id, &clean.result_sheets[0].analysis.source_sheet_id), (&clean.result_sheets[0].id, &clean_ds.id));
    assert_eq!(
        (&clean.graph_sheets[0].spec.primary_source_sheet_id, clean.active_sheet_id.as_ref(), clean.family_anchor_sheet_id.as_ref()),
        (&clean_ds.id, Some(&clean.graph_sheets[0].id), Some(&clean_ds.id))
    );
}

#[test]
fn audit_72_csv_formula_injection_neutralized_while_numbers_untouched() {
    let rows = [
        ("=Row0", "'=Row0", "=SUM(A1)", "'=SUM(A1)"),
        ("+Row1", "'+Row1", "+1+1", "'+1+1"),
        ("-Row2", "'-Row2", "-cmd", "'-cmd"),
        ("@Row3", "'@Row3", "@A1", "'@A1"),
        ("\tRow4", "'\tRow4", "\tcmd", "'\tcmd"),
        ("\rRow5", "'\rRow5", "\rcmd", "'\rcmd"),
        ("SafeRow6", "SafeRow6", "-1.5", "'-1.5"),
        ("SafeRow7", "SafeRow7", "ok", "ok"),
    ];
    let mut tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, rows.len()).unwrap();
    tbl.row_titles_expanded = true;
    tbl.row_titles = rows.iter().map(|(r, ..)| String::from(*r)).collect();
    tbl.data_sets[0].title = String::from("=ColTitle");
    tbl.data_sets[1].title = String::from("NumericCol");
    for (i, &(_, _, txt, _)) in rows.iter().enumerate() {
        tbl.data_sets[0].cells[i][0] = Cell::text(txt);
    }
    tbl.data_sets[1].cells[0][0] = Cell::number(-1.5);
    tbl.data_sets[1].cells[1][0] = Cell::excluded_number(-2.5);

    let opts = CsvOptions::csv();
    let exported = export_datasheet_to_csv(&DataSheet::new("formula-ds", "Formula Sheet", TableArchetype::Column(tbl)).unwrap(), &opts).unwrap();
    let parsed = parse_delimited_records(&exported, &opts).unwrap();
    assert_eq!(parsed[0], vec!["Row Title", "'=ColTitle", "NumericCol"]);
    for (i, &(_, exp_r, _, exp_t)) in rows.iter().enumerate() {
        assert_eq!((parsed[i + 1][0].as_str(), parsed[i + 1][1].as_str()), (exp_r, exp_t), "row {i}");
    }
    assert_eq!((parsed[1][2].as_str(), parsed[2][2].as_str()), ("-1.5", "-2.5*"));

    let re = import_csv_to_datasheet(
        "formula-ds-re",
        "Formula Sheet",
        &exported,
        TableKind::Column,
        SubcolumnMode::Single,
        &CsvOptions { has_row_titles: true, ..opts.clone() },
    )
    .unwrap();
    let TableArchetype::Column(ref re_col) = re.archetype else { panic!("expected Column") };
    assert_eq!((&re_col.data_sets[1].cells[0][0], &re_col.data_sets[1].cells[1][0]), (&Cell::number(-1.5), &Cell::excluded_number(-2.5)));
    assert_eq!((&re_col.data_sets[0].cells[4][0], &re_col.data_sets[0].cells[5][0]), (&Cell::text("'\tcmd"), &Cell::text("'\rcmd")));

    let mut pow = PartsOfWholeTable::new(1, 2).unwrap();
    pow.slice_labels = vec![String::from("=SliceA"), String::from("-SliceB")];
    pow.data_sets[0].cells = vec![vec![Cell::number(40.0)], vec![Cell::number(60.0)]];
    let pow_rows = parse_delimited_records(
        &export_datasheet_to_csv(&DataSheet::new("pow", "PoW", TableArchetype::PartsOfWhole(pow)).unwrap(), &opts).unwrap(),
        &opts,
    )
    .unwrap();
    assert_eq!((pow_rows[1][0].as_str(), pow_rows[2][0].as_str()), ("'=SliceA", "'-SliceB"));
}

#[test]
fn audit_72_result_sheet_csv_formula_injection_neutralized_while_numbers_untouched() {
    let opts = CsvOptions::csv();
    let mut res = result_sheet_with_kind("=res-id", "data-1", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()));
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
        points: vec![CurveSamplePoint { x: -2.0, y: -3.0, ci_lower: Some(-4.5), ci_upper: Some(1.5), pred_lower: None, pred_upper: None }],
    });
    res.checklist.push(ChecklistItem {
        assumption: String::from("-Assumption"),
        status: ChecklistStatus::Pass,
        diagnostic_metric: Some(String::from("=metric")),
        explanation: String::from("\texplanation"),
        remediation: None,
    });

    let tab_rows = parse_delimited_records(&export_result_tab_csv(&res.tabs[0], &opts), &opts).unwrap();
    assert_eq!(tab_rows, vec![vec!["Row", "'=H1", "'+H2", "'-H3"], vec!["'@R1", "-1.5", "'=HYPERLINK(\"http://evil\")", "'+0.001 (-star)"]]);

    let tidy_rows = parse_delimited_records(&export_result_sheet_tidy_csv(&res, &opts), &opts).unwrap();
    assert!(tidy_rows.iter().any(|r| r[2] == "tab_cell" && r[0] == "'=res-id" && r[1] == "'+BadTitle" && r[8] == "-1.5" && r[9] == "-1.5"));
    assert!(tidy_rows.iter().any(|r| r[2] == "scalar" && r[6] == "'@Key" && r[8] == "-3.25" && r[11] == "'=uM"));
    assert!(tidy_rows.iter().any(|r| r[2] == "pairwise_comparison" && r[6] == "'=A vs B" && r[9] == "-4.5"));
    assert!(tidy_rows.iter().any(|r| r[2] == "curve_point" && r[3] == "'@eq" && r[6] == "-2" && r[9] == "-3" && r[14] == "-4.5"));
    assert!(tidy_rows.iter().any(|r| r[2] == "checklist" && r[6] == "'-Assumption" && r[8] == "'\texplanation" && r[18] == "'=metric"));
}

#[test]
fn audit_73_quote_aware_csv_round_trip_and_colon_subcolumn_headers() {
    let mut xy = XyTable::new_with_x_mode(XColumnMode::default(), SubcolumnMode::MeanSdN, 1, 2).unwrap();
    xy.data_sets[0].title = String::from("Drug: high");
    xy.x_cells = vec![vec![Cell::number(1.0)], vec![Cell::number(2.0)]];
    xy.data_sets[0].cells =
        vec![vec![Cell::number(10.0), Cell::number(1.0), Cell::number(3.0)], vec![Cell::number(20.0), Cell::number(2.0), Cell::number(3.0)]];

    let opts = CsvOptions::csv();
    let xy_csv = export_datasheet_to_csv(&DataSheet::new("xy-colon", "Colon Test", TableArchetype::Xy(xy)).unwrap(), &opts).unwrap();
    assert!(xy_csv.lines().next().unwrap().contains("Drug: high:Mean,Drug: high:SD,Drug: high:N"));
    let TableArchetype::Xy(ref re_xy) =
        import_csv_to_datasheet("xy-colon-re", "Colon Test", &xy_csv, TableKind::Xy, SubcolumnMode::MeanSdN, &opts).unwrap().archetype
    else {
        panic!("expected XY")
    };
    assert_eq!(re_xy.data_sets[0].title, "Drug: high");

    let cases = [
        (Cell::text("NA"), "\"NA\""),
        (Cell::text("."), "\".\""),
        (Cell::text("NULL"), "\"NULL\""),
        (Cell::text("NaN"), "\"NaN\""),
        (Cell::text("abc*"), "\"abc*\""),
        (Cell::text("*abc"), "\"*abc\""),
        (Cell::text("*"), "\"*\""),
        (Cell::text("007"), "\"007\""),
        (Cell::text("1e3"), "\"1e3\""),
        (Cell::text("  lead_and_trail  "), "\"  lead_and_trail  \""),
        (Cell::text(""), "\"\""),
        (Cell::empty(), ""),
        (Cell { value: CellValue::Empty, excluded: true }, "*"),
        (Cell::number(-1.5), "-1.5"),
        (Cell::excluded_number(3.5), "3.5*"),
    ];
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, cases.len()).unwrap();
    col.data_sets[0].title = String::from("  Padded Title  ");
    col.data_sets[0].cells = cases.iter().map(|(c, _)| vec![c.clone()]).collect();
    let csv_out = export_datasheet_to_csv(&DataSheet::new("quote-rt", "Quote Round-Trip", TableArchetype::Column(col)).unwrap(), &opts).unwrap();
    let lines: Vec<&str> = csv_out.lines().collect();
    assert_eq!(lines[0], "\"  Padded Title  \"");
    for (r, (_, expected_line)) in cases.iter().enumerate() {
        assert_eq!(lines[r + 1], *expected_line, "row {r}");
    }

    let rt = import_csv_to_datasheet("quote-rt-re", "Quote Round-Trip", &csv_out, TableKind::Column, SubcolumnMode::Single, &opts).unwrap();
    let TableArchetype::Column(ref rt_col) = rt.archetype else { panic!("expected Column") };
    assert_eq!(rt_col.data_sets[0].title, "  Padded Title  ");
    assert_eq!(rt_col.data_sets[0].cells.iter().map(|r| r[0].clone()).collect::<Vec<_>>(), cases.iter().map(|(c, _)| c.clone()).collect::<Vec<_>>());

    let euro_opts = CsvOptions::european_csv();
    let mut euro = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap();
    euro.data_sets[0].title = String::from("Euro");
    euro.data_sets[0].cells = vec![vec![Cell::number(1.5)], vec![Cell::text("1,5")]];
    let euro_csv = export_datasheet_to_csv(&DataSheet::new("euro", "Euro", TableArchetype::Column(euro)).unwrap(), &euro_opts).unwrap();
    assert_eq!(euro_csv, "Euro\n1,5\n\"1,5\"");
    let TableArchetype::Column(ref re_euro) =
        import_csv_to_datasheet("euro-re", "Euro", &euro_csv, TableKind::Column, SubcolumnMode::Single, &euro_opts).unwrap().archetype
    else {
        panic!("expected Column")
    };
    assert_eq!(re_euro.data_sets[0].cells, vec![vec![Cell::number(1.5)], vec![Cell::text("1,5")]]);

    let mut doc = ProjectDocument::new("RT").unwrap();
    doc.add_data_sheet(rt).unwrap();
    assert!(!save_project_bytes(&doc).unwrap().is_empty());
}
