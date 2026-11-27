use crate::common::{build_zip_entries, col_sheet, make_result_sheet, pzfx_doc, pzfx_tables, result_sheet_with_kind, rnum, xlsx_single_sheet_zip};
use curvebook_doc::{
    AnalysisKind, Cell, CellCoord, CellValue, ChecklistItem, ChecklistStatus, CiBound, CurveSamplePoint, FittedCurveOverlay, OneWayAnovaSpec,
    PairwiseComparisonRecord, ResultCell, ResultScalarEntry, ResultStatus, ResultTab, ResultTabKind, SheetId, SourceCellId, SubcolumnMode,
    TableArchetype, TableKind, VariableRole,
};
use curvebook_format::xml::decode_xml_entities;
use curvebook_format::{
    CsvOptions, FormatError, ImportWarningKind, XmlLimits, ZipLimits, export_result_sheet_json, export_result_sheet_tidy_csv, export_result_tab_csv,
    import_prism_bytes, import_prism_bytes_with_limits, import_pzfx_str, import_xlsx_bytes, parse_delimited_records, parse_xml,
    parse_xml_with_limits,
};
use serde_json::json;

#[test]
fn xml_parser_handles_bom_comments_cdata_entities_and_security_bounds() {
    let xml = "\u{FEFF}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- lead comment -->\n<Root attr=\"a &amp; b\">\n  <Title>Conc (10<sup>-6</sup> M)</Title>\n  <Raw><![CDATA[<tag> & &#65;]]></Raw>\n  <Glyph>&#x3B1; &lt; 0.05 &apos;ok&apos; &quot;1&quot;</Glyph>\n</Root>";
    let root = parse_xml(xml).unwrap();
    assert_eq!(root.attr("attr"), Some("a & b"));
    assert_eq!(root.first_child("Title").unwrap().plain_text(), "Conc (10-6 M)");
    assert_eq!(root.first_child("Raw").unwrap().plain_text(), "<tag> & &#65;");
    assert_eq!(root.first_child("Glyph").unwrap().plain_text(), "α < 0.05 'ok' \"1\"");

    assert!(matches!(parse_xml("<!DOCTYPE foo [<!ENTITY bar \"baz\">]><Root/>"), Err(FormatError::XmlParse { .. })));
    let limits = XmlLimits { max_bytes: 4096, max_depth: 3, max_nodes: 100 };
    assert!(matches!(parse_xml_with_limits("<A><B><C><D/></C></B></A>", limits), Err(FormatError::XmlDepthExceeded { depth: 4, max: 3 })));

    // NUL and forbidden XML 1.0 C0/nonchar entities rejected everywhere; whitespace refs accepted.
    for bad in ["&#0;", "&#x0;", "&#x00;", "&#1;", "&#8;", "&#11;", "&#x0C;", "&#31;", "&#xFFFE;", "&#xFFFF;"] {
        assert!(matches!(decode_xml_entities(bad, 0), Err(FormatError::XmlParse { .. })), "{bad}");
        assert!(matches!(parse_xml(&format!("<Root>{bad}</Root>")), Err(FormatError::XmlParse { .. })), "{bad}");
    }
    let bad_pzfx =
        pzfx_tables("<Table TableType=\"OneWay\"><Title>T</Title><YColumn><Title>A</Title><Subcolumn><d>Dose&#0;1</d></Subcolumn></YColumn></Table>");
    assert!(matches!(import_pzfx_str(&bad_pzfx), Err(FormatError::XmlParse { .. })));
    let bad_xlsx = xlsx_single_sheet_zip("Nul", r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Sample&#x0;A</t></is></c></row>"#);
    assert!(matches!(import_xlsx_bytes(&bad_xlsx), Err(FormatError::XmlParse { .. })));
    assert_eq!(parse_xml("<Root>a&#9;b&#10;c&#13;d&#32;e</Root>").unwrap().plain_text(), "a\tb\nc\rd e");
}

#[test]
fn pzfx_importer_covers_all_eight_table_types_exclusions_and_info_constants() {
    let pzfx = pzfx_doc(
        r#"<Title>Multi-Table Assay</Title>
  <InfoSequence><Info ID="info-main"><Title>Experiment Metadata</Title>
    <Constant><Name>HotNM</Name><Value>2.5</Value><Units>nM</Units></Constant>
    <Constant><Name>Buffer</Name><Value>HEPES pH 7.4</Value></Constant>
    <Notes>All assays performed in triplicate.</Notes>
  </Info></InfoSequence>
  <TableSequence>
    <Table ID="t-xy" TableType="XY" XFormat="numbers" YFormat="replicates" Replicates="2">
      <Title>Dose Response (10<sup>-6</sup> M)</Title>
      <RowTitlesColumn><Subcolumn><d>Low</d><d>High</d></Subcolumn></RowTitlesColumn>
      <XColumn><Title>Conc</Title><Subcolumn><d>1.0</d><d>10.0</d></Subcolumn></XColumn>
      <YColumn><Title>Wildtype</Title><Subcolumn><d>12.5</d><d>80.0</d></Subcolumn><Subcolumn><d Excluded="1">999.0</d><d>82.0</d></Subcolumn></YColumn>
    </Table>
    <Table ID="t-col" TableType="OneWay"><Title>Treatment Groups</Title>
      <YColumn><Title>Control</Title><Subcolumn><d>5.1</d><d>5.3</d></Subcolumn></YColumn>
      <YColumn><Title>Drug</Title><Subcolumn><d>9.4</d><d Excluded="true">99.0</d></Subcolumn></YColumn>
    </Table>
    <Table ID="t-grp" TableType="TwoWay" YFormat="SD"><Title>Factorial Assay</Title>
      <RowTitlesColumn><Subcolumn><d>24h</d></Subcolumn></RowTitlesColumn>
      <YColumn><Title>Vehicle</Title><Subcolumn><d>10</d></Subcolumn><Subcolumn><d>1.2</d></Subcolumn><Subcolumn><d>6</d></Subcolumn></YColumn>
    </Table>
    <Table ID="t-cont" TableType="Contingency"><Title>Responder Counts</Title>
      <RowTitlesColumn><Subcolumn><d>Exposed</d><d>Unexposed</d></Subcolumn></RowTitlesColumn>
      <YColumn><Title>Positive</Title><Subcolumn><d>14</d><d>3</d></Subcolumn></YColumn>
      <YColumn><Title>Negative</Title><Subcolumn><d>6</d><d>17</d></Subcolumn></YColumn>
    </Table>
    <Table ID="t-surv" TableType="Survival"><Title>Kaplan-Meier</Title>
      <XColumn><Title>Days</Title><Subcolumn><d>5</d><d>12</d></Subcolumn></XColumn>
      <YColumn><Title>Cohort A</Title><Subcolumn><d>1</d><d>0</d></Subcolumn></YColumn>
    </Table>
    <Table ID="t-pow" TableType="PartsOfWhole"><Title>Cell Fractions</Title>
      <RowTitlesColumn><Subcolumn><d>CD4+</d><d>CD8+</d></Subcolumn></RowTitlesColumn>
      <YColumn><Title>Count</Title><Subcolumn><d>45</d><d>55</d></Subcolumn></YColumn>
    </Table>
    <Table ID="t-mv" TableType="MultipleVariables"><Title>Patient Cohort</Title>
      <YColumn><Title>Cohort</Title><Subcolumn><d>A</d><d>B</d></Subcolumn></YColumn>
      <YColumn><Title>Age</Title><Subcolumn><d>42</d><d>58</d></Subcolumn></YColumn>
    </Table>
    <Table ID="t-nest" TableType="Nested" Replicates="2"><Title>Animal Replicates</Title>
      <YColumn><Title>WT</Title><Subcolumn><d>1.1</d><d>1.2</d></Subcolumn><Subcolumn><d>1.3</d><d>1.4</d></Subcolumn></YColumn>
    </Table>
    <Table ID="t-unk" TableType="CustomFuture"><Title>Fallback</Title><YColumn><Title>A</Title><Subcolumn><d>1</d></Subcolumn></YColumn></Table>
  </TableSequence>
  <AnalysisSequence><Analysis ID="an-1"/></AnalysisSequence>"#,
    );
    let report = import_pzfx_str(&pzfx).unwrap();
    let doc = &report.document;
    assert_eq!((doc.title.as_str(), doc.info_sheets.len(), doc.data_sheets.len()), ("Multi-Table Assay", 1, 9));
    assert_eq!(doc.info_sheets[0].get_constant("HotNM").unwrap().value.as_finite_f64(), Some(2.5));
    assert_eq!(doc.data_sheets[0].title, "Dose Response (10<sup>-6</sup> M)");

    let TableArchetype::Xy(xy) = &doc.data_sheets[0].archetype else { panic!("expected XY") };
    let excl = xy.data_sets[0].cell(0, 1).unwrap();
    assert_eq!(
        (xy.y_subcolumn_mode, excl.active_number(), excl.excluded, excl.value.finite_f64()),
        (SubcolumnMode::RawReplicates(2), None, true, Some(999.0))
    );

    let expected_kinds = [
        TableKind::Xy,
        TableKind::Column,
        TableKind::Grouped,
        TableKind::Contingency,
        TableKind::Survival,
        TableKind::PartsOfWhole,
        TableKind::MultipleVariables,
        TableKind::Nested,
    ];
    for (idx, expected) in expected_kinds.iter().enumerate() {
        assert_eq!(doc.data_sheets[idx].kind(), *expected, "sheet {idx}");
    }
    let TableArchetype::MultipleVariables(mv) = &doc.data_sheets[6].archetype else { panic!("expected MV") };
    assert_eq!((mv.variables[0].role, mv.variables[1].role), (VariableRole::Categorical, VariableRole::Continuous));
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedTableType));
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::IgnoredSection));
}

#[test]
fn prism_zip_importer_reads_csv_json_and_warns_on_unknown_binary_chunks() {
    let manifest = serde_json::to_vec(&json!({
        "title": "External Container Assay",
        "sheets": [{ "id": "sheet-xy", "title": "Kinetic Trace", "kind": "XY", "subcolumn_mode": "Single", "csv_path": "data/kinetics.csv" }]
    }))
    .unwrap();
    let info = serde_json::to_vec(
        &json!({ "id": "info-1", "title": "Notes", "constants": { "Km": 12.5, "Operator": "A. Researcher" }, "notes": "Imported cleanly" }),
    )
    .unwrap();
    let zip_bytes = build_zip_entries(&[
        ("manifest.json", &manifest),
        ("data/kinetics.csv", b"Time,Signal\n0,1.0\n5,4.2*\n10,9.5\n"),
        ("info/info.json", &info),
        ("chunks/raw.arrow", &[0xDE, 0xAD, 0xBE, 0xEF]),
        ("scripts/plot.py", b"print('hi')"),
    ]);

    let report = import_prism_bytes(&zip_bytes).unwrap();
    assert_eq!(
        (report.document.title.as_str(), report.document.info_sheets.len(), report.document.data_sheets.len()),
        ("External Container Assay", 1, 1)
    );
    assert_eq!(report.document.info_sheets[0].get_constant("Km").unwrap().value.as_finite_f64(), Some(12.5));
    let TableArchetype::Xy(xy) = &report.document.data_sheets[0].archetype else { panic!("expected XY") };
    assert!(xy.data_sets[0].cell(1, 0).unwrap().excluded);
    assert_eq!(report.warnings.len(), 2);
    assert!(report.warnings.iter().all(|w| w.kind == ImportWarningKind::UnsupportedChunk));

    let tight = ZipLimits { max_entries: 2, max_total_uncompressed_bytes: 1024 * 1024, max_entry_uncompressed_bytes: 1024 * 1024 };
    assert!(matches!(import_prism_bytes_with_limits(&zip_bytes, tight), Err(FormatError::ZipBomb { .. })));
}

#[test]
fn result_sheet_tidy_csv_and_structured_json_export() {
    let mut res = result_sheet_with_kind("anova-1", "data-1", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()));
    res.status = ResultStatus::UpToDate;
    res.scalars.push(ResultScalarEntry { key: String::from("R2"), data_set_index: None, value: 0.942, units: None });
    res.tabs.push(ResultTab {
        id: String::from("summary"),
        title: String::from("ANOVA Summary"),
        kind: ResultTabKind::AnovaTable,
        column_headers: vec![String::from("F"), String::from("P value"), String::from("95% CI"), String::from("Significant")],
        row_labels: vec![String::from("Treatment")],
        rows: vec![vec![
            rnum(18.4),
            ResultCell::PValue { raw: 0.000_032, formatted: String::from("< 0.0001"), stars: String::from("****") },
            ResultCell::ConfidenceInterval { lower: CiBound::Finite(2.1), upper: CiBound::Open },
            ResultCell::Boolean(true),
        ]],
        footnotes: vec![],
    });
    res.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: 0,
        group_b: 1,
        label_a: String::from("Control"),
        label_b: String::from("Treated"),
        estimate_diff: Some(4.5),
        raw_p_or_q: 0.0021,
        is_fdr_q: false,
        significant: true,
    });
    res.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("four_pl"),
        converged: true,
        ambiguous: false,
        points: vec![CurveSamplePoint { x: -7.0, y: 50.0, ci_lower: Some(45.0), ci_upper: Some(55.0), pred_lower: None, pred_upper: None }],
    });
    res.checklist.push(ChecklistItem {
        assumption: String::from("Normality of residuals"),
        status: ChecklistStatus::Pass,
        diagnostic_metric: Some(String::from("W = 0.98")),
        explanation: String::from("Residuals consistent with Gaussian distribution"),
        remediation: None,
    });

    let opts = CsvOptions::csv();
    let tab_rows = parse_delimited_records(&export_result_tab_csv(&res.tabs[0], &opts), &opts).unwrap();
    assert_eq!(
        (tab_rows.len(), tab_rows[1][0].as_str(), tab_rows[1][2].as_str(), tab_rows[1][3].as_str()),
        (2, "Treatment", "< 0.0001 (****)", "[2.1, Open]")
    );

    let tidy_rows = parse_delimited_records(&export_result_sheet_tidy_csv(&res, &opts), &opts).unwrap();
    assert_eq!(tidy_rows.len(), 9);
    assert!(tidy_rows.iter().any(|r| r[2] == "tab_cell" && r[12] == "0.000032"));
    assert!(tidy_rows.iter().any(|r| r[2] == "scalar" && r[6] == "R2" && r[9] == "0.942"));
    assert!(tidy_rows.iter().any(|r| r[2] == "pairwise_comparison" && r[16] == "true"));
    assert!(tidy_rows.iter().any(|r| r[2] == "curve_point" && r[14] == "45"));
    assert!(tidy_rows.iter().any(|r| r[2] == "checklist" && r[17] == "Pass"));

    let parsed: serde_json::Value = serde_json::from_str(&export_result_sheet_json(&res, true).unwrap()).unwrap();
    assert_eq!((&parsed["id"], &parsed["scalars"][0]["key"]), (&json!("anova-1"), &json!("R2")));
}

#[test]
fn prism_zip_importer_uses_neutral_default_title_when_manifest_title_missing() {
    let zip = build_zip_entries(&[("manifest.json", br#"{"title":"   "}"#), ("data/values.csv", b"Group,Response\nA,1.5\nB,2.5\n")]);
    assert_eq!(import_prism_bytes(&zip).unwrap().document.title, "Imported Project");
}

#[test]
fn result_exports_preserve_outliers_pred_bands_footnotes_and_analyzable_data() {
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
    assert_eq!((tidy[0].len(), tidy[0][19].as_str(), tidy[0][20].as_str(), tidy[0][21].as_str()), (22, "pred_lower", "pred_upper", "provenance"));
    assert!(tidy.iter().all(|r| r.len() == 22));
    assert!(tidy.iter().any(|r| r[2] == "curve_point" && r[14] == "45" && r[15] == "55" && r[19] == "35" && r[20] == "65" && r[21] == "computed"));
    assert!(tidy.iter().any(|r| r[2] == "tab_footnote" && r[3] == "summary" && r[8] == "ROUT Q = 1%"));
    assert!(tidy.iter().any(|r| r[2] == "tab_footnote" && r[8] == "'=formula_note"));
    assert!(tidy.iter().any(|r| r[2] == "eliminated_outlier" && r[3] == "data-1" && r[6] == "row_2" && r[7] == "col_0_sub_1"));

    let jv: serde_json::Value = serde_json::from_str(&export_result_sheet_json(&rs, false).unwrap()).unwrap();
    assert_eq!(
        (
            &jv["eliminated_outliers"][0]["sheet_id"],
            &jv["eliminated_outliers"][0]["row"],
            &jv["eliminated_outliers"][0]["subcol"],
            &jv["analyzable_data"]["id"]
        ),
        (&json!("data-1"), &json!(2), &json!(1), &json!("fit-rout-cleaned"))
    );
    if let Some(ref mut ad) = rs.analyzable_data
        && let TableArchetype::Column(ref mut ct) = ad.archetype
    {
        ct.data_sets[0].cells[0][0] = Cell { value: CellValue::Number(f64::NAN), excluded: false };
    }
    assert!(matches!(export_result_sheet_json(&rs, false), Err(FormatError::NonFiniteFloat { .. })));
}
